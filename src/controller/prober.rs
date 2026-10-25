use serde::{Deserialize, Serialize};
use std::time::Duration;
use reqwest::Proxy;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeProbeResult {
    pub latency_ms: Option<u64>,
    pub speed_bps: Option<u64>,
    pub speed_formatted: Option<String>,
    pub outbound_ip: Option<String>,
    pub location: Option<String>,
    pub country_code: Option<String>,
    pub error: Option<String>,
    pub checked_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Recent probe history (most recent last), max 10 entries.
    #[serde(default)]
    pub history: Vec<ProbeHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeHistoryEntry {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    #[serde(default)]
    pub speed_bps: Option<u64>,
    pub checked_at: chrono::DateTime<chrono::Utc>,
}

pub fn format_speed_bps(bps: u64) -> String {
    if bps >= 1024 * 1024 * 1024 {
        format!("{:.1} GB/s", bps as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bps >= 1024 * 1024 {
        format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
    } else if bps >= 1024 {
        format!("{:.0} KB/s", bps as f64 / 1024.0)
    } else {
        format!("{} B/s", bps)
    }
}

pub const MAX_HISTORY: usize = 10;
const MAX_PROBER_CLIENTS: usize = 512;

static PROBER_CACHE: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, reqwest::Client>>> =
    std::sync::OnceLock::new();

/// Clears the cached prober clients (called on node creds reset or source sync).
pub fn clear_prober_cache() {
    if let Some(cache) = PROBER_CACHE.get() {
        if let Ok(mut guard) = cache.write() {
            guard.clear();
        }
    }
}

/// Cached probe clients, one per `user:pass@gateway` credential with bounded capacity.
fn client_for(proxy_url: &str, timeout_secs: u64) -> reqwest::Result<reqwest::Client> {
    let cache = PROBER_CACHE.get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()));

    let key = format!("{proxy_url}@{timeout_secs}");
    if let Ok(guard) = cache.read() {
        if let Some(c) = guard.get(&key) {
            return Ok(c.clone());
        }
    }

    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(Proxy::all(proxy_url)?)
        .pool_max_idle_per_host(0)
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;

    if let Ok(mut guard) = cache.write() {
        if guard.len() >= MAX_PROBER_CLIENTS {
            guard.clear();
        }
        guard.insert(key, client.clone());
    }
    Ok(client)
}

#[derive(Deserialize)]
struct IpInfoResponse {
    pub query: Option<String>,
    pub country: Option<String>,
    #[serde(rename = "countryCode")]
    pub country_code: Option<String>,
    pub city: Option<String>,
}

/// Resolves Cloudflare IATA airport codes to Chinese city names
fn colo_to_city(colo: &str) -> Option<&'static str> {
    match colo.to_uppercase().as_str() {
        "HKG" => Some("香港"),
        "NRT" => Some("东京"),
        "KIX" => Some("大阪"),
        "HND" => Some("东京羽田"),
        "SIN" => Some("新加坡"),
        "ICN" => Some("首尔"),
        "TPE" => Some("台北"),
        "LAX" => Some("洛杉矶"),
        "SJC" => Some("圣何塞"),
        "SFO" => Some("旧金山"),
        "SEA" => Some("西雅图"),
        "ORD" => Some("芝加哥"),
        "DFW" => Some("达拉斯"),
        "EWR" | "JFK" => Some("纽约"),
        "IAD" => Some("华盛顿"),
        "MIA" => Some("迈阿密"),
        "ATL" => Some("亚特兰大"),
        "FRA" => Some("法兰克福"),
        "LHR" => Some("伦敦"),
        "CDG" => Some("巴黎"),
        "AMS" => Some("阿姆斯特丹"),
        "SYD" => Some("悉尼"),
        "MEL" => Some("墨尔本"),
        "YYZ" => Some("多伦多"),
        "YVR" => Some("温哥华"),
        "BKK" => Some("曼谷"),
        "KUL" => Some("吉隆坡"),
        "MNL" => Some("马尼拉"),
        "SCL" => Some("圣地亚哥"),
        "GRU" => Some("圣保罗"),
        "EZE" => Some("布宜诺斯艾利斯"),
        "EVN" => Some("埃里温"),
        "DXB" => Some("迪拜"),
        "JNB" => Some("约翰内斯堡"),
        "BOM" => Some("孟买"),
        "DEL" => Some("新德里"),
        _ => None,
    }
}

pub fn urlencode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

pub struct NodeProber;

impl NodeProber {
    /// Directly query the kernel `/proxies/{name}/delay` API.
    /// This gives the exact, lightweight, native latency measured by Mihomo/Clash kernel,
    /// completely bypassing local proxy sockets, protocol sniffing, and HTTP Basic Auth.
    pub async fn probe_kernel_delay(
        api_url: &str,
        secret: Option<&str>,
        target_name: &str,
        timeout_ms: u64,
    ) -> Result<u64, String> {
        let client = match reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(timeout_ms + 1500))
            .build()
        {
            Ok(c) => c,
            Err(e) => return Err(format!("Client error: {}", e)),
        };

        let test_url = "http://cp.cloudflare.com/generate_204";
        let encoded_name = urlencode_component(target_name);
        let url = format!(
            "{}/proxies/{}/delay?timeout={}&url={}",
            api_url.trim_end_matches('/'),
            encoded_name,
            timeout_ms,
            urlencode_component(test_url)
        );

        let mut req = client.get(&url);
        if let Some(s) = secret {
            if !s.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", s));
            }
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return Err(format!("Kernel delay request failed: {}", e)),
        };

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Err(msg);
        }

        let json: serde_json::Value = match serde_json::from_str(&body) {
            Ok(j) => j,
            Err(_) => return Err(format!("Invalid JSON: {}", body)),
        };

        if let Some(delay) = json.get("delay").and_then(|d| d.as_u64()) {
            Ok(delay)
        } else if let Some(msg) = json.get("message").and_then(|m| m.as_str()) {
            Err(msg.to_string())
        } else {
            Err(format!("Unexpected delay payload: {}", body))
        }
    }

    pub async fn probe_node(
        mixed_port: u16,
        username: &str,
        password: &str,
        timeout_secs: u64,
    ) -> NodeProbeResult {
        let mut res = Self::probe_node_inner(mixed_port, username, password, timeout_secs).await;
        res.history.push(ProbeHistoryEntry {
            ok: res.error.is_none(),
            latency_ms: res.latency_ms,
            speed_bps: res.speed_bps,
            checked_at: chrono::Utc::now(),
        });
        res
    }

    async fn probe_node_inner(
        mixed_port: u16,
        username: &str,
        password: &str,
        timeout_secs: u64,
    ) -> NodeProbeResult {
        let proxy_url = format!("http://{}:{}@127.0.0.1:{}", username, password, mixed_port);
        let client = match client_for(&proxy_url, timeout_secs) {
            Ok(c) => c,
            Err(e) => {
                return NodeProbeResult {
                    error: Some(format!("Client build error: {}", e)),
                    checked_at: Some(chrono::Utc::now()),
                    ..Default::default()
                };
            }
        };

        // Phase 1: Clash-aligned Anycast 204 Latency Test
        // Primary: Cloudflare Anycast 204 (fast, 330+ globally distributed edge PoPs, unaffected by Google blocks)
        let mut measured_latency: Option<u64> = None;
        let t0 = std::time::Instant::now();
        if let Ok(resp) = client
            .get("http://cp.cloudflare.com/generate_204")
            .timeout(Duration::from_millis(2500))
            .send()
            .await
        {
            if resp.status().is_success() || resp.status().as_u16() == 204 {
                measured_latency = Some(t0.elapsed().as_millis() as u64);
            }
        }

        // Secondary 204 probe (Google Anycast) if Cloudflare was unreachable
        if measured_latency.is_none() {
            let t_g = std::time::Instant::now();
            if let Ok(resp) = client
                .get("http://www.gstatic.com/generate_204")
                .timeout(Duration::from_millis(2500))
                .send()
                .await
            {
                if resp.status().is_success() || resp.status().as_u16() == 204 {
                    measured_latency = Some(t_g.elapsed().as_millis() as u64);
                }
            }
        }

        // Phase 2: Fast Anycast Outbound IP & Geo Location Lookup
        // Primary: Cloudflare Trace (distributed across 300+ cities, no rate limit)
        let mut outbound_ip: Option<String> = None;
        let mut country_code: Option<String> = None;
        let mut location: Option<String> = None;

        let cf_trace_start = std::time::Instant::now();
        if let Ok(resp) = client
            .get("http://cloudflare.com/cdn-cgi/trace")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
            .timeout(Duration::from_millis(3500))
            .send()
            .await
        {
            if resp.status().is_success() {
                if measured_latency.is_none() {
                    measured_latency = Some(cf_trace_start.elapsed().as_millis() as u64);
                }
                if let Ok(body) = resp.text().await {
                    let mut colo: Option<String> = None;
                    for line in body.lines() {
                        if let Some(ip) = line.strip_prefix("ip=") {
                            outbound_ip = Some(ip.trim().to_string());
                        } else if let Some(loc) = line.strip_prefix("loc=") {
                            country_code = Some(loc.trim().to_uppercase());
                        } else if let Some(c) = line.strip_prefix("colo=") {
                            colo = Some(c.trim().to_uppercase());
                        }
                    }

                    if let Some(ref cc) = country_code {
                        let cname = crate::collector::country::country_name(cc);
                        location = match colo {
                            Some(ref cl) => match colo_to_city(cl) {
                                Some(city) => Some(format!("{}, {}", city, cname)),
                                None => Some(format!("{}, {}", cl, cname)),
                            },
                            None => Some(cname.to_string()),
                        };
                    }
                }
            }
        }

        // Fallback GeoIP (ip-api.com) if Cloudflare trace failed or didn't return IP
        if outbound_ip.is_none() {
            let ip_api_start = std::time::Instant::now();
            let target_url = "http://ip-api.com/json/?fields=status,message,country,countryCode,city,query";
            if let Ok(resp) = client
                .get(target_url)
                .timeout(Duration::from_millis(4000))
                .send()
                .await
            {
                if resp.status().is_success() {
                    if measured_latency.is_none() {
                        measured_latency = Some(ip_api_start.elapsed().as_millis() as u64);
                    }
                    if let Ok(info) = resp.json::<IpInfoResponse>().await {
                        outbound_ip = info.query;
                        country_code = info.country_code;
                        location = match (info.city, info.country) {
                            (Some(c), Some(co)) if !c.is_empty() && !co.is_empty() => Some(format!("{}, {}", c, co)),
                            (_, Some(co)) => Some(co),
                            (Some(c), _) => Some(c),
                            _ => None,
                        };
                    }
                }
            }
        }

        // Phase 3: Lightweight throughput & download speed test (350KB Anycast sample)
        let mut speed_bps: Option<u64> = None;
        let mut speed_formatted: Option<String> = None;

        if measured_latency.is_some() {
            let speed_start = std::time::Instant::now();
            let speed_url = "http://speed.cloudflare.com/__down?bytes=350000";
            if let Ok(resp) = client
                .get(speed_url)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
                .timeout(Duration::from_millis(2500))
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        let elapsed = speed_start.elapsed().as_secs_f64();
                        if elapsed > 0.04 && !bytes.is_empty() {
                            let bps = (bytes.len() as f64 / elapsed) as u64;
                            speed_bps = Some(bps);
                            speed_formatted = Some(format_speed_bps(bps));
                        }
                    }
                }
            }
        }

        // Final result evaluation
        if outbound_ip.is_none() && measured_latency.is_none() {
            NodeProbeResult {
                latency_ms: None,
                speed_bps: None,
                speed_formatted: None,
                error: Some("Probe timeout / unreachable".to_string()),
                checked_at: Some(chrono::Utc::now()),
                ..Default::default()
            }
        } else if outbound_ip.is_none() {
            NodeProbeResult {
                latency_ms: measured_latency,
                speed_bps,
                speed_formatted,
                outbound_ip: None,
                location: None,
                country_code: None,
                error: Some("IP lookup failed".to_string()),
                checked_at: Some(chrono::Utc::now()),
                history: Vec::new(),
            }
        } else {
            NodeProbeResult {
                latency_ms: measured_latency,
                speed_bps,
                speed_formatted,
                outbound_ip,
                location: location.or_else(|| country_code.clone()),
                country_code,
                error: None,
                checked_at: Some(chrono::Utc::now()),
                history: Vec::new(),
            }
        }
    }
}
