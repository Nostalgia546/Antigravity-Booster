use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelQuota {
    pub name: String,
    pub percentage: f64,
    pub reset_time: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuotaData {
    pub models: Vec<ModelQuota>,
    pub last_updated: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenData {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
    pub token: String, // Deprecated: legacy string token
    pub token_data: Option<TokenData>, // NEW: Full OAuth data
    pub account_type: String, // Gemini, Claude, OpenAI
    pub status: String,
    pub quota: Option<QuotaData>,
    pub is_active: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BoosterConfig {
    pub proxy_enabled: bool,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub proxy_type: String, // "http", "socks5"
    pub target_processes: Vec<String>,
    pub antigravity_executable: Option<String>,
}

impl Default for BoosterConfig {
    fn default() -> Self {
        Self {
            proxy_enabled: false,
            proxy_host: "127.0.0.1".into(),
            proxy_port: 7890,
            proxy_type: "http".into(),
            target_processes: vec!["Antigravity.exe".into(), "Cursor.exe".into()],
            antigravity_executable: None,
        }
    }
}

pub fn get_app_dir(app_handle: &tauri::AppHandle) -> PathBuf {
    app_handle.path().app_data_dir().unwrap_or_else(|_| {
        let mut path = dirs::home_dir().unwrap();
        path.push(".antigravity-booster");
        path
    })
}

pub fn save_accounts(app_handle: &tauri::AppHandle, accounts: &Vec<Account>) -> anyhow::Result<()> {
    let mut path = get_app_dir(app_handle);
    fs::create_dir_all(&path)?;
    path.push("accounts.json");
    let content = serde_json::to_string_pretty(accounts)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn load_accounts(app_handle: &tauri::AppHandle) -> Vec<Account> {
    let mut path = get_app_dir(app_handle);
    path.push("accounts.json");
    if !path.exists() {
        return vec![];
    }
    let content = fs::read_to_string(path).unwrap_or_else(|_| "[]".to_string());
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn save_config(app_handle: &tauri::AppHandle, config: &BoosterConfig) -> anyhow::Result<()> {
    let mut path = get_app_dir(app_handle);
    fs::create_dir_all(&path)?;
    path.push("config.json");
    let content = serde_json::to_string_pretty(config)?;
    fs::write(path, content)?;
    Ok(())
}

pub fn load_config(app_handle: &tauri::AppHandle) -> BoosterConfig {
    let mut path = get_app_dir(app_handle);
    path.push("config.json");
    if !path.exists() {
        return BoosterConfig {
            proxy_enabled: false,
            proxy_host: "127.0.0.1".to_string(),
            proxy_port: 7890,
            proxy_type: "socks5".to_string(),
            target_processes: vec![],
            antigravity_executable: None,
        };
    }
    let content = fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
    serde_json::from_str(&content).unwrap_or_else(|_| BoosterConfig {
        proxy_enabled: false,
        proxy_host: "127.0.0.1".to_string(),
        proxy_port: 7890,
        proxy_type: "socks5".to_string(),
        target_processes: vec![],
        antigravity_executable: None,
    })
}
