use std::fs;
use std::path::PathBuf;
use std::process::Command;
use sysinfo::System;
use tauri::Manager;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Get Antigravity installation directory with caching
/// 优先使用配置中保存的路径，找到后自动保存
fn get_antigravity_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
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

/// Enable system proxy for Antigravity
pub fn enable_system_proxy(app: &tauri::AppHandle) -> Result<String, String> {
    // 1. Find Antigravity directory
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录。请确保 Antigravity 已安装或至少运行过一次。".to_string())?;
    
    println!("Found Antigravity directory: {:?}", antigravity_dir);
    
    // 2. Get version.dll from Booster resources
    // Try multiple possible paths (dev mode vs release mode)
    let possible_paths = vec![
        app.path().resource_dir().ok().map(|p| p.join("version.dll")),
        Some(PathBuf::from("src-tauri/resources/version.dll")),
        Some(PathBuf::from("src-tauri/target/debug/resources/version.dll")),
        Some(PathBuf::from("resources/version.dll")),
    ];
    
    let mut resource_path = None;
    for path_opt in possible_paths {
        if let Some(path) = path_opt {
            println!("Trying resource path: {:?}", path);
            if path.exists() {
                println!("Found DLL at: {:?}", path);
                resource_path = Some(path);
                break;
            }
        }
    }
    
    let resource_path = resource_path.ok_or_else(|| {
        "version.dll 资源文件不存在。请确保已编译 DLL 并运行 update-dll.bat".to_string()
    })?;
    
    // 3. 检测 Antigravity 是否正在运行
    let was_running = is_antigravity_running();
    
    // 4. 如果正在运行，先关闭
    if was_running {
        kill_antigravity()?;
        println!("Antigravity killed");
        
        // Wait for process to exit (max 5 seconds)
        if !wait_for_antigravity_exit(5000) {
            println!("Warning: Antigravity may still be running");
        }
    } else {
        println!("Antigravity is not running, skipping kill step");
    }
    
    // 5. Copy version.dll to Antigravity directory
    let target_path = antigravity_dir.join("version.dll");
    println!("Target DLL path: {:?}", target_path);
    
    fs::copy(&resource_path, &target_path)
        .map_err(|e| format!("复制 version.dll 失败: {}。请确保有管理员权限。", e))?;
    
    println!("DLL copied successfully");
    
    // 6. 如果之前在运行，重启
    if was_running {
        restart_antigravity(&antigravity_dir)?;
        println!("Antigravity restarted");
        Ok("已启用系统代理，Antigravity 已重启".to_string())
    } else {
        Ok("已启用系统代理。下次启动 Antigravity 时将自动生效。".to_string())
    }
}

/// Disable system proxy for Antigravity
pub fn disable_system_proxy(app: &tauri::AppHandle) -> Result<String, String> {
    // 1. Find Antigravity directory
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录。请确保 Antigravity 已安装或至少运行过一次。".to_string())?;
    
    println!("Found Antigravity directory: {:?}", antigravity_dir);
    
    // 2. 检测 Antigravity 是否正在运行
    let was_running = is_antigravity_running();
    
    // 3. 如果正在运行，先关闭
    if was_running {
        kill_antigravity()?;
        println!("Antigravity killed");
        
        // Wait for process to exit (max 5 seconds)
        if !wait_for_antigravity_exit(5000) {
            println!("Warning: Antigravity may still be running, attempting to delete DLL anyway");
        }
    } else {
        println!("Antigravity is not running, skipping kill step");
    }
    
    // 4. Remove version.dll with retry
    let dll_path = antigravity_dir.join("version.dll");
    println!("DLL path to remove: {:?}", dll_path);
    
    if dll_path.exists() {
        // Try up to 3 times
        let mut attempts = 0;
        let mut last_error = None;
        
        while attempts < 3 {
            match fs::remove_file(&dll_path) {
                Ok(_) => {
                    println!("DLL removed successfully on attempt {}", attempts + 1);
                    break;
                }
                Err(e) => {
                    last_error = Some(e);
                    attempts += 1;
                    if attempts < 3 {
                        println!("Failed to remove DLL (attempt {}), retrying...", attempts);
                        std::thread::sleep(std::time::Duration::from_millis(1000));
                    }
                }
            }
        }
        
        if let Some(e) = last_error {
            if attempts >= 3 {
                return Err(format!("删除 version.dll 失败（已重试 3 次）: {}。DLL 可能仍被占用，请手动删除。", e));
            }
        }
    } else {
        println!("DLL not found, skipping removal");
    }
    
    // 5. 如果之前在运行，重启
    if was_running {
        restart_antigravity(&antigravity_dir)?;
        println!("Antigravity restarted");
        Ok("已禁用系统代理，Antigravity 已重启".to_string())
    } else {
        Ok("已禁用系统代理。下次启动 Antigravity 时将自动生效。".to_string())
    }
}

/// Check if system proxy is enabled
pub fn is_proxy_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    let antigravity_dir = get_antigravity_dir(app)
        .ok_or("无法找到 Antigravity 安装目录")?;
    
    let dll_path = antigravity_dir.join("version.dll");
    Ok(dll_path.exists())
}
