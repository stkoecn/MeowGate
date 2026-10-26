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
    /// Extracts the credential/identity indicator of a node (e.g. UUID, password, username, or token).
    /// Used to distinguish multi-user nodes operating on the same physical server and port.
    pub fn credential_id(&self) -> Option<String> {
        // 1. Check direct common credential fields in extra
        for key in &[
            "uuid", "password", "username", "user", "auth", "auth-str", "auth_str",
            "token", "psk", "secret", "private-key", "private_key", "public-key", "public_key",
        ] {
            if let Some(val) = self.extra.get(*key) {
                if let Some(s) = val.as_str() {
                    let s_trimmed = s.trim();
                    if !s_trimmed.is_empty() {
                        return Some(s_trimmed.to_string());
                    }
                }
            }
        }
        // 2. Check user/pass combination if both present (e.g. socks5 / http)
        let user = self.extra.get("user").and_then(|v| v.as_str()).map(|s| s.trim()).unwrap_or("");
        let pass = self.extra.get("pass").and_then(|v| v.as_str()).map(|s| s.trim()).unwrap_or("");
        if !user.is_empty() || !pass.is_empty() {
            return Some(format!("{}:{}", user, pass));
        }
        None
    }

    /// Dedup key: combines server + port + proxy_type + optional credential_id.
    ///
    /// If two nodes share the same physical server and port, but belong to different
    /// users (different UUID / password / token), they will have different dedup keys
    /// and will both be retained for distinct user routing.
    pub fn dedup_key(&self) -> String {
        let base = format!("{}:{}:{}", self.server.to_ascii_lowercase(), self.port, self.proxy_type.to_ascii_lowercase());
        if let Some(cred) = self.credential_id() {
            format!("{}:{}", base, cred)
        } else {
            base
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub node_name: String,
    pub username: String,
    pub password: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
