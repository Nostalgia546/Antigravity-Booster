use std::fs;
use std::path::PathBuf;
use std::process::Command;
use sysinfo::System;
use tauri::Manager;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Get Antigravity installation directory
fn get_antigravity_dir() -> Option<PathBuf> {
    // Try common installation paths
    let paths = vec![
        dirs::data_local_dir().map(|d| d.join("Programs/Antigravity")),
        dirs::home_dir().map(|d| d.join("AppData/Local/Programs/Antigravity")),
    ];
    
    for path_opt in paths {
        if let Some(path) = path_opt {
            if path.join("Antigravity.exe").exists() {
                return Some(path);
            }
        }
    }
    
    // Try to find from running process
    let mut system = System::new_all();
    system.refresh_all();
    
    for (_, process) in system.processes() {
        let name = process.name().to_lowercase();
        if name.contains("antigravity") && !name.contains("booster") {
            if let Some(exe_path) = process.exe() {
                if let Some(parent) = exe_path.parent() {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }
    
    None
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
            if name.contains("antigravity") && !name.contains("booster") {
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
    let antigravity_dir = get_antigravity_dir()
        .ok_or("无法找到 Antigravity 安装目录。请确保 Antigravity 正在运行。".to_string())?;
    
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
    
    // 3. Copy version.dll to Antigravity directory
    let target_path = antigravity_dir.join("version.dll");
    println!("Target DLL path: {:?}", target_path);
    
    fs::copy(&resource_path, &target_path)
        .map_err(|e| format!("复制 version.dll 失败: {}。请确保有管理员权限。", e))?;
    
    println!("DLL copied successfully");
    
    // 4. Kill and restart Antigravity
    kill_antigravity()?;
    println!("Antigravity killed");
    
    // Wait for process to exit (max 5 seconds)
    if !wait_for_antigravity_exit(5000) {
        println!("Warning: Antigravity may still be running");
    }
    
    restart_antigravity(&antigravity_dir)?;
    println!("Antigravity restarted");
    
    Ok("已启用系统代理，Antigravity 已重启".to_string())
}

/// Disable system proxy for Antigravity
pub fn disable_system_proxy(_app: &tauri::AppHandle) -> Result<String, String> {
    // 1. Find Antigravity directory
    let antigravity_dir = get_antigravity_dir()
        .ok_or("无法找到 Antigravity 安装目录。请确保 Antigravity 正在运行。".to_string())?;
    
    println!("Found Antigravity directory: {:?}", antigravity_dir);
    
    // 2. Kill Antigravity FIRST (before trying to delete DLL)
    kill_antigravity()?;
    println!("Antigravity killed");
    
    // 3. Wait for process to exit (max 5 seconds)
    if !wait_for_antigravity_exit(5000) {
        println!("Warning: Antigravity may still be running, attempting to delete DLL anyway");
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
    
    // 5. Restart Antigravity
    restart_antigravity(&antigravity_dir)?;
    println!("Antigravity restarted");
    
    Ok("已禁用系统代理，Antigravity 已重启".to_string())
}

/// Check if system proxy is enabled
pub fn is_proxy_enabled(_app: &tauri::AppHandle) -> Result<bool, String> {
    let antigravity_dir = get_antigravity_dir()
        .ok_or("无法找到 Antigravity 安装目录")?;
    
    let dll_path = antigravity_dir.join("version.dll");
    Ok(dll_path.exists())
}
