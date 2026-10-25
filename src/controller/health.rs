use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};

pub struct MeowHealthChecker {
    client: reqwest::Client,
    api_url: String,
    secret: Option<String>,
}

impl MeowHealthChecker {
    pub fn new(api_url: &str, secret: Option<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
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

    pub async fn check(&self) -> (bool, Option<u128>) {
        let start = std::time::Instant::now();
        let url = format!("{}/version", self.api_url);
        match self.client.get(&url).headers(self.headers()).send().await {
            Ok(resp) if resp.status().is_success() => {
                let latency = start.elapsed().as_millis();
                (true, Some(latency))
            }
            _ => (false, None),
        }
    }
}
