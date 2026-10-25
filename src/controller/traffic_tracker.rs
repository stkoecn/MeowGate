use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeTrafficStats {
    pub total_download: u64,
    pub total_upload: u64,
    pub conn_count: u64,
    pub active_conns: usize,
    pub peak_speed_bps: u64,
    pub last_speed_bps: u64,
    pub last_speed_formatted: Option<String>,
    pub peak_speed_formatted: Option<String>,
    pub total_download_formatted: String,
    pub last_active: Option<DateTime<Utc>>,
}

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}
