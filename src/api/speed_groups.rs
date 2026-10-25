use axum::extract::{Path, State, Query};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::state::SharedState;
use crate::error::Result;

#[derive(Debug, Deserialize)]
pub struct ProbeQuery {
    #[serde(default = "default_strategy")]
    pub strategy: String,
}

fn default_strategy() -> String {
    "auto".to_string()
}

pub fn speed_strategy_key(code: &str, strategy: &str) -> String {
    if strategy == "fallback" || strategy == "fb" {
        format!("[speed]{}-fb", code.to_lowercase())
    } else {
        format!("[speed]{}", code.to_lowercase())
    }
}

/// In-memory cache for speed group 'now' selected node
static SPEED_NOW_CACHE: std::sync::OnceLock<tokio::sync::RwLock<(std::time::Instant, std::collections::HashMap<String, String>)>> = std::sync::OnceLock::new();

async fn fetch_speed_group_now(controller: &str, secret: Option<&str>) -> std::collections::HashMap<String, String> {
    let cache_lock = SPEED_NOW_CACHE.get_or_init(|| {
        tokio::sync::RwLock::new((std::time::Instant::now() - std::time::Duration::from_secs(60), std::collections::HashMap::new()))
    });

    {
        let guard = cache_lock.read().await;
        if guard.0.elapsed() < std::time::Duration::from_secs(15) && !guard.1.is_empty() {
            return guard.1.clone();
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
        if let Ok(body) = res.json::<serde_json::Value>().await {
            if let Some(proxies) = body.get("proxies").and_then(|p| p.as_object()) {
                for (name, val) in proxies {
                    if let Some(now) = val.get("now").and_then(|n| n.as_str()) {
                        out.insert(name.clone(), now.to_string());
                    }
                }
            }
        }
    }

    let mut guard = cache_lock.write().await;
    *guard = (std::time::Instant::now(), out.clone());
    out
}

/// GET /api/speed-groups
pub async fn list_speed_groups(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_speed_groups().await;

    let port = state.config.meow.gateway_port;
    let bind = state.config.meow.gateway_bind.clone();
    let host = if bind == "0.0.0.0" || bind.is_empty() {
        "127.0.0.1".to_string()
    } else {
        bind
    };

    let controller = state.config.meow.api_url.clone();
    let secret = state.config.meow.api_secret.clone();
    let now_map = fetch_speed_group_now(&controller, secret.as_deref()).await;

    let total_nodes: usize = groups.iter().map(|g| g.node_count).sum();

    let probes = state.probe_results.read().await;
    let read_probe = |code: &str, strategy: &str| {
        let k1 = speed_strategy_key(code, strategy);
        let k2 = if strategy == "fallback" || strategy == "fb" {
            format!("speed-{}-fb", code.to_lowercase())
        } else {
            format!("speed-{}", code.to_lowercase())
        };
        probes.get(&k1).or_else(|| probes.get(&k2)).cloned().unwrap_or_default()
    };

    let data: Vec<Value> = groups
        .iter()
        .map(|g| {
            json!({
                "tier": g.tier,
                "code": g.code,
                "name": g.name,
                "desc": g.desc,
                "tag": g.tag,
                "color": g.color,
                "min_speed_bps": g.min_speed_bps,
                "max_speed_bps": g.max_speed_bps,
                "node_count": g.node_count,
                "healthy_count": g.healthy_count,
                "avg_speed_bps": g.avg_speed_bps,
                "avg_speed_formatted": g.avg_speed_formatted,
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

/// POST /api/speed-groups/:code/probe?strategy=auto|fallback
pub async fn probe_single_speed_group(
    Path(code): Path<String>,
    Query(query): Query<ProbeQuery>,
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let strategy = match query.strategy.as_str() {
        "fallback" | "fb" => "fallback",
        _ => "auto",
    };

    let groups = state.list_speed_groups().await;
    let group = groups
        .into_iter()
        .find(|g| g.code.eq_ignore_ascii_case(&code))
        .ok_or_else(|| crate::error::AppError::NodeNotFound(format!("Speed group not found: {}", code)))?;

    let user = group
        .users
        .iter()
        .find(|u| u.strategy == strategy)
        .cloned()
        .or_else(|| group.users.first().cloned())
        .unwrap();

    let key = speed_strategy_key(&code, strategy);
    let prev = state.probe_results.read().await.get(&key).cloned().or_else(|| {
        state.probe_results.try_read().ok().and_then(|p| p.get(&user.group_name).cloned())
    });

    if group.nodes.is_empty() {
        let mut empty_res = prev.unwrap_or_default();
        empty_res.error = Some("该速度分组下暂无可探测的节点".to_string());
        return Ok(Json(json!({
            "success": true,
            "message": "该速度分组下暂无可探测的节点",
            "data": empty_res
        })));
    }

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
            r.error = Some(e.clone());
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

    Ok(Json(json!({
        "success": true,
        "data": merged
    })))
}

/// POST /api/speed-groups/probe
pub async fn probe_all_speed_groups(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_speed_groups().await;
    let total = groups.len() * 2;

    tokio::spawn(async move {
        let semaphore = Arc::new(Semaphore::new(10));
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
                let key = speed_strategy_key(&code, &u.strategy);
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
    });

    Ok(Json(json!({
        "success": true,
        "message": format!("Started probing all {} speed group strategies in background", total),
        "total": total
    })))
}
