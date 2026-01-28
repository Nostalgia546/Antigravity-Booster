/**
 * History Service - Quota usage history tracking
 * 
 * Records quota snapshots periodically and provides chart data.
 */

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const MAX_HISTORY_HOURS: i64 = 24 * 7; // 7 days

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuotaHistoryPoint {
    pub timestamp: i64,
    /// 账户模型 ID -> 剩余百分比
    pub usage: HashMap<String, f64>,
    /// 账户模型 ID -> 绝对重置时间戳 (用于精准判定是否发生了重置)
    #[serde(default)]
    pub reset_at: HashMap<String, i64>,
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
            let res: Result<Vec<QuotaHistoryPoint>, _> = serde_json::from_str(&content);
            match res {
                Ok(points) => {
                    // Filter out old points
                    let cutoff = chrono::Utc::now().timestamp() - MAX_HISTORY_HOURS * 3600;
                    points.into_iter().filter(|p| p.timestamp > cutoff).collect()
                }
                Err(e) => {
                    println!("Error parsing history json: {}. Content len: {}", e, content.len());
                    vec![]
                }
            }
        }
        Err(e) => {
            println!("Error reading history file: {}", e);
            vec![]
        },
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
    
    for acc in &accounts {
        if let Some(quota) = &acc.quota {
            for model in &quota.models {
                let key = format!("{}:{}", acc.id, model.name);
                usage_map.insert(key, model.percentage);
            }
        }
    }
    
    let now = chrono::Utc::now().timestamp();
    let mut reset_at = HashMap::new();
    
    // 同时也记录重置绝对时间戳，用于计算消耗量时识别重置动作
    for acc in &accounts {
        if let Some(quota) = &acc.quota {
            for model in &quota.models {
                let key = format!("{}:{}", acc.id, model.name);
                if let Some(ts) = model.reset_at {
                    reset_at.insert(key, ts);
                }
            }
        }
    }

    let point = QuotaHistoryPoint {
        timestamp: now,
        usage: usage_map,
        reset_at,
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
    
    // 核心修复：向上取整。确保“当前正在进行的周期”也能作为最后一根柱子显示在右侧。
    // 解决“最新数据看不到”的问题。例如 17:44 时，aligned_end 将对齐到 18:00，
    // 从而让 17:30~18:00 这根代表当前的柱子能够成功生成。
    let bucket_seconds = bucket_minutes * 60;
    let aligned_end = ((now / bucket_seconds) + 1) * bucket_seconds;
    
    // 从对齐后的终点往回推 display_minutes，确保窗口始终平滑滑动
    let start_time = aligned_end - display_minutes * 60;
    let bucket_count = (display_minutes / bucket_minutes) as usize;
    
    // 恢复标准顺序：从左往右 (旧 -> 新)
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
    
    // 辅助结构：记录每个桶在其时间范围内是否存在"隐式使用中"的历史点
    // Key -> bucket_index -> is_implicitly_in_use
    let mut implicit_use_flags: HashMap<String, Vec<bool>> = HashMap::new();

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
                // 读取重置绝对时间戳，判断该区间内是否发生了重置 (Google 周期刷新)
                let r1 = p1.reset_at.get(key);
                let r2 = p2.reset_at.get(key);
                
                let consumed = if r1 != r2 && r1.is_some() && r2.is_some() {
                    // 发生了重置！这种情况下，消耗量 = (100% - val2)
                    (100.0 - val2).max(0.0)
                } else {
                    // 正常情况：消耗量 = 之前的余量 - 现在的余量
                    // 注意：如果余量不降反增且没有显式重置 (可能是 API 抖动)，记为 0
                    (val1 - val2).max(0.0)
                };
                
                // === 核心新增：检测"隐式使用中"状态 ===
                // 条件：余量 ≥ 99.9% 且 距离下次重置 < 299分钟（4小时59分）
                // 这意味着用户正在使用，但额度变化还未体现出来
                let is_p1_implicit_use = if let Some(&reset_ts) = r1 {
                    let remaining_secs = reset_ts - t1;
                    val1 >= 99.9 && remaining_secs > 0 && remaining_secs < 299 * 60
                } else { false };

                // 将该区间产生的消耗量分配到对应的桶中
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
                        
                        // 记录隐式使用标志
                        if is_p1_implicit_use {
                            implicit_use_flags.entry(key.clone())
                                .or_insert_with(|| vec![false; bucket_count])[b_idx] = true;
                        }
                    }
                }
            }
        }
    }
    
    // === 核心新增：两步回溯均匀化后处理 ===
    // 对于每个 key，如果发现 [≈0消耗] → [有消耗] 的模式，
    // 且前一个桶被标记为"隐式使用中"，则将消耗均匀分配
    for (key, bucket_values) in distribution.iter_mut() {
        let flags = implicit_use_flags.get(key);
        
        for b_idx in 1..bucket_count {
            let current = bucket_values[b_idx];
            let prev = bucket_values[b_idx - 1];
            
            // 触发条件：当前桶有显著消耗（>1%），前一桶几乎没有（<0.5%），且前一桶被标记为隐式使用
            let prev_is_implicit = flags.map(|f| f[b_idx - 1]).unwrap_or(false);
            
            if current > 1.0 && prev < 0.5 && prev_is_implicit {
                // 均匀分配：将当前桶的一半移动到前一桶
                let half = current / 2.0;
                bucket_values[b_idx] = half;
                bucket_values[b_idx - 1] += half;
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
