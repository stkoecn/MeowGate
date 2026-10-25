pub mod clash_api;
pub mod country;
pub mod detector;
pub mod yaml_parser;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    #[serde(rename = "type")]
    pub proxy_type: String,
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub port: u16,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Node {
    /// Dedup key: same server + port + type counts as the same physical node,
    /// regardless of how it is named in different subscription files.
    pub fn dedup_key(&self) -> String {
        format!("{}:{}:{}", self.server.to_ascii_lowercase(), self.port, self.proxy_type.to_ascii_lowercase())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub node_name: String,
    pub username: String,
    pub password: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
