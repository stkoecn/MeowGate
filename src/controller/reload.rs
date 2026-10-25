use crate::error::{AppError, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use std::path::Path;

pub struct MeowReloader {
    client: reqwest::Client,
    api_url: String,
    secret: Option<String>,
}

impl MeowReloader {
    pub fn new(api_url: &str, secret: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            api_url: api_url.trim_end_matches('/').to_string(),
            secret,
        }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(secret) = &self.secret {
            if !secret.is_empty() {
                if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", secret)) {
                    headers.insert(AUTHORIZATION, val);
                }
            }
        }
        headers
    }

    /// Triggers a hot reload via `PUT /configs` using a configuration file path on disk.
    pub async fn reload(&self, config_path: &Path) -> Result<()> {
        let absolute_path = std::fs::canonicalize(config_path)
            .unwrap_or_else(|_| config_path.to_path_buf());
        let payload = std::fs::read_to_string(&absolute_path)
            .map_err(|e| AppError::Meow(format!("Failed to read generated config: {}", e)))?;
        self.reload_with_payload(&absolute_path, &payload).await
    }

    /// Triggers a hot reload via `PUT /configs` supplying the in-memory YAML payload directly.
    ///
    /// Avoids an extra full-file read from disk right after writing it.
    /// The authoritative Clash/Mihomo API takes `{"path": "...", "payload": "<yaml>"}` with `?force=true`.
    pub async fn reload_with_payload(&self, config_path: &Path, payload: &str) -> Result<()> {
        let absolute_path = std::fs::canonicalize(config_path)
            .unwrap_or_else(|_| config_path.to_path_buf());
        let path_str = absolute_path.to_string_lossy().to_string();

        let url = format!("{}/configs?force=true", self.api_url);
        let body = json!({
            "path": path_str,
            "payload": payload,
        });

        let resp = self
            .client
            .put(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Meow(format!("Failed to call meow-rs reload API: {}", e)))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Meow(format!(
                "meow-rs reload returned error status {}: {}",
                status, text
            )));
        }

        tracing::info!(
            "meow-rs hot reload applied ({} bytes) for {}",
            payload.len(),
            path_str
        );
        Ok(())
    }
}
