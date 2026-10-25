use crate::collector::Node;
use crate::error::{AppError, Result};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::Value;

pub struct ClashApiClient {
    client: reqwest::Client,
    api_url: String,
    secret: Option<String>,
}

impl ClashApiClient {
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
        if let Some(secret) = &self.secret {
            if !secret.is_empty() {
                if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", secret)) {
                    headers.insert(AUTHORIZATION, val);
                }
            }
        }
        headers
    }

    pub async fn check_health(&self) -> bool {
        let url = format!("{}/version", self.api_url);
        match self.client.get(&url).headers(self.headers()).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn fetch_nodes(&self) -> Result<Vec<Node>> {
        let url = format!("{}/proxies", self.api_url);
        let resp = self
            .client
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| AppError::Clash(format!("Failed to connect to Clash API ({}): {}", url, e)))?;

        if !resp.status().is_success() {
            return Err(AppError::Clash(format!(
                "Clash API returned status {}",
                resp.status()
            )));
        }

        let json_val: Value = resp
            .json()
            .await
            .map_err(|e| AppError::Clash(format!("Failed to parse Clash API response: {}", e)))?;

        let proxies_obj = json_val
            .get("proxies")
            .and_then(Value::as_object)
            .ok_or_else(|| AppError::Clash("Clash response missing 'proxies' object".to_string()))?;

        let mut nodes = Vec::new();

        // Clash /proxies includes selector/URLTest groups as well as individual proxies.
        // We filter out meta-types like Selector, URLTest, Fallback, Direct, Reject, Compatible.
        let ignored_types = [
            "Selector", "URLTest", "Fallback", "Direct", "Reject", "Compatible", "Relay", "Pass",
        ];

        for (name, proxy_val) in proxies_obj {
            let proxy_type = proxy_val
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();

            if ignored_types.iter().any(|&t| t.eq_ignore_ascii_case(proxy_type)) {
                continue;
            }

            // Convert to Node struct
            if let Ok(mut node) = serde_json::from_value::<Node>(proxy_val.clone()) {
                node.name = name.clone();
                nodes.push(node);
            }
        }

        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(nodes)
    }
}
