use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::error::Result;

/// In-memory / persisted key for a region group's synthetic probe results.
#[allow(dead_code)]
pub fn region_probe_key(code: &str) -> String {
    format!("[region]{}", code)
}

/// Persisted probe key for a region strategy entry.
/// auto → `[region]CODE`, fallback → `[region]CODE-fb`.
pub fn region_strategy_key(code: &str, strategy: &str) -> String {
    let c = code.to_uppercase();
    if strategy == "fallback" {
        format!("[region]{}-fb", c)
    } else {
        format!("[region]{}", c)
    }
}

/// GET /api/regions
/// Returns country/region groups aggregated from all nodes + probe data.
/// Each region exposes two synthetic auth users:
///   - "<code>"      e.g. us   -> url-test (auto fastest) group
///   - "<code>-fb"   e.g. us-fb -> fallback (failover) group
pub async fn list_regions(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_regions().await;
    // Clients connect through the single public gateway port; probing uses the
    // internal loopback inbound.
    let port = state.config.meow.gateway_port;
    let bind = state.config.meow.gateway_bind.clone();
    let host = if bind == "0.0.0.0" || bind.is_empty() { "127.0.0.1".to_string() } else { bind };

    // Query the running kernel for each region group's currently-selected node
    let controller = state.config.meow.api_url.clone();
    let secret = state.config.meow.api_secret.clone();
    let now_map = fetch_group_now(&controller, secret.as_deref()).await;

    let total_nodes: usize = groups.iter().map(|g| g.node_count).sum();

    let probes = state.probe_results.read().await;
    let read_probe = |code: &str, strategy: &str| {
        let k1 = region_strategy_key(code, strategy);
        let k2 = if strategy == "fallback" {
            format!("region-{}-fb", code.to_lowercase())
        } else {
            format!("region-{}", code.to_lowercase())
        };
        probes.get(&k1).or_else(|| probes.get(&k2)).cloned().unwrap_or_default()
    };

    let data: Vec<Value> = groups
        .iter()
        .map(|g| {
            json!({
                "code": g.code,
                "flag": g.flag,
                "node_count": g.node_count,
                "healthy_count": g.healthy_count,
                "avg_latency_ms": g.avg_latency_ms,
                "nodes": g.nodes,
                "probe": read_probe(&g.code, "auto"),
                "probe_fb": read_probe(&g.code, "fallback"),
                "users": g.users.iter().map(|u| json!({
                    "username": u.username,
                    "strategy": u.strategy,
                    "group_name": u.group_name,
                    "now": now_map.get(&u.group_name).cloned().or_else(|| g.nodes.first().cloned()),
                    "proxy": format!("http://{}:{}@{}:{}", u.username, u.username, host, port),
                    "curl": format!("curl -x http://{}:{}@{}:{} https://api.ipify.org", u.username, u.username, host, port),
                })).collect::<Vec<_>>()
            })
        })
        .collect();

    Ok(Json(json!({
        "success": true,
        "count": data.len(),
        "grouped_nodes": total_nodes,
        "data": data
    })))
}

use std::time::{Duration, Instant};
use tokio::sync::RwLock;

static REGION_NOW_CACHE: RwLock<Option<(Instant, std::collections::HashMap<String, String>)>> = RwLock::const_new(None);

/// Fetch `now` (selected proxy) for every proxy-group from the kernel REST API.
async fn fetch_group_now(controller: &str, secret: Option<&str>) -> std::collections::HashMap<String, String> {
    if let Some((at, ref map)) = *REGION_NOW_CACHE.read().await {
        if at.elapsed() < Duration::from_secs(15) {
            return map.clone();
        }
    }

    let mut out = std::collections::HashMap::new();
    let client = match reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(3)).build() {
        Ok(c) => c,
        Err(_) => return out,
    };
    let url = format!("{}/proxies", controller.trim_end_matches('/'));
    let mut req = client.get(&url);
    if let Some(s) = secret {
        if !s.is_empty() {
            req = req.bearer_auth(s);
        }
    }
    if let Ok(res) = req.send().await {
        if let Ok(json) = res.json::<Value>().await {
            if let Some(obj) = json.get("proxies").and_then(|p| p.as_object()) {
                for (name, info) in obj {
                    if let Some(now) = info.get("now").and_then(|n| n.as_str()) {
                        out.insert(name.clone(), now.to_string());
                    }
                }
            }
        }
    }

    *REGION_NOW_CACHE.write().await = Some((Instant::now(), out.clone()));
    out
}

/// POST /api/regions/{code}/probe?strategy=auto|fallback
/// Probes the region group's entry (through the local kernel using the
/// synthetic region credentials) and records the result under `[region]CODE`.
pub async fn probe_region(
    State(state): State<SharedState>,
    Path(code): Path<String>,
    uri: axum::http::Uri,
) -> Result<Json<Value>> {
    let code = code.to_uppercase();
    let strategy = uri
        .query()
        .and_then(|q| {
            q.split('&').find_map(|kv| kv.strip_prefix("strategy="))
        })
        .unwrap_or("auto");

    let groups = state.list_regions().await;
    let group = groups.iter().find(|g| g.code == code).cloned();
    let group = match group {
        Some(g) => g,
        None => return Err(crate::error::AppError::NodeNotFound(format!("region {}", code))),
    };
    let user = group
        .users
        .iter()
        .find(|u| u.strategy == strategy)
        .cloned()
        .or_else(|| group.users.first().cloned())
        .unwrap();

    let key = region_strategy_key(&code, strategy);
    let prev = state.probe_results.read().await.get(&key).cloned().or_else(|| {
        state.probe_results.try_read().ok().and_then(|p| p.get(&user.group_name).cloned())
    });

    if group.nodes.is_empty() {
        let mut empty_res = prev.unwrap_or_default();
        empty_res.error = Some("该地区分组下暂无可探测的节点".to_string());
        return Ok(Json(json!({
            "success": true,
            "message": "该地区分组下暂无可探测的节点",
            "data": empty_res,
            "username": user.username
        })));
    }

    // Directly query kernel native delay for this region group
    let meow = &state.config.meow;
    let kernel_res = crate::controller::prober::NodeProber::probe_kernel_delay(
        &meow.api_url,
        meow.api_secret.as_deref(),
        &user.group_name,
        3000,
    )
    .await;

    let res = match kernel_res {
        Ok(delay) => {
            let mut r = prev.unwrap_or_default();
            r.latency_ms = Some(delay);
            r.error = None;
            r.checked_at = Some(chrono::Utc::now());
            if r.outbound_ip.is_none() {
                if let Some(first_node) = group.nodes.first() {
                    let probes = state.probe_results.read().await;
                    if let Some(np) = probes.get(first_node) {
                        r.outbound_ip = np.outbound_ip.clone();
                        r.location = np.location.clone();
                        r.country_code = np.country_code.clone();
                    }
                }
            }
            r.history.push(crate::controller::prober::ProbeHistoryEntry {
                ok: true,
                latency_ms: Some(delay), speed_bps: None, checked_at: chrono::Utc::now(),
            });
            if r.history.len() > crate::controller::prober::MAX_HISTORY {
                r.history.remove(0);
            }
            r
        }
        Err(e) => {
            let mut r = prev.unwrap_or_default();
            r.latency_ms = None;
            r.error = Some(e);
            r.checked_at = Some(chrono::Utc::now());
            r.history.push(crate::controller::prober::ProbeHistoryEntry {
                ok: false,
                latency_ms: None, speed_bps: None, checked_at: chrono::Utc::now(),
            });
            if r.history.len() > crate::controller::prober::MAX_HISTORY {
                r.history.remove(0);
            }
            r
        }
    };

    state.update_probe_result(key.clone(), res.clone()).await;
    let merged = state.probe_results.read().await.get(&key).cloned().unwrap_or(res);

    Ok(Json(json!({ "success": true, "data": merged, "username": user.username })))
}

/// POST /api/regions/probe — probe every region's auto + fallback entries in background.
pub async fn probe_all_regions(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_regions().await;
    let total = groups.len();

    tokio::spawn(async move {
        use tokio::sync::Semaphore;
        use std::sync::Arc;
        let semaphore = Arc::new(Semaphore::new(16));
        let mut set = tokio::task::JoinSet::new();
        for g in groups {
            if g.nodes.is_empty() {
                continue;
            }
            let code = g.code.clone();
            let first_node = g.nodes.first().cloned();
            for u in &g.users {
                let sem = semaphore.clone();
                let st = state.clone();
                let key = region_strategy_key(&code, &u.strategy);
                let group_name = u.group_name.clone();
                let fn_clone = first_node.clone();
                set.spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let meow = &st.config.meow;
                    let prev = st.probe_results.read().await.get(&key).cloned();
                    let kernel_res = crate::controller::prober::NodeProber::probe_kernel_delay(
                        &meow.api_url,
                        meow.api_secret.as_deref(),
                        &group_name,
                        3000,
                    )
                    .await;

                    let res = match kernel_res {
                        Ok(delay) => {
                            let mut r = prev.unwrap_or_default();
                            r.latency_ms = Some(delay);
                            r.error = None;
                            r.checked_at = Some(chrono::Utc::now());
                            if r.outbound_ip.is_none() {
                                if let Some(ref fn_node) = fn_clone {
                                    let probes = st.probe_results.read().await;
                                    if let Some(np) = probes.get(fn_node) {
                                        r.outbound_ip = np.outbound_ip.clone();
                                        r.location = np.location.clone();
                                        r.country_code = np.country_code.clone();
                                    }
                                }
                            }
                            r.history.push(crate::controller::prober::ProbeHistoryEntry {
                                ok: true,
                                latency_ms: Some(delay), speed_bps: None, checked_at: chrono::Utc::now(),
                            });
                            if r.history.len() > crate::controller::prober::MAX_HISTORY {
                                r.history.remove(0);
                            }
                            r
                        }
                        Err(e) => {
                            let mut r = prev.unwrap_or_default();
                            r.latency_ms = None;
                            r.error = Some(e);
                            r.checked_at = Some(chrono::Utc::now());
                            r.history.push(crate::controller::prober::ProbeHistoryEntry {
                                ok: false,
                                latency_ms: None, speed_bps: None, checked_at: chrono::Utc::now(),
                            });
                            if r.history.len() > crate::controller::prober::MAX_HISTORY {
                                r.history.remove(0);
                            }
                            r
                        }
                    };
                    st.update_probe_result(key, res).await;
                });
            }
        }
        while set.join_next().await.is_some() {}
        tracing::info!("Finished probing {} region entries via kernel API", total);
    });

    Ok(Json(json!({ "success": true, "message": format!("Started probing {} regions in background", total), "total": total })))
}
