use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Semaphore;
use crate::state::SharedState;
use crate::error::Result;
use crate::controller::prober::NodeProbeResult;

pub fn inherited_strategy_key(username: &str) -> String {
    format!("[inherited]{}", username.to_lowercase())
}

/// In-memory cache for inherited group 'now' selected node
static INHERITED_NOW_CACHE: std::sync::OnceLock<tokio::sync::RwLock<(std::time::Instant, std::collections::HashMap<String, String>)>> = std::sync::OnceLock::new();

async fn fetch_inherited_group_now(controller: &str, secret: Option<&str>) -> std::collections::HashMap<String, String> {
    let cache_lock = INHERITED_NOW_CACHE.get_or_init(|| {
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct InheritedGroupView {
    #[serde(flatten)]
    pub group: crate::generator::inherited_groups::InheritedGroup,
    pub proxy: String,
    pub curl: String,
    pub now: Option<String>,
    pub probe: Option<NodeProbeResult>,
}

pub async fn list_inherited_groups(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let raw_groups = state.list_inherited_groups().await;
    let probes = state.probe_results.read().await;

    let meow_api = &state.config.meow.api_url;
    let meow_secret = state.config.meow.api_secret.as_deref();
    let now_map = fetch_inherited_group_now(meow_api, meow_secret).await;

    let port = state.config.meow.gateway_port;
    let bind = &state.config.meow.gateway_bind;
    let host = if bind == "0.0.0.0" { "127.0.0.1" } else { bind.as_str() };

    let mut views = Vec::new();
    let mut total_nodes = 0;

    for g in raw_groups {
        total_nodes += g.node_count;
        let proxy = format!("http://{}:{}@{}:{}", g.username, g.username, host, port);
        let curl = format!("curl -x {} https://api.ipify.org", proxy);

        let now = now_map.get(&g.name).cloned().or_else(|| g.nodes.first().cloned());
        let p_key = inherited_strategy_key(&g.username);
        let probe = probes.get(&p_key).cloned();

        views.push(InheritedGroupView {
            group: g,
            proxy,
            curl,
            now,
            probe,
        });
    }

    Ok(Json(json!({
        "success": true,
        "count": views.len(),
        "grouped_nodes": total_nodes,
        "data": views
    })))
}

pub async fn probe_single_inherited_group(
    Path(username): Path<String>,
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_inherited_groups().await;
    let group = match groups.into_iter().find(|g| g.username == username || g.aliases.contains(&username)) {
        Some(g) => g,
        None => return Err(crate::error::AppError::Config(format!("Inherited group '{}' not found", username))),
    };

    let port = state.config.meow.mixed_port;
    let res = crate::controller::prober::NodeProber::probe_node(port, &group.username, &group.username, 8).await;

    let key = inherited_strategy_key(&group.username);
    state.update_probe_result(key.clone(), res.clone()).await;
    let merged = state.probe_results.read().await.get(&key).cloned().unwrap_or(res);

    Ok(Json(json!({
        "success": true,
        "data": merged
    })))
}

pub async fn probe_all_inherited_groups(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let groups = state.list_inherited_groups().await;
    let total = groups.len();
    let port = state.config.meow.mixed_port;

    tokio::spawn(async move {
        let sem = Arc::new(Semaphore::new(4));
        let mut set = tokio::task::JoinSet::new();

        for g in groups {
            let sem_c = sem.clone();
            let st = state.clone();
            let key = inherited_strategy_key(&g.username);
            let uname = g.username.clone();

            set.spawn(async move {
                let _permit = sem_c.acquire().await.unwrap();
                let res = crate::controller::prober::NodeProber::probe_node(port, &uname, &uname, 8).await;
                st.update_probe_result(key, res).await;
            });
        }
        while set.join_next().await.is_some() {}
    });

    Ok(Json(json!({
        "success": true,
        "message": format!("Started background probing for {} inherited groups", total)
    })))
}
