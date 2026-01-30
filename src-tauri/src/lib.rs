mod storage;
mod oauth;
mod history;
mod proxy;
mod quota;

use std::fs;
use std::process::Command;
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, Manager, menu::{Menu, MenuItem}, tray::{TrayIconBuilder, TrayIconEvent}};
use crate::storage::{Account, load_accounts, save_accounts, load_config, TokenData, BoosterConfig, save_config, get_app_dir};
use crate::oauth::export_backup;
use crate::proxy::get_antigravity_dir;
use base64::{Engine as _, engine::general_purpose};
use chrono::Timelike;

#[tauri::command]
fn sync_vault_entries(app: AppHandle) -> Vec<Account> {
    let mut accounts = load_accounts(&app);
    
    // Sanitize/Map data strictly for display
    for acc in &mut accounts {
        if let Some(q) = &mut acc.quota {
            // 1. Identify valid items (both new sanitized names AND old raw key names)
            // If it's an old raw key, we rename it in-memory for display immediately.
            let mut valid_models = Vec::new();

            for m in &q.models {
                let name = m.name.as_str();
                let display_name = match name {
                    "Gemini Pro" | "gemini-3-pro-high" => Some("Gemini Pro"),
                    "Gemini Flash" | "gemini-3-flash" => Some("Gemini Flash"),
                    "Claude" | "claude-sonnet-4-5" => Some("Claude"),
                    // Also support the previous display names if any lingering
                    "Claude 3.5 Sonnet" => Some("Claude"),
                    _ => None
                };

                if let Some(new_name) = display_name {
                    let mut new_m = m.clone();
                    new_m.name = new_name.to_string();
                    valid_models.push(new_m);
                }
            }

            // Replace with filtered list
            q.models = valid_models;

            // Re-sort
            q.models.sort_by(|a, b| {
                let score = |name: &str| {
                    if name == "Gemini Pro" { 1 }
                    else if name == "Gemini Flash" { 2 }
                    else { 3 }
                };
                score(&a.name).cmp(&score(&b.name))
            });
        }
    }
    
    accounts
}

#[tauri::command]
fn save_account(app: AppHandle, account: Account) -> Result<(), String> {
    let mut accounts = load_accounts(&app);
    let mut new_account = account;
    if accounts.is_empty() {
        new_account.is_active = true;
    }
    accounts.push(new_account);
    save_accounts(&app, &accounts).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_account(app: AppHandle, id: String) -> Result<(), String> {
    let mut accounts = load_accounts(&app);
    accounts.retain(|a| a.id != id);
    save_accounts(&app, &accounts).map_err(|e| e.to_string())
}

#[tauri::command]
async fn switch_account(app: AppHandle, id: String) -> Result<String, String> {
    use sysinfo::System;

    // --- Phase 1: Load & Validate ---
    let mut accounts = load_accounts(&app);
    
    // [重要] 切换前快照：记录当前活跃账号的最终状态
    if let Some(current_active) = accounts.iter().find(|a| a.is_active) {
        let current_id = current_active.id.clone();
        println!("[Switch] Recording pre-switch snapshot for: {}", current_active.name);
        // 尝试刷新一下当前账号，确保记录的是最新的余量
        let _ = pulse_check_quota(app.clone(), current_id, true).await;
    }

    let mut matching_account_index = None;
    let mut found_token_data: Option<TokenData> = None;

    // Find the account and check data integrity
    for (idx, acc) in accounts.iter().enumerate() {
        if acc.id == id {
            matching_account_index = Some(idx);
            if let Some(td) = &acc.token_data {
                found_token_data = Some(td.clone());
            }
            break;
        }
    }

    // Fail early if not found
    if matching_account_index.is_none() {
        return Err("Account not found.".into());
    }

    // --- Phase 2: Update State on Disk ---
    for acc in &mut accounts {
        acc.is_active = acc.id == id;
    }
    save_accounts(&app, &accounts).map_err(|e| e.to_string())?;

    // Prepare variables for injection
    let (final_access, final_refresh, final_expiry) = if let Some(td) = found_token_data {
        // Full OAuth data available
        // 修复: 如果 refresh_token 为空(从 Antigravity 导入的账号),使用 access_token 作为 fallback
        let refresh = if td.refresh_token.is_empty() {
            td.access_token.clone()
        } else {
            td.refresh_token
        };
        (td.access_token, refresh, td.expires_at)
    } else {
        // Fallback: Use legacy token field (likely an access token from auto-import)
        let acc = &accounts[matching_account_index.unwrap()];
        let access = acc.token.clone();
        // We don't have a refresh token, so we use the access token as both
        // This will work for the current session but won't be refreshable
        let expiry = chrono::Utc::now().timestamp() + 3600; // Assume 1 hour validity
        (access.clone(), access, expiry)
    };

    // --- 智能刷新机制: 如果 token 是 refresh token,先刷新获取新的 access token ---
    let (final_access, final_refresh, final_expiry) = if final_access.starts_with("1//") {
        // final_access 是 refresh token,需要先刷新
        println!("[Switch] Detected refresh token as access token, refreshing...");
        match oauth::refresh_access_token(&final_access, None).await {
            Ok(new_access) => {
                println!("[Switch] Successfully refreshed access token");
                // 使用新的 access token,保持 refresh token 不变
                (new_access, final_refresh, chrono::Utc::now().timestamp() + 3600)
            },
            Err(e) => {
                println!("[Switch] Failed to refresh token: {}", e);
                return Err(format!("Token 已过期且刷新失败: {}。请在 Antigravity Booster 中重新使用 OAuth 方式添加账号。", e));
            }
        }
    } else if final_refresh.starts_with("1//") && final_refresh != final_access {
        // final_refresh 是真正的 refresh token,final_access 可能过期了,尝试刷新
        println!("[Switch] Access token may be expired, refreshing with refresh token...");
        match oauth::refresh_access_token(&final_refresh, None).await {
            Ok(new_access) => {
                println!("[Switch] Successfully refreshed access token from refresh token");
                (new_access, final_refresh, chrono::Utc::now().timestamp() + 3600)
            },
            Err(e) => {
                // 刷新失败,但还是尝试用原来的 access token (可能还没过期)
                println!("[Switch] Refresh failed, will try with existing access token: {}", e);
                (final_access, final_refresh, final_expiry)
            }
        }
    } else {
        // access token 看起来是有效的,直接使用
        (final_access, final_refresh, final_expiry)
    };


    // --- Phase 3: Process Management (Kill Antigravity) ---
    // 从配置获取 Antigravity 路径（如果之前运行过会被缓存）
    let config = load_config(&app);
    let antigravity_dir = config.antigravity_executable
        .as_ref()
        .map(|p| std::path::PathBuf::from(p));
    
    // 检测 Antigravity 是否正在运行
    let was_running = {
        let mut system = System::new_all();
        system.refresh_all();
        let mut running = false;
        
        for (_, process) in system.processes() {
            let name = process.name().to_lowercase();
            // 精确匹配 Antigravity.exe，避免匹配到 Booster 或其他相关进程
            if name == "antigravity.exe" {
                running = true;
                println!("Detected Antigravity.exe is running");
                break;
            }
        }
        
        if !running {
            println!("Antigravity.exe is not running");
        }
        
        running
    };

    // 只在 Antigravity 运行时才关闭
    if was_running {
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            // 尝试优雅关闭
            let _ = Command::new("taskkill").args(&["/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
            std::thread::sleep(std::time::Duration::from_millis(500)); // 缩短等待时间
            // 确保进程已退出
            let _ = Command::new("taskkill").args(&["/F", "/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
        }
        println!("Antigravity was running, triggered fast restart");
    } else {
        println!("Antigravity is not running, skipping kill step");
    }
    // --- Phase 4: Database Injection ---
    let mut target_dbs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        target_dbs.push(home.join("AppData/Roaming/Antigravity/User/globalStorage/state.vscdb"));
    }
    // Portable support - 使用缓存的路径
    if let Some(ref dir) = antigravity_dir {
        target_dbs.push(dir.join("data/user-data/User/globalStorage/state.vscdb"));
    }

    // Inject
    let mut success_count = 0;
    
    // --- 构造通用 V2 数据包 (纯净实现) ---
    fn to_varint(mut v: u64) -> Vec<u8> {
        let mut b = Vec::new();
        while v >= 0x80 { b.push((v & 0x7F | 0x80) as u8); v >>= 7; }
        b.push(v as u8);
        b
    }
    let mut inner = Vec::new();
    // AccessToken (Field 1)
    inner.extend(to_varint((1 << 3) | 2));
    inner.extend(to_varint(final_access.len() as u64));
    inner.extend(final_access.as_bytes());
    // RefreshToken (Field 3)
    inner.extend(to_varint((3 << 3) | 2));
    inner.extend(to_varint(final_refresh.len() as u64));
    inner.extend(final_refresh.as_bytes());
    // Expiry (Field 4 -> 1)
    let mut ts = Vec::new();
    ts.extend(to_varint((1 << 3) | 0));
    ts.extend(to_varint(final_expiry as u64));
    inner.extend(to_varint((4 << 3) | 2));
    inner.extend(to_varint(ts.len() as u64));
    inner.extend(ts);
    
    let mut payload = Vec::new();
    payload.extend(to_varint((6 << 3) | 2));
    payload.extend(to_varint(inner.len() as u64));
    payload.extend(inner);
    let v2_b64 = general_purpose::STANDARD.encode(&payload);

    for db in target_dbs {
        if !db.exists() { continue; }
        if let Ok(conn) = rusqlite::Connection::open(&db) {
            let _ = conn.execute("PRAGMA busy_timeout = 3000;", []);
            
            // 1. 注入到深度状态键位 (遵循 Manager/Agent 结构协议)
            let _ = conn.execute("INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('jetskiStateSync.agentManagerInitState', ?1)", [&v2_b64]);
            // 2. 注入到全局用户键位 (确保基础编辑器身份识别)
            let _ = conn.execute("INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('current_user', ?1)", [&final_access]);
            // 3. 注入引导状态标志 (确保跳过登录后的引导弹窗)
            let _ = conn.execute("INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('antigravityOnboarding', 'true')", []);

            success_count += 1;
        }
    }

    // --- Phase 5: Restart (only if it was running before) ---
    if was_running {
        #[cfg(target_os = "windows")]
        {
            std::thread::sleep(std::time::Duration::from_millis(100)); // 只有 0.1 秒的间隙
            // 使用缓存的路径或常见路径
            let restart_path = antigravity_dir
                .as_ref()
                .map(|d| d.join("Antigravity.exe"))
                .or_else(|| {
                    dirs::data_local_dir().map(|d| d.join("Programs/Antigravity/Antigravity.exe"))
                });

            if let Some(path) = restart_path {
                if path.exists() {
                    let _ = open::that(&path);
                    println!("Antigravity restarted");
                }
            }
        }
    } else {
        println!("Antigravity is not running, skipping restart");
    }

    Ok(format!("Switched ({} DBs synced)", success_count))
}

#[tauri::command]
fn record_history_snapshot(app: AppHandle) -> Result<(), String> {
    crate::history::record_quota_point(&app)
}

#[tauri::command]
fn load_booster_settings(app: AppHandle) -> BoosterConfig {
    load_config(&app)
}

#[tauri::command]
fn update_booster_settings(app: AppHandle, config: BoosterConfig) -> Result<(), String> {
    save_config(&app, &config).map_err(|e| e.to_string())
}

#[tauri::command]
async fn pulse_check_quota(app: AppHandle, id: String, record_history: bool) -> Result<storage::QuotaData, String> {
    // 1. Load accounts
    let mut accounts = load_accounts(&app);
    let proxy_url = None;

    // 2. Find index
    let acc_idx = accounts.iter().position(|a| a.id == id)
        .ok_or_else(|| "Account not found".to_string())?;

    // 3. Prepare data
    let (access_token, email) = {
        let acc = &accounts[acc_idx];
        let token_to_use = if acc.token.starts_with("1//") {
            match oauth::refresh_access_token(&acc.token, proxy_url.clone()).await {
                Ok(at) => at,
                Err(e) => return Err(format!("Quota error: Token refresh failed: {}", e)),
            }
        } else {
            acc.token.clone()
        };
        (token_to_use, acc.email.clone())
    };

    // 5. Fetch Quota
    let (new_quota, detected_tier, debug_logs) = quota::fetch_account_quota_real(&access_token, &email, proxy_url).await?;
    let _ = app.emit("debug-log", debug_logs);

    // 6. Mutate Accounts
    {
        let acc = &mut accounts[acc_idx];
        acc.quota = Some(new_quota.clone());
        if !detected_tier.is_empty() {
             acc.account_type = detected_tier;
        }
    }

    // 7. Save to Disk
    save_accounts(&app, &accounts).map_err(|e| e.to_string())?;

    // 8. Bridge Write (Only if active)
    if accounts[acc_idx].is_active {
        write_quota_bridge_file(&app, &new_quota);
    }

    if record_history {
        let _ = crate::history::record_quota_point(&app);
    }
    
    // [重要] 通知前端数据已刷新，立即响应 UI 变化
    let _ = app.emit("quota-updated", ());
    
    Ok(new_quota)
}

// 辅助函数：将中文小时数转换为 "x天x小时"
fn prettify_duration(raw: &str) -> String {
    let re = regex::Regex::new(r"(\d+)\s*小时").unwrap();
    if let Some(caps) = re.captures(raw) {
        if let Ok(hours) = caps[1].parse::<i32>() {
            if hours >= 24 {
                let days = hours / 24;
                let rem = hours % 24;
                let replace_str = if rem > 0 {
                    format!("{}天{}小时", days, rem)
                } else {
                    format!("{}天", days)
                };
                return re.replace(raw, replace_str).to_string();
            }
        }
    }
    raw.to_string()
}

// 写入桥接文件供插件读取
fn write_quota_bridge_file(app: &AppHandle, quota: &crate::storage::QuotaData) {
    let bridge_dir = get_app_dir(app); 
    let _ = std::fs::create_dir_all(&bridge_dir);
    let path = bridge_dir.join("quota_bridge.json");
    
    // 1. Generate Short Text for Status Bar (e.g. "Pro: 90% | Flash: 100%")
    let mut short_parts = Vec::new();
    // 2. Generate Tooltip (Markdown Table)
    let mut tooltip_lines = Vec::new();
    
    tooltip_lines.push("| Model | Usage | Reset |".to_string());
    tooltip_lines.push("|---|---|---|".to_string());

    for m in &quota.models {
        // Short Name mapping
        let s_name = if m.name.to_lowercase().contains("pro") { "Pro" }
        else if m.name.to_lowercase().contains("flash") { "Flash" }
        else if m.name.to_lowercase().contains("claude") { "Claude" }
        else { &m.name };
        
        let percent = m.percentage;
        short_parts.push(format!("{}: {:.0}%", s_name, percent));
        
        // Tooltip Row
        let reset_pretty = prettify_duration(&m.reset_time);
        tooltip_lines.push(format!("| {} | {:.1}% | {} |", m.name, percent, reset_pretty));
    }

    let short_text = if short_parts.is_empty() {
        "No Quota Data".to_string()
    } else {
        short_parts.join("  ") // Use double space for separation
    };

    let tooltip_text = tooltip_lines.join("\n");
    
    let json_content = serde_json::json!({
        "status_text": short_text,
        "tooltip": tooltip_text,
        "models": quota.models, // Pass raw data for TreeView customization
        "update_time": chrono::Local::now().to_rfc3339()
    });
    
    let _ = std::fs::write(path, serde_json::to_string(&json_content).unwrap_or_default());
}

#[tauri::command]
async fn install_assistant_extension(app: AppHandle) -> Result<String, String> {
    println!("[Extension] Starting installation...");
    
    // 1. 确定 VSIX 源文件路径 (我们在 resources 目录里)
    let possible_paths = vec![
        app.path().resource_dir().ok().map(|p| p.join("resources/antigravity-booster-helper.vsix")),
        app.path().resource_dir().ok().map(|p| p.join("antigravity-booster-helper.vsix")),
        Some(std::path::PathBuf::from("src-tauri/resources/antigravity-booster-helper.vsix")),
        Some(std::path::PathBuf::from("resources/antigravity-booster-helper.vsix")),
        Some(std::path::PathBuf::from("antigravity-booster-helper.vsix")),
    ];

    let mut vsix_path = None;
    for path_opt in possible_paths {
        if let Some(path) = path_opt {
            if path.exists() {
                vsix_path = Some(path);
                break;
            }
        }
    }

    let vsix_path = vsix_path.ok_or_else(|| "找不到插件安装包 (vsix)。请确保已执行打包流程。".to_string())?;
    println!("[Extension] Found VSIX at: {:?}", vsix_path);

    // 2. 找到 Antigravity CLI
    let ag_dir = get_antigravity_dir(&app).ok_or("未找到 Antigravity 安装目录")?;
    println!("[Extension] Antigravity dir: {:?}", ag_dir);
    
    // 注意：Antigravity 的 CLI 通常在 bin 目录下
    let cli_path = ag_dir.join("bin").join("antigravity.cmd"); 
    
    let final_cli = if cli_path.exists() {
        println!("[Extension] Using CLI: {:?}", cli_path);
        cli_path 
    } else {
        // 尝试 resources/app/out/cli.js (需要 node 运行)? 
        // 或者直接调用主程序带参数
        let exe_path = ag_dir.join("Antigravity.exe");
        println!("[Extension] CLI not found, trying EXE: {:?}", exe_path);
        exe_path 
    };

    // 3. 执行安装命令
    println!("[Extension] Executing command...");
    // Antigravity.exe --install-extension <path>
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        
        let output = std::process::Command::new(&final_cli)
            .arg("--install-extension")
            .arg(&vsix_path) // Borrow path
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("执行安装命令失败: {}", e))?;
            
        println!("[Extension] Command output: {:?}", output);
        
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            // Try visible window if hidden failed?
            println!("[Extension] Installation failed (Hidden). Error: {}", err);
            
            return Err(format!("安装失败: {}", err));
        }
    }

    Ok("插件已成功安装，请重启编辑器生效。".to_string())
}

#[tauri::command]
async fn start_boosting(config: BoosterConfig, app: AppHandle) -> Result<(), String> {
    save_config(&app, &config).map_err(|e| e.to_string())?;
    let mut path = get_app_dir(&app);
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    path.push("booster_runtime_config.json");
    let content = serde_json::to_string_pretty(&config).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
async fn stop_boosting() -> Result<(), String> {
    Ok(())
}

// --- Extension Manager ---

const LATEST_HELPER_VERSION: &str = "1.3.5";

#[derive(serde::Serialize, serde::Deserialize)]
struct ExtensionStatus {
    installed_version: Option<String>,
    latest_version: String,
    status: String, // "not_installed", "outdated", "installed"
}

#[tauri::command]
fn get_extension_status(_app: AppHandle) -> ExtensionStatus {
    let home = dirs::home_dir();
    let mut installed_ver = None;
    
    // 尝试寻找插件安装目录
    // 默认 Antigravity 这里的命名可能是 .antigravity
    if let Some(h) = home {
        // 常见路径探测
        let check_paths = vec![
            h.join(".antigravity/extensions"),
            h.join(".vscode/extensions"), // 兼容
        ];
        
        for base_dir in check_paths {
            if !base_dir.exists() { continue; }
            
            // 查找名为 nostalgia546.antigravity-booster-helper-* 的文件夹
            // 因为 VS Code 插件目录通常包含版本号，例如: publisher.name-version
            if let Ok(entries) = std::fs::read_dir(&base_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                            if fname.starts_with("nostalgia546.antigravity-booster-helper") {
                                // 找到目录了，读取 package.json
                                let pkg_path = path.join("package.json");
                                if let Ok(content) = std::fs::read_to_string(&pkg_path) {
                                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                                        if let Some(v) = json.get("version").and_then(|s| s.as_str()) {
                                            installed_ver = Some(v.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if installed_ver.is_some() { break; }
        }
    }

    let status = match &installed_ver {
        None => "not_installed",
        Some(v) => if v == LATEST_HELPER_VERSION { "installed" } else { "outdated" }
    };

    ExtensionStatus {
        installed_version: installed_ver,
        latest_version: LATEST_HELPER_VERSION.to_string(),
        status: status.to_string(),
    }
}

#[tauri::command]
async fn restart_antigravity(app: AppHandle) -> Result<(), String> {
    use sysinfo::System;
    use std::process::Command;
    use std::os::windows::process::CommandExt;
    
    // 1. Kill
    let mut system = System::new_all();
    system.refresh_all();
    let mut running = false;
    for (_, process) in system.processes() {
        if process.name().to_lowercase() == "antigravity.exe" {
            running = true; 
            break;
        }
    }
    
    if running {
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let _ = Command::new("taskkill").args(&["/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = Command::new("taskkill").args(&["/F", "/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
        }
    }
    
    // 2. Restart
    let config = load_config(&app);
    let antigravity_dir = config.antigravity_executable.map(|p| std::path::PathBuf::from(p));
    
    #[cfg(target_os = "windows")]
    {
         let restart_path = antigravity_dir
            .as_ref()
            .map(|d| d.join("Antigravity.exe"))
            .or_else(|| {
                // Fallback attempt
                 get_antigravity_dir(&app).map(|d| d.join("Antigravity.exe"))
            });

        if let Some(path) = restart_path {
            if path.exists() {
                // open crate handles detached process well
                let _ = open::that(&path);
                return Ok(());
            }
        }
    }
    
    Err("无法找到 Antigravity 主程序，请手动启动".to_string())
}


#[tauri::command]
fn analyze_network_gate() -> String {
    // 1. Check Env Vars first
    let env_proxy = std::env::var("ALL_PROXY")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("HTTP_PROXY"));
        
    if let Ok(p) = env_proxy {
        return p;
    }

    // 2. Check Windows Registry via Command (Simple heuristic)
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let output_enable = Command::new("reg")
            .args(&["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings", "/v", "ProxyEnable"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
            
        if let Ok(o) = output_enable {
            let out_str = String::from_utf8_lossy(&o.stdout);
            // Check for 0x1 (Enabled)
            if out_str.contains("0x1") {
                 let output_server = Command::new("reg")
                    .args(&["query", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings", "/v", "ProxyServer"])
                    .creation_flags(CREATE_NO_WINDOW)
                    .output();
                if let Ok(os) = output_server {
                    let s = String::from_utf8_lossy(&os.stdout);
                    if let Some(idx) = s.find("REG_SZ") {
                        let val = s[idx + 6..].trim();
                        return val.to_string();
                    }
                }
            }
        }
    }

    "Direct / Auto".to_string()
}

fn log_event(app: &AppHandle, msg: &str) {
    println!("{}", msg);
    let _ = app.emit("debug-log", msg);
}

#[tauri::command]
async fn reconcile_active_session(app: AppHandle) -> Result<String, String> {
    use rusqlite::Connection;

    // 1. 获取本地配置中的代理（如果有），用于身份验证请求
    let config = load_config(&app);
    let proxy_url = if config.proxy_enabled {
        Some(format!("{}://{}:{}", config.proxy_type, config.proxy_host, config.proxy_port))
    } else {
        None
    };

    // 2. 准备探测路径 (与 switch_account 镜像)
    let mut target_dbs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        target_dbs.push(home.join("AppData/Roaming/Antigravity/User/globalStorage/state.vscdb"));
    }
    if let Some(dir) = get_antigravity_dir(&app) {
        target_dbs.push(dir.join("data/user-data/User/globalStorage/state.vscdb"));
    }

    let mut found_token = String::new();

    // 3. 逐个数据库探测，优先寻找“鲜活”的 Token
    for db_path in target_dbs {
        if !db_path.exists() { continue; }
        
        log_event(&app, &format!("[Sync] Checking database: {:?}", db_path));
        
        if let Ok(conn) = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            // 策略 A: 扫描所有已知的身份令牌键位 (覆盖多种存储协议分支)
            let keys = "'current_user', 'jetskiStateSync.agentManagerInitState', 'cursor.auth.accessToken', 'gap.auth.accessToken'";
            let query = format!("SELECT key, value FROM ItemTable WHERE key IN ({})", keys);
            
            if let Ok(mut stmt) = conn.prepare(&query) {
                let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).ok();
                if let Some(row_iter) = rows {
                    for r in row_iter.flatten() {
                        let (key, val) = r;
                        let mut token = String::new();
                        
                        // 在检测当前登录时,优先提取 access token (ya29.),因为需要用它来验证身份
                        if val.starts_with("ya29.") {
                            token = val;
                        } else if let Ok(decoded_vec) = general_purpose::STANDARD.decode(&val) {
                            let text = String::from_utf8_lossy(&decoded_vec);
                            if let Some(start) = text.find("ya29.") {
                                let substr = &text[start..];
                                let end = substr.find(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '-').unwrap_or(substr.len());
                                token = substr[0..end].to_string();
                            }
                        }

                        if !token.is_empty() {
                            token = token.trim().trim_matches('"').to_string();
                            // 如果这个 Token 能通过验证，或者本地能对上，我们就锁定它
                            log_event(&app, &format!("[Sync] Candidate found in key '{}' from {:?}", key, db_path));
                            found_token = token;
                            // 注意：这里我们不 break，我们要继续尝试所有可能的 Key，或者所有数据库，直到找到能用的
                        }
                    }
                }
            }
        }
        if !found_token.is_empty() { break; } // 在当前数据库找到了就先试试
    }

    // 3.5 Token 预处理：去掉可能存在的双引号和不可见字符
    let found_token = found_token.trim().trim_matches('"').to_string();

    if found_token.is_empty() {
        log_event(&app, "[Sync] Result: No active identity found in state.vscdb");
    } else {
        log_event(&app, &format!("[Sync] Extracted Token prefix: {}...", &found_token[..10.min(found_token.len())]));
    }

    let mut accounts = load_accounts(&app);
    let mut changed = false;
    let status_msg: String;

    // 4. 解析身份
    let mut matching_idx = None;

    if !found_token.is_empty() {
        // --- 策略 A: 本地 Token 快速查表 (最强力，不依赖网络，不惧过期) ---
        for (i, acc) in accounts.iter().enumerate() {
            let mut is_match = false;
            if acc.token == found_token {
                is_match = true;
            } else if let Some(td) = &acc.token_data {
                if td.access_token == found_token {
                    is_match = true;
                }
            }

            if is_match {
                log_event(&app, &format!("[Sync] Perfect local match found: {}", acc.name));
                matching_idx = Some(i);
                break;
            }
        }

        // --- 策略 B: 只有本地查不到，才请求 Google API ---
        if matching_idx.is_none() {
            log_event(&app, "[Sync] No local match. Requesting Google API identity verification...");
            match oauth::get_user_info(&found_token, proxy_url).await {
                Ok(info) => {
                    let editor_email = info.email.to_lowercase();
                    log_event(&app, &format!("[Sync] Online check confirmed: {}", editor_email));
                    
                    for (i, acc) in accounts.iter().enumerate() {
                        if acc.email.to_lowercase() == editor_email {
                            matching_idx = Some(i);
                            break;
                        }
                    }
                },
                Err(e) => {
                    log_event(&app, &format!("[Sync] Google API verification failed: {}. Token may be fully expired or network issue.", e));
                }
            }
        }
    }

    // 5. 应用同步结果
    if let Some(idx) = matching_idx {
        // 情况 A: 找到了对应的账号
        let name = accounts[idx].name.clone();
        for (i, acc) in accounts.iter_mut().enumerate() {
            let should_be_active = i == idx;
            if acc.is_active != should_be_active {
                acc.is_active = should_be_active;
                changed = true;
            }
            
            // 核心修复：如果识别到了是这个号，不管 is_active 有没有变，都把最新的 Token 同步过来
            // 解决"编辑器自动刷新了 Token，但 Booster 还在用旧 Token 刷新导致数据不准"的问题
            if should_be_active && !found_token.is_empty() {
                // 智能判断: 如果 acc.token 是 refresh token (1// 开头),不要覆盖它
                // 只更新 token_data 中的 access_token
                if !acc.token.starts_with("1//") && acc.token != found_token {
                    // acc.token 是 access token,可以更新
                    acc.token = found_token.clone();
                    changed = true;
                }
                // 无论如何都更新 token_data 中的 access_token
                if let Some(td) = &mut acc.token_data {
                    if td.access_token != found_token {
                        td.access_token = found_token.clone();
                        changed = true;
                    }
                }
            }
        }
        status_msg = format!("同步成功: 已识别为账号 '{}' 并标记激活", name);
    } else if !found_token.is_empty() {
        // 情况 B: 编辑器有登录，但 Booster 里没这个号
        for acc in &mut accounts {
            if acc.is_active { acc.is_active = false; changed = true; }
        }
        status_msg = "检测到未知账号，已取消本地活跃状态".into();
    } else {
        // 情况 C: 编辑器未登录
        for acc in &mut accounts {
            if acc.is_active { acc.is_active = false; changed = true; }
        }
        status_msg = "编辑器未登录任何账号".into();
    }

    if changed {
        let _ = save_accounts(&app, &accounts);
        let _ = app.emit("quota-updated", ());
    }
    
    Ok(status_msg)
}


#[tauri::command]
fn get_usage_chart(app: AppHandle, display_minutes: i64, bucket_minutes: i64) -> history::UsageChartData {
    history::calculate_usage_buckets(&app, display_minutes, bucket_minutes)
}

#[tauri::command]
fn enable_system_proxy(app: AppHandle) -> Result<String, String> {
    proxy::enable_system_proxy(&app)
}

#[tauri::command]
fn disable_system_proxy(app: AppHandle) -> Result<String, String> {
    proxy::disable_system_proxy(&app)
}

#[tauri::command]
fn is_proxy_enabled(app: AppHandle) -> Result<bool, String> {
    proxy::is_proxy_enabled(&app)
}

async fn start_auto_refresh_task(app: AppHandle) {
    use tokio::time::{sleep, Duration};
    
    // 启动时先收割一次插件在离线期间记录的数据
    let _ = history::consume_plugin_buffer(&app);

    // Immediate refresh on startup
    let mut accounts = load_accounts(&app);
    for acc in &mut accounts {
        let access_token = if acc.token.starts_with("1//") {
            match oauth::refresh_access_token(&acc.token, None).await {
                Ok(at) => at,
                Err(_) => continue,
            }
        } else {
            acc.token.clone()
        };
        
        if let Ok((new_quota, detected_tier, _)) = quota::fetch_account_quota_real(&access_token, &acc.email, None).await {
            acc.quota = Some(new_quota.clone());
            if !detected_tier.is_empty() {
                acc.account_type = detected_tier;
            }
            if acc.is_active {
                 write_quota_bridge_file(&app, &new_quota);
            }
        }
    }
    let _ = save_accounts(&app, &accounts);
    let _ = history::record_quota_point(&app);
    let _ = app.emit("quota-updated", ());
    
    loop {
        let now_local = chrono::Local::now();
        let now_ts = now_local.timestamp();
        
        // --- 核心改进：时钟对齐逻辑 ---
        // 计算距离下一个整 5 分钟（即 mm % 5 == 0 且 ss == 0）还有多少秒
        let current_minute = now_local.minute();
        let current_second = now_local.second();
        let seconds_past_5min = (current_minute % 5) * 60 + current_second;
        let sleep_until_next_tick = 300 - seconds_past_5min;
        
        let mut next_run = now_ts + sleep_until_next_tick as i64;
        let original_next_run = next_run; // 备份一下，用于判定是否发生了抢占
        
        // 抢占式记录逻辑保持不变
        {
            let accounts = load_accounts(&app);
            for acc in &accounts {
                if let Some(quota) = &acc.quota {
                    for model in &quota.models {
                        if let Some(reset_ts) = model.reset_at {
                            // 1. 记录快照点：重置前 30 秒
                            let snapshot_target = reset_ts - 30;
                            if snapshot_target > now_ts && snapshot_target < next_run {
                                next_run = snapshot_target;
                            }
                            // 2. 刷新数据点：重置时刻
                            if reset_ts > now_ts && reset_ts < next_run {
                                next_run = reset_ts;
                            }
                        }
                    }
                }
            }
        }
        
        // 判定是否是因为重置相关的抢占触发（快照或到点刷新）
        let is_reset_event = next_run < original_next_run;

        let sleep_secs = (next_run - now_ts).max(1); // 缩短保底时间到 1s，提高整分检测精度
        sleep(Duration::from_secs(sleep_secs as u64)).await;

        // 刷新时间判定
        let now = chrono::Local::now();
        let minute = now.minute();
        
        let is_30_min_tick = minute % 30 == 0;
        let is_5_min_tick = minute % 5 == 0;

        // 如果是重置相关的事件，或者到了 5 分钟点，就执行刷新
        if !is_5_min_tick && !is_reset_event { 
            continue; 
        } 

        // 1. 同步真实身份
        let _ = reconcile_active_session(app.clone()).await;
        
        let mut accounts = load_accounts(&app);
        let config = load_config(&app);
        let proxy_url = if config.proxy_enabled {
            Some(format!("{}://{}:{}", config.proxy_type, config.proxy_host, config.proxy_port))
        } else {
            None
        };
        
        let mut any_fetched = false;
        let current_ts = chrono::Utc::now().timestamp();

        for acc in &mut accounts {
            // 判定该特定账号是否快到重置点了 (30秒抢占)
            let mut acc_near_reset = false;
            if let Some(q) = &acc.quota {
                for m in &q.models {
                    if let Some(r_ts) = m.reset_at {
                        // 窗口扩大，确保在快照点（-30s）和重置点都能涵盖到
                        if r_ts > current_ts && (r_ts - current_ts) < 45 {
                            acc_near_reset = true;
                            break;
                        }
                    }
                }
            }

            // 策略优化：
            // 1. 活跃账号必刷
            // 2. 到了 30 分钟全量点必刷
            // 3. 只有本账号快到重置点了才刷（不再因为别人快重置了就带上全家）
            // 4. 新号没数据的必刷
            let should_fetch = acc.is_active || is_30_min_tick || acc_near_reset || acc.quota.is_none();
            
            if should_fetch {
                let access_token = if acc.token.starts_with("1//") {
                    match oauth::refresh_access_token(&acc.token, proxy_url.clone()).await {
                        Ok(at) => at,
                        Err(_) => continue,
                    }
                } else {
                    acc.token.clone()
                };
                
                if let Ok((new_quota, detected_tier, _)) = quota::fetch_account_quota_real(&access_token, &acc.email, proxy_url.clone()).await {
                    acc.quota = Some(new_quota.clone());
                    if !detected_tier.is_empty() {
                        acc.account_type = detected_tier;
                    }
                    if acc.is_active {
                         write_quota_bridge_file(&app, &new_quota);
                    }
                    any_fetched = true;
                }
            }
        }
        
        if any_fetched {
            let mut disk_accounts = load_accounts(&app);
            for d_acc in &mut disk_accounts {
                if let Some(refreshed) = accounts.iter().find(|a| a.id == d_acc.id) {
                    if refreshed.quota.is_some() {
                        d_acc.quota = refreshed.quota.clone();
                        d_acc.account_type = refreshed.account_type.clone();
                    }
                }
            }
            let _ = save_accounts(&app, &disk_accounts);
            
            // 决定是否记录快照
            // 只有在 30 分钟整点、重置抢占成功、或者这是该账号第一次初始化数据时才记录
            // 决定是否记录快照
            // 策略：如果是 30 分钟整点，或者是我们刻意在重置前 30 秒抢占的那个点，则记录记录历史
            let is_pre_reset_snapshot_point = accounts.iter().any(|a| {
                a.quota.as_ref().map_or(false, |q| {
                    q.models.iter().any(|m| m.reset_at.map_or(false, |ts| (ts - current_ts).abs() < 35 && (ts - current_ts) > 20))
                })
            });

            if is_30_min_tick || is_pre_reset_snapshot_point {
                let _ = history::record_quota_point(&app);
                log_event(&app, "[Task] Pre-reset snapshot recorded.");
            }
            
            let _ = app.emit("quota-updated", ());
            log_event(&app, "[Task] Minute cycle completed.");
        }
    }
}

#[tauri::command]
async fn import_account_from_antigravity(app: AppHandle) -> Result<Account, String> {
    log_event(&app, "[Import] Starting intelligent account extraction...");

    // 1. 探测所有可能的候选 Token
    let candidates = find_all_tokens_in_editor(&app).await;
    if candidates.is_empty() {
        log_event(&app, "[Import] Error: No valid-looking tokens found in Editor.");
        return Err("未能检测到登录信息，请确保编辑器已登录。".into());
    }

    log_event(&app, &format!("[Import] Found {} candidate tokens. Testing them one by one...", candidates.len()));

    // 2. 获取代理配置
    let config = load_config(&app);
    let proxy_url = if config.proxy_enabled {
        Some(format!("{}://{}:{}", config.proxy_type, config.proxy_host, config.proxy_port))
    } else {
        None
    };

    // 3. 逐个尝试验证，优先测试 refresh token
    let mut verified_info = None;
    let mut working_token = String::new();
    let mut working_refresh_token = String::new();

    // 分离 refresh token 和 access token
    let mut refresh_tokens = Vec::new();
    let mut access_tokens = Vec::new();
    for token in candidates {
        if token.starts_with("1//") {
            refresh_tokens.push(token);
        } else {
            access_tokens.push(token);
        }
    }

    // 先测试所有 refresh token (优先使用长期有效的)
    for token in refresh_tokens.iter().chain(access_tokens.iter()) {
        log_event(&app, &format!("[Import] Testing token prefix: {}...", &token[..10.min(token.len())]));
        
        // 判断是 refresh token 还是 access token
        let (access_token_to_test, is_refresh) = if token.starts_with("1//") {
            // 这是 refresh token,需要先换取 access token
            log_event(&app, "[Import] Detected refresh token, exchanging for access token...");
            match oauth::refresh_access_token(&token, proxy_url.clone()).await {
                Ok(at) => {
                    log_event(&app, "[Import] Successfully obtained access token from refresh token");
                    (at, true)
                },
                Err(e) => {
                    log_event(&app, &format!("[Import] Failed to refresh token: {}, trying next...", e));
                    continue;
                }
            }
        } else {
            // 这是 access token,直接使用
            (token.clone(), false)
        };
        
        // 用 access token 验证身份
        match oauth::get_user_info(&access_token_to_test, proxy_url.clone()).await {
            Ok(info) => {
                verified_info = Some(info);
                working_token = access_token_to_test;
                if is_refresh {
                    working_refresh_token = token.clone(); // 保存原始的 refresh token
                }
                break; // 找到了活的，立即退出循环
            },
            Err(e) => {
                log_event(&app, &format!("[Import] Token validation failed: {}, trying next...", e));
            }
        }
    }

    let info = match verified_info {
        Some(i) => i,
        None => {
            log_event(&app, "[Import] Error: All discovered tokens are expired or invalid.");
            return Err("所有探测到的登录信息均已过期，请在编辑器中重新登录。".into());
        }
    };

    log_event(&app, &format!("[Import] Identity confirmed: {} ({})", info.name.as_deref().unwrap_or("Unknown"), info.email));

    let mut accounts = load_accounts(&app);
    let email = info.email.to_lowercase();
    let mut target_acc = None;

    // 4. 全部先设为不活跃
    for acc in &mut accounts {
        acc.is_active = false;
    }

    // 5. 查找或创建
    let mut is_new = true;
    for acc in &mut accounts {
        if acc.email.to_lowercase() == email {
            log_event(&app, "[Import] Account already exists. Updating token and activating...");
            // 如果有 refresh token,优先保存 refresh token;否则保存 access token
            let token_to_save = if !working_refresh_token.is_empty() {
                working_refresh_token.clone()
            } else {
                working_token.clone()
            };
            acc.token = token_to_save.clone();
            acc.token_data = Some(TokenData {
                access_token: working_token.clone(),
                refresh_token: working_refresh_token.clone(),
                expires_at: chrono::Utc::now().timestamp() + 3500,
            });
            acc.is_active = true;
            target_acc = Some(acc.clone());
            is_new = false;
            break;
        }
    }

    if is_new {
        log_event(&app, "[Import] Creating new account record...");
        // 如果有 refresh token,优先保存 refresh token;否则保存 access token
        let token_to_save = if !working_refresh_token.is_empty() {
            working_refresh_token.clone()
        } else {
            working_token.clone()
        };
        let new_acc = Account {
            id: email.clone(),
            name: info.name.clone().unwrap_or_else(|| info.email.clone()),
            email: info.email.clone(),
            token: token_to_save,
            token_data: Some(TokenData {
                access_token: working_token,
                refresh_token: working_refresh_token,
                expires_at: chrono::Utc::now().timestamp() + 3500,
            }),
            account_type: "Gemini".to_string(),
            status: "active".to_string(),
            quota: None,
            is_active: true,
        };
        accounts.push(new_acc.clone());
        target_acc = Some(new_acc);
    }

    // 6. 保存并广播
    save_accounts(&app, &accounts).map_err(|e| e.to_string())?;
    let _ = app.emit("quota-updated", ()); // 强制 UI 刷新
    
    log_event(&app, "[Import] SUCCESS! Account is now ready.");
    
    match target_acc {
        Some(a) => Ok(a),
        None => Err("逻辑异常：未捕获到目标账号".into())
    }
}

// 辅助函数：提取所有候选 Token
async fn find_all_tokens_in_editor(app: &AppHandle) -> Vec<String> {
    use rusqlite::Connection;
    let mut candidates = Vec::new();
    let mut target_dbs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        target_dbs.push(home.join("AppData/Roaming/Antigravity/User/globalStorage/state.vscdb"));
    }
    if let Some(dir) = get_antigravity_dir(app) {
        target_dbs.push(dir.join("data/user-data/User/globalStorage/state.vscdb"));
    }

    for db_path in target_dbs {
        if !db_path.exists() { continue; }
        if let Ok(conn) = Connection::open_with_flags(&db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let keys = "'current_user', 'jetskiStateSync.agentManagerInitState', 'cursor.auth.accessToken', 'gap.auth.accessToken'";
            let query = format!("SELECT value FROM ItemTable WHERE key IN ({})", keys);
            if let Ok(mut stmt) = conn.prepare(&query) {
                let mut rows = stmt.query_map([], |row| row.get::<usize, String>(0)).ok().unwrap();
                while let Some(Ok(val)) = rows.next() {
                    let mut token = String::new();
                    // 优先提取 refresh token (1//),其次提取 access token (ya29.)
                    if val.starts_with("1//") {
                        token = val;
                    } else if val.starts_with("ya29.") {
                        token = val;
                    } else if let Ok(decoded_vec) = general_purpose::STANDARD.decode(&val) {
                        let text = String::from_utf8_lossy(&decoded_vec);
                        // 先找 refresh token
                        if let Some(start) = text.find("1//") {
                            let substr = &text[start..];
                            let end = substr.find(|c: char| !c.is_ascii_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-').unwrap_or(substr.len());
                            token = substr[0..end].to_string();
                        } else if let Some(start) = text.find("ya29.") {
                            // 如果没有 refresh token,再找 access token
                            let substr = &text[start..];
                            let end = substr.find(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '-').unwrap_or(substr.len());
                            token = substr[0..end].to_string();
                        }
                    }
                    if !token.is_empty() {
                        let clean_token = token.trim().trim_matches('"').to_string();
                        if !candidates.contains(&clean_token) {
                            candidates.push(clean_token);
                        }
                    }
                }
            }
        }
    }
    candidates
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let _ = app.get_webview_window("main").map(|w| {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            });
        }))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(|app| {
            let app_handle = app.handle().clone();
            
            // 异步后台执行 DLL 维护，防止白屏
            let maintenance_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                crate::proxy::ensure_dll_compatibility(&maintenance_handle);
            });

            // Create Tray Menu
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let show_i = MenuItem::with_id(app, "show", "显示主界面", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } => {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            tauri::async_runtime::spawn(async move {
                start_auto_refresh_task(app_handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                window.hide().unwrap();
                api.prevent_close();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            sync_vault_entries,
            save_account,
            delete_account,
            import_account_from_antigravity,
            export_backup,
            switch_account,
            reconcile_active_session,
            load_booster_settings,
            update_booster_settings,
            pulse_check_quota,
            record_history_snapshot,
            start_boosting,
            stop_boosting,
            analyze_network_gate,
            oauth::import_backup,
            oauth::export_backup,
            oauth::start_oauth_login,
            get_usage_chart,
            enable_system_proxy,
            disable_system_proxy,
            is_proxy_enabled,
            install_assistant_extension,
            restart_antigravity,
            get_extension_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
