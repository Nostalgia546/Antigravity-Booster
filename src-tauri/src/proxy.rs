use std::path::PathBuf;
use std::process::Command;
use sysinfo::System;
use tauri::Manager;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Get Antigravity installation directory with caching
/// 优先使用配置中保存的路径，找到后自动保存
pub fn get_antigravity_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    // 1. 优先尝试从配置文件读取已保存的路径
    let config = crate::storage::load_config(app);
    if let Some(saved_path) = &config.antigravity_executable {
        let path = PathBuf::from(saved_path);
        // 验证路径是否仍然有效
        if path.join("Antigravity.exe").exists() {
            println!("Using cached Antigravity path: {:?}", path);
            return Some(path);
        } else {
            println!("Cached path invalid, will search again");
        }
    }
    
    // 2. 尝试常见安装路径
    let paths = vec![
        dirs::data_local_dir().map(|d| d.join("Programs/Antigravity")),
        dirs::home_dir().map(|d| d.join("AppData/Local/Programs/Antigravity")),
    ];
    
    for path_opt in paths {
        if let Some(path) = path_opt {
            if path.join("Antigravity.exe").exists() {
                println!("Found Antigravity at common path: {:?}", path);
                // 保存到配置
                save_antigravity_path(app, &path);
                return Some(path);
            }
        }
    }
    
    // 3. 尝试从正在运行的进程中查找
    let mut system = System::new_all();
    system.refresh_all();
    
    for (_, process) in system.processes() {
        let name = process.name().to_lowercase();
        // 精确匹配 Antigravity.exe
        if name == "antigravity.exe" {
            if let Some(exe_path) = process.exe() {
                if let Some(parent) = exe_path.parent() {
                    let path = parent.to_path_buf();
                    println!("Found Antigravity from running process: {:?}", path);
                    // 保存到配置
                    save_antigravity_path(app, &path);
                    return Some(path);
                }
            }
        }
    }
    
    None
}

/// 保存 Antigravity 路径到配置文件
fn save_antigravity_path(app: &tauri::AppHandle, path: &PathBuf) {
    let mut config = crate::storage::load_config(app);
    let path_str = path.to_string_lossy().to_string();
    
    // 只有当路径发生变化时才保存
    if config.antigravity_executable.as_ref() != Some(&path_str) {
        config.antigravity_executable = Some(path_str);
        if let Err(e) = crate::storage::save_config(app, &config) {
            eprintln!("Failed to save Antigravity path to config: {}", e);
        } else {
            println!("Saved Antigravity path to config");
        }
    }
}

/// 检测 Antigravity 是否正在运行
fn is_antigravity_running() -> bool {
    let mut system = System::new_all();
    system.refresh_all();
    
    for (_, process) in system.processes() {
        let name = process.name().to_lowercase();
        // 精确匹配 Antigravity.exe，避免匹配到 Booster 或其他相关进程
        if name == "antigravity.exe" {
            return true;
        }
    }
    false
}

/// Kill Antigravity process
fn kill_antigravity() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = Command::new("taskkill")
            .args(&["/IM", "Antigravity.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let _ = Command::new("taskkill")
            .args(&["/F", "/IM", "Antigravity.exe"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    Ok(())
}

/// Wait for Antigravity process to exit (with timeout)
fn wait_for_antigravity_exit(timeout_ms: u64) -> bool {
    use std::time::{Duration, Instant};
    
    let start = Instant::now();
    let timeout = Duration::from_millis(timeout_ms);
    
    while start.elapsed() < timeout {
        let mut system = System::new_all();
        system.refresh_all();
        
        let mut found = false;
        for (_, process) in system.processes() {
            let name = process.name().to_lowercase();
            // 精确匹配 Antigravity.exe
            if name == "antigravity.exe" {
                found = true;
                break;
            }
        }
        
        if !found {
            println!("Antigravity process exited");
            return true;
        }
        
        std::thread::sleep(Duration::from_millis(100));
    }
    
    println!("Timeout waiting for Antigravity to exit");
    false
}

/// Restart Antigravity
fn restart_antigravity(antigravity_dir: &PathBuf) -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(1000));
    let exe_path = antigravity_dir.join("Antigravity.exe");
    if exe_path.exists() {
        let _ = open::that(&exe_path);
        Ok(())
    } else {
        Err("Antigravity.exe not found".to_string())
    }
}

// 定义当前 DLL 的预期版本号（每次更新 C++ 代码后应修改此值）
// 定义当前 DLL 的预期版本号（每次更新 C++ 代码后应修改此值）
const EXPECTED_DLL_VERSION: &str = "2026.01.28.02";

/// 更新 Antigravity 目录下的 proxy_config.json
fn update_proxy_json(antigravity_dir: &std::path::Path, config: &crate::storage::BoosterConfig, enabled: bool) -> Result<(), String> {
    let json_path = antigravity_dir.join("proxy_config.json");
    let json_content = serde_json::json!({
        "version": EXPECTED_DLL_VERSION, // 写入版本号
        "enabled": enabled,
        "host": config.proxy_host,
        "port": config.proxy_port,
        "type": config.proxy_type,
        "ipv6_mode": "block"
    });
    
    std::fs::write(&json_path, serde_json::to_string_pretty(&json_content).unwrap())
        .map_err(|e| format!("写入配置文件失败: {}", e))
}

/// 简单比对两个文件内容是否一致
fn files_are_equal(a: &std::path::Path, b: &std::path::Path) -> bool {
    let Ok(f1) = std::fs::read(a) else { return false };
    let Ok(f2) = std::fs::read(b) else { return false };
    f1 == f2
}

/// Enable system proxy for Antigravity
pub fn enable_system_proxy(app: &tauri::AppHandle) -> Result<String, String> {
    // 1. Find Antigravity directory
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录。".to_string())?;
    
    let config = crate::storage::load_config(app);
    let dll_path = antigravity_dir.join("version.dll");
    let was_running = is_antigravity_running();

    // 2. 这里的关键：找到资源里的最新 DLL
    let possible_paths = vec![
        app.path().resource_dir().ok().map(|p| p.join("version.dll")),
        app.path().resource_dir().ok().map(|p| p.join("resources/version.dll")),
        Some(std::path::PathBuf::from("src-tauri/resources/version.dll")),
        Some(std::path::PathBuf::from("resources/version.dll")),
    ];
    
    let mut resource_path = None;
    for path_opt in possible_paths {
        if let Some(path) = path_opt {
            if path.exists() {
                resource_path = Some(path);
                break;
            }
        }
    }
    let resource_path = resource_path.ok_or("找不到 version.dll 资源文件。")?;

    // 3. 校验 DLL 是否需要更新 (不存在，或者内容不一致)
    let needs_dll_update = !dll_path.exists() || !files_are_equal(&dll_path, &resource_path);

    if needs_dll_update {
        // 如果正在运行且需要更新核心 DLL，必须重启
        if was_running {
            kill_antigravity()?;
            wait_for_antigravity_exit(5000);
        }
        
        std::fs::copy(&resource_path, &dll_path)
            .map_err(|e| format!("更新 version.dll 核心失败: {}", e))?;
    }

    // 4. 更新 JSON 配置文件 (无论是否更新了 DLL)
    update_proxy_json(&antigravity_dir, &config, true)?;

    // 5. 判断反馈信息
    if was_running && needs_dll_update {
        restart_antigravity(&antigravity_dir)?;
        Ok("核心组件已升级，Antigravity 已重启以启用最新功能".to_string())
    } else if was_running {
        Ok("已实时开启代理，无需重启".to_string())
    } else {
        Ok("代理配置已就绪，下次启动将自动生效".to_string())
    }
}

/// 启动时的兼容性检查：确保 DLL 是最新的，且配置一致
/// 这是为了解决老版本用户升级上来的兼容性问题
pub fn ensure_dll_compatibility(app: &tauri::AppHandle) {
    let Some(antigravity_dir) = get_antigravity_dir(app) else { return };
    let dll_path = antigravity_dir.join("version.dll");
    let json_path = antigravity_dir.join("proxy_config.json");

    // 0. 快速检查：如果版本号匹配，直接跳过 (解决重复重启问题)
    if dll_path.exists() && json_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&json_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(v) = json.get("version").and_then(|s| s.as_str()) {
                    if v == EXPECTED_DLL_VERSION {
                         // 版本一致，无需任何操作！完美！
                         return;
                    }
                }
            }
        }
    }

    if !dll_path.exists() { return; }

    // 1. 查找资源里的最新 DLL
    let possible_paths = vec![
        app.path().resource_dir().ok().map(|p| p.join("resources/version.dll")),
        app.path().resource_dir().ok().map(|p| p.join("version.dll")),
        Some(std::path::PathBuf::from("src-tauri/resources/version.dll")),
        Some(std::path::PathBuf::from("resources/version.dll")),
    ];
    
    let mut resource_path = None;
    for path_opt in possible_paths {
        if let Some(path) = path_opt {
            if path.exists() {
                resource_path = Some(path);
                break;
            }
        }
    }
    let Some(resource_path) = resource_path else { return };

    // 2. 如果内容不一致，说明是旧版 DLL
    if !files_are_equal(&dll_path, &resource_path) {
        println!("Detected old version.dll, performing robust upgrade...");
        let was_running = is_antigravity_running();
        
        if was_running {
            // 强力关闭并等待更久一点
            let _ = kill_antigravity();
            wait_for_antigravity_exit(5000); 
        }
        
        // 3. 循环重试复制（解决 Windows 文件锁定带来的延时）
        let mut success = false;
        for i in 0..5 {
            if std::fs::copy(&resource_path, &dll_path).is_ok() {
                success = true;
                break;
            }
            println!("DLL copy attempt {} failed, retrying...", i + 1);
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }

        if success {
            let config = crate::storage::load_config(app);
            let _ = update_proxy_json(&antigravity_dir, &config, true);
        }
        
        // 4. 关键：只要刚才在运行，现在就必须重启
        if was_running {
            let _ = restart_antigravity(&antigravity_dir);
        }
    }
}

/// Disable system proxy for Antigravity
pub fn disable_system_proxy(app: &tauri::AppHandle) -> Result<String, String> {
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录。".to_string())?;
    
    let config = crate::storage::load_config(app);
    let was_running = is_antigravity_running();

    // 只更新配置文件，不删除 DLL，不重启
    update_proxy_json(&antigravity_dir, &config, false)?;
    
    if was_running {
        Ok("已实时禁用代理，无需重启".to_string())
    } else {
        Ok("已禁用代理配置".to_string())
    }
}

/// Check if system proxy is enabled
pub fn is_proxy_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录")?;
    
    let dll_path = antigravity_dir.join("version.dll");
    let json_path = antigravity_dir.join("proxy_config.json");

    // 1. 如果 DLL 不存在，那肯定没开
    if !dll_path.exists() {
        return Ok(false);
    }

    // 2. 如果 DLL 存在，优先看 JSON 配置
    if let Ok(content) = std::fs::read_to_string(&json_path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(enabled) = json.get("enabled").and_then(|v| v.as_bool()) {
                return Ok(enabled);
            }
        }
    }

    // 3. 如果 JSON 也没说清楚，默认 DLL 存在就是开启 (兼容旧行为)
    Ok(true)
}
