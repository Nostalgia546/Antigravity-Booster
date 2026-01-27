/**
 * History Service - Quota usage history tracking
 * 
 * Records quota snapshots periodically and provides chart data.
 */

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

const MAX_HISTORY_HOURS: i64 = 24 * 7; // 7 days

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuotaHistoryPoint {
    pub timestamp: i64,
    /// Map of "account_id:model_name" -> remaining percentage
    pub usage: HashMap<String, f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BucketItem {
    pub group_id: String,
    pub account_name: String,
    pub model_name: String,
    pub usage: f64,
    pub color: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UsageBucket {
    pub start_time: i64,
    pub end_time: i64,
    pub items: Vec<BucketItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UsageChartData {
    pub buckets: Vec<UsageBucket>,
    pub max_usage: f64,
    pub display_minutes: i64,
    pub interval: i64,
}

/// Get history file path
fn get_history_path(app: &tauri::AppHandle) -> PathBuf {
    let mut path = crate::storage::get_app_dir(app);
    path.push("quota_history.json");
    path
}

/// Load history from disk
pub fn load_history(app: &tauri::AppHandle) -> Vec<QuotaHistoryPoint> {
    let path = get_history_path(app);
    if !path.exists() {
        return vec![];
    }
    
    match fs::read_to_string(&path) {
        Ok(content) => {
            let points: Vec<QuotaHistoryPoint> = serde_json::from_str(&content).unwrap_or_default();
            // Filter out old points
            let cutoff = chrono::Utc::now().timestamp() - MAX_HISTORY_HOURS * 3600;
            points.into_iter().filter(|p| p.timestamp > cutoff).collect()
        }
        Err(_) => vec![],
    }
}

/// Save history to disk
pub fn save_history(app: &tauri::AppHandle, history: &[QuotaHistoryPoint]) -> Result<(), String> {
    let path = get_history_path(app);
    let content = serde_json::to_string_pretty(history).map_err(|e| e.to_string())?;
    fs::write(&path, content).map_err(|e| e.to_string())?;
    Ok(())
}

/// Record a new quota snapshot
pub fn record_quota_point(app: &tauri::AppHandle) -> Result<(), String> {
    let accounts = crate::storage::load_accounts(app);
    let mut usage_map = HashMap::new();
    
    for acc in accounts {
        if let Some(quota) = acc.quota {
            for model in quota.models {
                let key = format!("{}:{}", acc.id, model.name);
                usage_map.insert(key, model.percentage);
            }
        }
    }
    
    let now = chrono::Utc::now().timestamp();
    let point = QuotaHistoryPoint {
        timestamp: now,
        usage: usage_map,
    };
    
    let mut history = load_history(app);
    let before_count = history.len();
    history.push(point);
    
    // Keep only recent history
    let cutoff = now - MAX_HISTORY_HOURS * 3600;
    history.retain(|p| p.timestamp > cutoff);
    let after_count = history.len();
    
    println!("History: Before={}, After={}, Cutoff={}, Now={}", before_count, after_count, cutoff, now);
    
    save_history(app, &history)?;
    Ok(())
}

/// Get model color based on name
fn get_model_color(model_name: &str) -> String {
    let lower = model_name.to_lowercase();
    if lower.contains("pro") {
        "#6366f1".to_string() // Blue for Pro
    } else if lower.contains("flash") {
        "#10b981".to_string() // Green for Flash
    } else {
        "#a855f7".to_string() // Purple for others
    }
}

/// Calculate usage buckets for chart
pub fn calculate_usage_buckets(
    app: &tauri::AppHandle,
    display_minutes: i64,
    bucket_minutes: i64,
) -> UsageChartData {
    let history = load_history(app);
    let accounts = crate::storage::load_accounts(app);
    
    let mut account_names = HashMap::new();
    for acc in &accounts {
        account_names.insert(acc.id.clone(), acc.name.clone());
    }
    
    let now = chrono::Utc::now().timestamp();
    
    // 将结束时间对齐到 bucket_minutes 的整数倍
    // 例如: 如果现在是 19:33，bucket_minutes=30，则对齐到 19:30
    let bucket_seconds = bucket_minutes * 60;
    let aligned_end = (now / bucket_seconds) * bucket_seconds;
    
    // 从对齐的结束时间往回推 display_minutes
    let start_time = aligned_end - display_minutes * 60;
    let bucket_count = (display_minutes / bucket_minutes) as usize;
    
    // Initialize buckets - 从 start_time 开始，每个桶都对齐
    let mut buckets: Vec<UsageBucket> = (0..bucket_count).map(|i| {
        let b_start = start_time + (i as i64) * bucket_minutes * 60;
        UsageBucket {
            start_time: b_start,
            end_time: b_start + bucket_minutes * 60,
            items: Vec::new(),
        }
    }).collect();

    // Key -> bucket_index -> amount
    let mut distribution: HashMap<String, Vec<f64>> = HashMap::new();

    // Process intervals between adjacent history points
    for i in 0..history.len().saturating_sub(1) {
        let p1 = &history[i];
        let p2 = &history[i+1];
        
        // Distribution interval
        let t1 = p1.timestamp;
        let t2 = p2.timestamp;
        if t2 <= t1 { continue; }

        for (key, &val1) in &p1.usage {
            if let Some(&val2) = p2.usage.get(key) {
                // Percentage consumed (e.g. 100% -> 90% = 10% consumed)
                let consumed = (val1 - val2).max(0.0);
                if consumed < 0.001 { continue; }

                // Distribute 'consumed' over [t1, t2] based on bucket overlaps
                let total_duration = (t2 - t1) as f64;
                
                for b_idx in 0..bucket_count {
                    let b = &buckets[b_idx];
                    
                    // Overlap between [t1, t2] and [b.start, b.end]
                    let overlap_start = t1.max(b.start_time);
                    let overlap_end = t2.min(b.end_time);
                    
                    if overlap_end > overlap_start {
                        let overlap_duration = (overlap_end - overlap_start) as f64;
                        let weight = overlap_duration / total_duration;
                        let amount = consumed * weight;
                        
                        distribution.entry(key.clone())
                            .or_insert_with(|| vec![0.0; bucket_count])[b_idx] += amount;
                    }
                }
            }
        }
    }

    // Convert distribution back to bucket items
    for (key, bucket_values) in distribution {
        let parts: Vec<&str> = key.split(':').collect();
        if parts.len() != 2 { continue; }
        
        let account_id = parts[0];
        let model_name = parts[1];
        let account_name = account_names.get(account_id).cloned().unwrap_or_else(|| "Unknown".to_string());
        let color = get_model_color(model_name);

        for (b_idx, &usage) in bucket_values.iter().enumerate() {
            if usage > 0.001 {
                buckets[b_idx].items.push(BucketItem {
                    group_id: key.clone(),
                    account_name: account_name.clone(),
                    model_name: model_name.to_string(),
                    usage,
                    color: color.clone(),
                });
            }
        }
    }

    let max_usage = buckets.iter()
        .map(|b| b.items.iter().map(|it| it.usage).sum::<f64>())
        .fold(0.0, f64::max);

    UsageChartData {
        buckets,
        max_usage: max_usage.max(1.0),
        display_minutes,
        interval: bucket_minutes,
    }
}
