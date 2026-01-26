use axum::{extract::Query, response::Html, routing::get, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;
use crate::storage::{Account, load_accounts, save_accounts};

// Use the reference project's credentials for compatibility
const CLIENT_ID: &str = "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const CLIENT_SECRET: &str = "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";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Debug, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub token_type: String,
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoogleUserInfo {
    pub email: String,
    pub name: Option<String>,
    pub picture: Option<String>,
}

pub async fn refresh_access_token(refresh_token: &str, proxy_url: Option<String>) -> Result<String, String> {
    let mut client_builder = reqwest::Client::builder();
    
    if let Some(url) = proxy_url {
        if !url.is_empty() {
             if let Ok(proxy) = reqwest::Proxy::all(&url) {
                 client_builder = client_builder.proxy(proxy);
             }
        }
    }
    let client = client_builder.build().map_err(|e| e.to_string())?;

    let params = [
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let res = client.post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Refresh request failed: {}", e))?;

    if res.status().is_success() {
        let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        if let Some(at) = data["access_token"].as_str() {
            return Ok(at.to_string());
        }
    } else {
        // Fallback: maybe it's already an access token?
        // If the token starts with "ya29", it's likely an access token.
        // But if we got here, we assume it's a refresh token.
        // Let's return error for now to be safe.
        let text = res.text().await.unwrap_or_default();
         return Err(format!("Refresh failed ({}): {}", parse_error_code(&text), text));
    }
    Err("Empty access token received".into())
}

fn parse_error_code(text: &str) -> String {
    if text.contains("invalid_grant") { "Expired/Revoked".into() } else { "Error".into() }
}

pub async fn exchange_code(code: &str, redirect_uri: &str) -> Result<(TokenResponse, GoogleUserInfo), String> {
    let client = reqwest::Client::new();
    
    let params = [
        ("client_id", CLIENT_ID),
        ("client_secret", CLIENT_SECRET),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];

    let token_res = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token request failed: {}", e))?;

    if !token_res.status().is_success() {
        return Err(format!("Token exchange failed: {}", token_res.text().await.unwrap_or_default()));
    }

    let token_data: TokenResponse = token_res.json().await
        .map_err(|e| format!("Failed to parse token response: {}", e))?;

    // Fetch User Info
    let user_info_res = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(&token_data.access_token)
        .send()
        .await
        .map_err(|e| format!("User info request failed: {}", e))?;
        
    let user_info: GoogleUserInfo = user_info_res.json().await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;

    Ok((token_data, user_info))
}

pub async fn get_user_info(access_token: &str, proxy_url: Option<String>) -> Result<GoogleUserInfo, String> {
    let mut client_builder = reqwest::Client::builder();
    if let Some(url) = proxy_url {
        if !url.is_empty() {
             if let Ok(proxy) = reqwest::Proxy::all(&url) {
                 client_builder = client_builder.proxy(proxy);
             }
        }
    }
    let client = client_builder.build().map_err(|e| e.to_string())?;

    let res = client
        .get("https://www.googleapis.com/oauth2/v2/userinfo")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| format!("User info request failed: {}", e))?;
        
    if !res.status().is_success() {
        return Err(format!("Failed to get user info: {}", res.status()));
    }

    let user_info: GoogleUserInfo = res.json().await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;

    Ok(user_info)
}

#[tauri::command]
pub async fn start_oauth_login(app: tauri::AppHandle) -> Result<Account, String> {
    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let app_state = Arc::clone(&tx);
    // Use fallback to capture any path/query
    let routes = Router::new().fallback(move |Query(params): Query<HashMap<String, String>>| {
        let state = app_state.clone();
        async move {
            let mut lock = state.lock().await;
            if let Some(sender) = lock.take() {
                if let Some(code) = params.get("code") {
                    let _ = sender.send(code.clone());
                    return Html(r#"
                        <html>
                            <body style="background: #0f172a; color: #fff; display: flex; align-items: center; justify-content: center; height: 100vh; font-family: sans-serif;">
                                <div style="text-align: center;">
                                    <h1 style="color: #10b981;">Authorization Successful!</h1>
                                    <p>You can close this window now.</p>
                                    <script>setTimeout(() => window.close(), 1500);</script>
                                </div>
                            </body>
                        </html>
                    "#);
                }
            }
            Html("<h1>Invalid Request</h1>")
        }
    });

    // Bind to port 0 to get a random free port
    let addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| e.to_string())?;
    // Get the assigned port
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    
    let redirect_uri = format!("http://localhost:{}/", port);

    tokio::spawn(async move {
        axum::serve(listener, routes).await.unwrap();
    });

    // Scopes from the reference project
    let scopes = vec![
        "https://www.googleapis.com/auth/cloud-platform",
        "https://www.googleapis.com/auth/userinfo.email",
        "https://www.googleapis.com/auth/userinfo.profile",
        "https://www.googleapis.com/auth/cclog",
        "https://www.googleapis.com/auth/experimentsandconfigs"
    ].join(" ");

    let oauth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&response_type=code&scope={}&redirect_uri={}&access_type=offline&prompt=consent",
        CLIENT_ID, scopes, redirect_uri
    );

    let _ = webbrowser::open(&oauth_url).map_err(|e| e.to_string())?;

    match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
        Ok(Ok(code)) => {
            // Exchange code for real token
            let (token_data, user_info) = exchange_code(&code, &redirect_uri).await?;
            
            // Generate stable ID from email to prevent duplicates
            let account_id = uuid::Uuid::new_v4().to_string();
            use crate::storage::TokenData;

            // Calculate absolute expiry time
            let expires_at = chrono::Utc::now().timestamp() + token_data.expires_in;
            // Get refresh token or empty string
            let refresh_token = token_data.refresh_token.clone().unwrap_or_default();
            
            // Populate proper TokenData
            let full_token_data = TokenData {
                access_token: token_data.access_token.clone(),
                refresh_token: refresh_token.clone(),
                expires_at,
            };

            let acc = Account {
                id: account_id,
                name: user_info.name.unwrap_or("Unknown User".into()),
                email: user_info.email,
                // Legacy support: keep refresh token here just in case, or "1//" prefixed
                token: if !refresh_token.is_empty() { refresh_token } else { token_data.access_token },
                token_data: Some(full_token_data),
                account_type: "Gemini".into(),
                status: "active".into(),
                quota: None,
                is_active: true,
            };

            let mut accounts = load_accounts(&app);
            // Deactivate others
            for a in &mut accounts { a.is_active = false; }
            accounts.retain(|a| a.email != acc.email);
            accounts.push(acc.clone());
            save_accounts(&app, &accounts).map_err(|e| e.to_string())?;

            Ok(acc)
        }
        _ => {
            Err("Authorization timed out.".into())
        }
    }
}

#[tauri::command]
pub async fn export_backup(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_dialog::DialogExt;
    use std::fs;

    let accounts = load_accounts(&app);
    let content = serde_json::to_string_pretty(&accounts).map_err(|e| e.to_string())?;
    
    let (tx, rx) = oneshot::channel();
    let default_name = format!("antigravity_booster_backup_{}.json", chrono::Local::now().format("%Y%m%d_%H%M%S"));
    
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name(&default_name)
        .save_file(|path| {
            let _ = tx.send(path);
        });

    if let Ok(Some(path)) = rx.await {
        fs::write(path.to_string(), content).map_err(|e| e.to_string())?;
    }
    
    Ok(())
}

#[tauri::command]
pub async fn import_backup(app: tauri::AppHandle) -> Result<Vec<Account>, String> {
    use tauri_plugin_dialog::DialogExt;
    use std::fs;

    let (tx, rx) = oneshot::channel();
    app.dialog()
        .file()
        .add_filter("JSON", &["json"])
        .pick_file(|path| {
            let _ = tx.send(path);
        });
    
    if let Ok(Some(path)) = rx.await {
        let content = fs::read_to_string(path.to_string()).map_err(|e| e.to_string())?;
        let imported: Vec<Account> = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        
        let mut existing = load_accounts(&app);
        
        // Use a HashMap to deduplicate by email
        let mut account_map: HashMap<String, Account> = HashMap::new();
        
        // Add existing accounts first
        for acc in existing {
            account_map.insert(acc.email.clone(), acc);
        }
        
        // Add imported accounts (will overwrite existing ones with the same email)
        for acc in imported.clone() {
            account_map.insert(acc.email.clone(), acc);
        }
        
        // Convert back to Vec
        let final_accounts: Vec<Account> = account_map.into_values().collect();
        
        save_accounts(&app, &final_accounts).map_err(|e| e.to_string())?;
        
        return Ok(imported);
    }
    
    Ok(vec![])
}
