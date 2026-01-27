mod storage;
mod migration;
mod oauth;
mod history;
mod proxy;

use std::fs;
use tauri::{AppHandle, Emitter, Manager, menu::{Menu, MenuItem}, tray::{TrayIconBuilder, TrayIconEvent}};
use crate::storage::{Account, BoosterConfig, load_accounts, save_accounts, get_app_dir, load_config, save_config};

#[tauri::command]
fn get_accounts(app: AppHandle) -> Vec<Account> {
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
fn switch_account(app: AppHandle, id: String) -> Result<String, String> {
    use sysinfo::{Pid, System};
    use std::process::Command;
    use std::os::windows::process::CommandExt;
    use base64::{Engine as _, engine::general_purpose};
    use crate::storage::TokenData;

    // --- Phase 1: Load & Validate ---
    let mut accounts = load_accounts(&app);
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
        (td.access_token, td.refresh_token, td.expires_at)
    } else {
        // Fallback: Use legacy token field (likely an access token from auto-import)
        let acc = &accounts[matching_account_index.unwrap()];
        let access = acc.token.clone();
        // We don't have a refresh token, so we use the access token as both
        // This will work for the current session but won't be refreshable
        let expiry = chrono::Utc::now().timestamp() + 3600; // Assume 1 hour validity
        (access.clone(), access, expiry)
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
            // Step A: Polite close
            let _ = Command::new("taskkill").args(&["/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            // Step B: Ensure it's dead
            let _ = Command::new("taskkill").args(&["/F", "/IM", "Antigravity.exe"]).creation_flags(CREATE_NO_WINDOW).output();
        }
        println!("Antigravity was running, killed it");
    } else {
        println!("Antigravity is not running, skipping kill step");
    }

    // --- Phase 4: Database Injection ---
    let mut target_dbs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        target_dbs.push(home.join("AppData/Roaming/Antigravity/User/globalStorage/state.vscdb"));
        target_dbs.push(home.join("AppData/Roaming/com.antigravity.manager/storage.sqlite"));
    }
    // Portable support - 使用缓存的路径
    if let Some(ref dir) = antigravity_dir {
        target_dbs.push(dir.join("data/user-data/User/globalStorage/state.vscdb"));
        target_dbs.push(dir.join("data/storage.sqlite"));
    }

    // Construct Protobuf Payload
    fn make_varint(mut v: u64) -> Vec<u8> {
        let mut b = Vec::new();
        while v >= 0x80 { b.push((v & 0x7F | 0x80) as u8); v >>= 7; }
        b.push(v as u8);
        b
    }

    let mut oauth_block = Vec::new();
    // Field 1: Access Token
    oauth_block.extend(make_varint((1 << 3) | 2));
    oauth_block.extend(make_varint(final_access.len() as u64));
    oauth_block.extend(final_access.as_bytes());
    // Field 2: Bearer
    oauth_block.extend(make_varint((2 << 3) | 2));
    oauth_block.extend(make_varint(6));
    oauth_block.extend(b"Bearer");
    // Field 3: Refresh Token
    oauth_block.extend(make_varint((3 << 3) | 2));
    oauth_block.extend(make_varint(final_refresh.len() as u64));
    oauth_block.extend(final_refresh.as_bytes());
    // Field 4: Expiry
    let mut timestamp_msg = Vec::new();
    timestamp_msg.extend(make_varint((1 << 3) | 0));
    timestamp_msg.extend(make_varint(final_expiry as u64));
    oauth_block.extend(make_varint((4 << 3) | 2));
    oauth_block.extend(make_varint(timestamp_msg.len() as u64));
    oauth_block.extend(timestamp_msg);

    // Final Wrap
    let mut final_payload = Vec::new();
    final_payload.extend(make_varint((6 << 3) | 2));
    final_payload.extend(make_varint(oauth_block.len() as u64));
    final_payload.extend(oauth_block);

    // Inject
    let mut success_count = 0;
    let mut log_msgs = Vec::new();

    for db in target_dbs {
        if !db.exists() { continue; }
        if let Ok(conn) = rusqlite::Connection::open(&db) {
            let _ = conn.execute("PRAGMA busy_timeout = 3000;", []);
            
            // 1. Clean bad data
            let _ = conn.execute("DELETE FROM ItemTable WHERE key = 'jetskiStateSync.agentManagerInitState'", []);
            
            // 2. Insert New Protobuf (Base64)
            let encoded = general_purpose::STANDARD.encode(&final_payload);
            let res_v2 = conn.execute(
                "INSERT INTO ItemTable (key, value) VALUES ('jetskiStateSync.agentManagerInitState', ?1)",
                [&encoded],
            );

            // 3. Insert Onboarding Flag
            let _ = conn.execute("INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('antigravityOnboarding', 'true')", []);

            // 4. Legacy Support (using same final_access)
            let res_v1 = conn.execute(
                "INSERT OR REPLACE INTO ItemTable (key, value) VALUES ('current_user', ?1)",
                [&final_access],
            );

            if res_v2.is_ok() || res_v1.is_ok() {
                success_count += 1;
                log_msgs.push(format!("Updated {:?}", db.file_name().unwrap()));
            }
        }
    }

    // --- Phase 5: Restart (only if it was running before) ---
    if was_running {
        #[cfg(target_os = "windows")]
        {
            std::thread::sleep(std::time::Duration::from_millis(1000));
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
        println!("Antigravity was not running, skipping restart");
    }

    Ok(format!("Switched ({} DBs synced)", success_count))
}

#[tauri::command]
fn get_config(app: AppHandle) -> BoosterConfig {
    load_config(&app)
}

#[tauri::command]
fn set_config(app: AppHandle, config: BoosterConfig) -> Result<(), String> {
    save_config(&app, &config).map_err(|e| e.to_string())
}

mod quota;

#[tauri::command]
async fn fetch_account_quota(app: AppHandle, id: String) -> Result<storage::QuotaData, String> {
    let mut accounts = load_accounts(&app);
    // User requested to use system proxy directly, so we pass None.
    // reqwest Client uses system proxy by default if not overridden.
    let proxy_url = None;

    if let Some(acc) = accounts.iter_mut().find(|a| a.id == id) {
        // Decide if we need to refresh
        let access_token = if acc.token.starts_with("1//") {
            // It's a refresh token, get a fresh access token
            match oauth::refresh_access_token(&acc.token, proxy_url.clone()).await {
                Ok(at) => at,
                Err(e) => return Err(format!("Quota error: Token refresh failed: {}", e)),
            }
        } else {
            acc.token.clone()
        };

        // Use real fetcher with proxy
        let (new_quota, detected_tier, debug_logs) = quota::fetch_account_quota_real(&access_token, &acc.email, proxy_url).await?;
        
        // Emit debug logs to frontend
        let _ = app.emit("debug-log", debug_logs);

        acc.quota = Some(new_quota.clone());
        // Update tier if valid
        if !detected_tier.is_empty() {
             acc.account_type = detected_tier;
        } else {
             acc.account_type = "Gemini".to_string();
        }

        save_accounts(&app, &accounts).map_err(|e| e.to_string())?;
        let _ = crate::history::record_quota_point(&app);
        Ok(new_quota)
    } else {
        Err("Account not found".to_string())
    }
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

#[tauri::command]
fn detect_system_proxy() -> String {
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

#[tauri::command]
async fn sync_active_status(app: AppHandle) -> Result<String, String> {
    use rusqlite::Connection;
    use base64::{Engine as _, engine::general_purpose};

    // 0. Get proxy config for the user info request
    let config = load_config(&app);
    let proxy_url = if config.proxy_enabled {
        Some(format!("{}://{}:{}", config.proxy_type, config.proxy_host, config.proxy_port))
    } else {
        None
    };

    // 1. Identify DB Paths
    let mut target_dbs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        target_dbs.push(home.join("AppData/Roaming/Antigravity/User/globalStorage/state.vscdb"));
        target_dbs.push(home.join("AppData/Roaming/com.antigravity.manager/storage.sqlite"));
    }

    let mut found_token = String::new();

    // 2. Read DB to find current token
    for db in target_dbs {
        if !db.exists() { continue; }
        if let Ok(conn) = Connection::open(&db) {
            let _ = conn.execute("PRAGMA busy_timeout = 1000;", []);
            
            // Try V2 match (Base64 Protobuf)
            let stmt = conn.prepare("SELECT value FROM ItemTable WHERE key = 'jetskiStateSync.agentManagerInitState'").ok();
            if let Some(mut s) = stmt {
                let rows = s.query_map([], |row| row.get::<_, String>(0)).ok();
                if let Some(row_iter) = rows {
                    for r in row_iter {
                        if let Ok(b64) = r {
                            if let Ok(bytes) = general_purpose::STANDARD.decode(&b64) {
                                if let Ok(s) = String::from_utf8(bytes) {
                                    if let Some(idx) = s.find("ya29.") {
                                        let substr = &s[idx..];
                                        let end = substr.find(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '_' && c != '-').unwrap_or(substr.len());
                                        found_token = substr[0..end].to_string();
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !found_token.is_empty() { break; }

            // Try V1 match (Plain String)
            let stmt = conn.prepare("SELECT value FROM ItemTable WHERE key = 'current_user'").ok();
            if let Some(mut s) = stmt {
                let rows = s.query_map([], |row| row.get::<_, String>(0)).ok();
                if let Some(row_iter) = rows {
                    for r in row_iter {
                        if let Ok(t) = r {
                            if t.starts_with("ya29.") {
                                found_token = t;
                            }
                        }
                    }
                }
            }
        }
        if !found_token.is_empty() { break; }
    }

    if found_token.is_empty() {
        return Ok("No active Antigravity session found.".into());
    }

    // 3. Resolve Identity of found token (Crucial for robust matching)
    let user_info = match oauth::get_user_info(&found_token, proxy_url).await {
        Ok(info) => Some(info),
        Err(_) => None, // Token might be valid but info request failed
    };

    // 4. Update or Insert
    let mut accounts = load_accounts(&app);
    let mut changed = false;
    let mut matched_index = None;
    let mut matched_name = "Unknown".to_string();

    let target_email = user_info.as_ref().map(|u| u.email.clone());

    // Try to find existing by email OR exact token match
    for (i, acc) in accounts.iter_mut().enumerate() {
        let is_email_match = target_email.is_some() && target_email.as_ref().unwrap() == &acc.email;
        let is_token_match = if let Some(td) = &acc.token_data {
            td.access_token == found_token
        } else {
            acc.token == found_token
        };

        if is_email_match || is_token_match {
            matched_index = Some(i);
            matched_name = acc.name.clone();
            
            // DO NOT modify token_data here!
            // The account already has proper OAuth data from login.
            // We only care about setting is_active flag.

            if !acc.is_active {
                acc.is_active = true;
                changed = true;
            }
        } else {
            // Unmark others
            if acc.is_active {
                acc.is_active = false;
                changed = true;
            }
        }
    }

    // If not found, Auto-Import (only if we have user info)
    if matched_index.is_none() && user_info.is_some() {
        let new_id = uuid::Uuid::new_v4().to_string();
        matched_name = user_info.as_ref().and_then(|u| u.name.clone()).unwrap_or_else(|| "Antigravity 当前用户".to_string());
        
        let new_acc = crate::storage::Account {
            id: new_id,
            name: matched_name.clone(),
            email: target_email.unwrap_or_else(|| "auto-sync@antigravity".into()),
            token: found_token.clone(),
            token_data: None, 
            account_type: "Gemini".into(),
            status: "active".into(),
            quota: None,
            is_active: true,
        };
        
        accounts.push(new_acc);
        changed = true;
    }

    if changed {
        save_accounts(&app, &accounts).map_err(|e| e.to_string())?;
        Ok(format!("Synced status: Active account is '{}'", matched_name))
    } else {
        Ok("Status already synced.".into())
    }
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
            acc.quota = Some(new_quota);
            if !detected_tier.is_empty() {
                acc.account_type = detected_tier;
            }
        }
    }
    let _ = save_accounts(&app, &accounts);
    let _ = history::record_quota_point(&app);
    let _ = app.emit("quota-updated", ());
    
    // Then continue with 5-minute interval
    loop {
        sleep(Duration::from_secs(300)).await; // 5 minutes
        
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
                acc.quota = Some(new_quota);
                if !detected_tier.is_empty() {
                    acc.account_type = detected_tier;
                }
            }
        }
        
        let _ = save_accounts(&app, &accounts);
        let _ = history::record_quota_point(&app);
        let _ = app.emit("quota-updated", ());
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .setup(|app| {
            let app_handle = app.handle().clone();
            
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
            get_accounts,
            save_account,
            delete_account,
            switch_account,
            sync_active_status, // Added here
            get_config,
            set_config,
            fetch_account_quota,
            start_boosting,
            stop_boosting,
            detect_system_proxy,
            migration::import_from_antigravity_v1,
            oauth::import_backup,
            oauth::export_backup,
            oauth::start_oauth_login,
            get_usage_chart,
            enable_system_proxy,
            disable_system_proxy,
            is_proxy_enabled
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
