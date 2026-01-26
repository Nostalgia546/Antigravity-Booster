use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::storage::{QuotaData, ModelQuota};

const CLOUD_CODE_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";
const QUOTA_API_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
const USER_AGENT: &str = "antigravity/1.11.3 Darwin/arm64";

#[derive(Debug, Deserialize)]
struct LoadProjectResponse {
    #[serde(rename = "cloudaicompanionProject")]
    project_id: Option<String>,
    #[serde(rename = "currentTier")]
    current_tier: Option<Tier>,
    #[serde(rename = "paidTier")]
    paid_tier: Option<Tier>,
}

#[derive(Debug, Deserialize)]
struct Tier {
    id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct QuotaResponse {
    models: std::collections::HashMap<String, ModelInfo>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    #[serde(rename = "quotaInfo")]
    quota_info: Option<QuotaInfo>,
}

#[derive(Debug, Deserialize)]
struct QuotaInfo {
    #[serde(rename = "remainingFraction")]
    remaining_fraction: Option<f64>,
    #[serde(rename = "resetTime")]
    reset_time: Option<String>,
}

pub async fn fetch_account_quota_real(access_token: &str, _email: &str, proxy_url: Option<String>) -> Result<(QuotaData, String, String), String> {
    let mut debug_log = String::new();
    debug_log.push_str("Starting fetch_account_quota_real...\n");

    let mut client_builder = Client::builder()
        .user_agent(USER_AGENT);

    if let Some(url) = proxy_url {
        if !url.is_empty() {
             if let Ok(proxy) = reqwest::Proxy::all(&url) {
                 client_builder = client_builder.proxy(proxy);
             }
        }
    }

    let client = client_builder.build().map_err(|e| e.to_string())?;

    // Helper for project ID fetching with same client
    // We cannot change signature of inner helper easily to return logs string without major refactor, 
    // so we inline the logic or keep it simple. Let's inline a bit of logic or accept we capture logs outside.
    // Actually, we can just rewrite the helper to return logs too.
    
    debug_log.push_str("Fetching Project/Tier info...\n");
    let meta = json!({"metadata": {"ideType": "ANTIGRAVITY"}});
    let pid_res = client
        .post(format!("{}/v1internal:loadCodeAssist", CLOUD_CODE_BASE_URL))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", access_token))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&meta)
        .send()
        .await;

    let (project_id, tier_id_opt) = match pid_res {
        Ok(res) => {
            let body = res.text().await.unwrap_or_default();
            // debug_log.push_str(&format!("loadCodeAssist Body Length: {}\n", body.len()));
            
            if let Ok(data) = serde_json::from_str::<LoadProjectResponse>(&body) {
                    let subscription_tier = data.paid_tier
                    .and_then(|t| t.id)
                    .or_else(|| data.current_tier.and_then(|t| t.id));
                (data.project_id, subscription_tier)
            } else {
                debug_log.push_str("JSON Parse Error for LoadProjectResponse\n");
                (None, None)
            }
        }
        Err(e) => {
            debug_log.push_str(&format!("loadCodeAssist Network Error: {}\n", e));
            (None, None)
        }
    };

    let final_pid = project_id.unwrap_or_else(|| "bamboo-precept-lgxtn".to_string());
    
    // Resolve Tier Name
    let tier_display = match tier_id_opt.as_deref() {
        Some("gemini_code_assist_premium") | Some("cloudaicompanion_gemini_code_assist_premium") => "Ultra".to_string(), // Assume this is Ultra
        // ID G1-PRO-TIER -> Pro
        Some("G1-PRO-TIER") | Some("g1-pro-tier") => "Pro".to_string(),
        Some("gemini_code_assist_business") => "Business".to_string(),
        Some("gemini_code_assist_enterprise") => "Enterprise".to_string(),
        Some(other) => format!("Raw: {}", other), 
        None => "Gemini".to_string()
    };

    debug_log.push_str(&format!("Resolved Display: {}\n", tier_display));

    let payload = json!({ "project": final_pid });

    let res = client
        .post(QUOTA_API_URL)
        .bearer_auth(access_token)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Network Error: {}", e))?;

    if res.status().is_success() {
        let data: QuotaResponse = res.json().await.map_err(|e| e.to_string())?;
        
        let mut models = Vec::new();

        for (name, info) in data.models {
            // 1. Identify if this is a model we care about
            let display_name = match name.as_str() {
                "gemini-3-pro-high" => "Gemini Pro",
                "gemini-3-flash" => "Gemini Flash",
                "claude-sonnet-4-5" => "Claude",
                 _ => continue,
            };

            // 2. Extract quota
            let (pct, raw_time) = if let Some(q) = info.quota_info {
                (q.remaining_fraction.map(|f| (f * 100.0) as f64).unwrap_or(0.0), q.reset_time)
            } else { (0.0, None) };
            
            let formatted_time = if let Some(t_str) = raw_time {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&t_str) {
                    let now = chrono::Utc::now();
                    let diff = dt.with_timezone(&chrono::Utc).signed_duration_since(now);
                    let local_dt = dt.with_timezone(&chrono::Local);
                    let time_str = local_dt.format("%Y-%m-%d %H:%M").to_string();

                    if diff.num_seconds() > 0 {
                        let hours = diff.num_hours();
                        let mins = diff.num_minutes() % 60;
                        format!("{} (剩余 {}小时 {}分)", time_str, hours, mins)
                    } else {
                         format!("{} (已重置)", time_str)
                    }
                } else { t_str }
            } else { "每日重置".to_string() };

            models.push(ModelQuota {
                name: display_name.to_string(),
                percentage: pct,
                reset_time: formatted_time,
            });
        }

        // Sort
        models.sort_by(|a, b| {
            let score = |name: &str| {
                if name.contains("Pro") { 1 } else if name.contains("Flash") { 2 } else { 3 }
            };
            score(&a.name).cmp(&score(&b.name))
        });

        Ok((QuotaData {
            models,
            last_updated: chrono::Utc::now().timestamp(),
        }, tier_display, debug_log))
    } else {
        Err(format!("API Error: {}", res.status()))
    }
}
