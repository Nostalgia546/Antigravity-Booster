use rusqlite::Connection;
use crate::storage::{Account, QuotaData, ModelQuota};
use crate::storage::{save_accounts, load_accounts};

/// Robust migration for Antigravity Manager V1/V2
/// Reads from the manager's SQLite database to extract valid session tokens.
#[tauri::command]
pub async fn import_from_antigravity_v1(app: tauri::AppHandle) -> Result<Vec<Account>, String> {
    let home = dirs::home_dir().ok_or("Home directory not found")?;
    
    // Antigravity Manager typical DB locations (Windows)
    let db_paths = vec![
        home.join("AppData/Roaming/com.antigravity.manager/storage.sqlite"),
        home.join(".antigravity-agent/accounts.sqlite"),
        home.join(".antigravity-agent/storage.sqlite"),
    ];

    let mut imported = Vec::new();

    for path in db_paths {
        if !path.exists() { continue; }

        let conn = Connection::open(&path).map_err(|e| format!("DB Error: {}", e))?;
        
        let mut stmt = conn.prepare("SELECT key, value FROM ItemTable WHERE key LIKE 'account_%' OR key = 'current_user'")
                           .map_err(|e| e.to_string())?;
        
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }).map_err(|e| e.to_string())?;

        for row in rows {
            if let Ok((key, value)) = row {
                let acc = Account {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: format!("Imported ({})", key),
                    email: "migrated@prev-app.com".into(),
                    token: value, 
                    token_data: None, // V1 migration has no refresh token data
                    account_type: "Gemini".into(),
                    status: "active".into(),
                    quota: Some(QuotaData {
                        models: vec![ModelQuota { name: "Migrated".into(), percentage: 100.0, reset_time: "Fresh".into() }],
                        last_updated: chrono::Utc::now().timestamp(),
                    }),
                    is_active: false,
                };
                imported.push(acc);
            }
        }
    }

    if !imported.is_empty() {
        let mut existing = load_accounts(&app);
        existing.extend(imported.clone());
        save_accounts(&app, &existing).map_err(|e| e.to_string())?;
    } else {
        return Err("No Antigravity Manager data found or database format unrecognized.".into());
    }

    Ok(imported)
}
