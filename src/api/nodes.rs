use crate::collector::yaml_parser::YamlParser;
use crate::controller::prober::{NodeProbeResult, NodeProber};
use axum::{extract::State, Json};
use serde::Serialize;
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::error::Result;
use crate::collector::Node;

#[derive(Serialize)]
pub struct NodeWithUser {
    #[serde(flatten)]
    pub node: Node,
    pub username: String,
    pub password: String,
    pub probe: Option<NodeProbeResult>,
    pub traffic: Option<crate::controller::traffic_tracker::NodeTrafficStats>,
}

pub async fn list_nodes(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let nodes = state.nodes.read().await;
    let users = state.users.read().await;
    let probes = state.probe_results.read().await;
    let traffic_stats = state.traffic_stats.read().await;

    let user_map: std::collections::HashMap<_, _> = users
        .iter()
        .map(|u| (u.node_name.as_str(), u))
        .collect();

    let list: Vec<NodeWithUser> = nodes
        .iter()
        .map(|n| {
            let user = user_map.get(n.name.as_str());
            let probe = probes.get(&n.name).cloned();
            let traffic = traffic_stats.get(&n.name).cloned();
            NodeWithUser {
                node: n.clone(),
                username: user.map(|u| u.username.clone()).unwrap_or_default(),
                password: user.map(|u| u.password.clone()).unwrap_or_default(),
                probe,
                traffic,
            }
        })
        .collect();

    Ok(Json(json!({
        "success": true,
        "count": list.len(),
        "probing": state.is_probing.load(std::sync::atomic::Ordering::Relaxed),
        "data": list
    })))
}

pub async fn refresh_nodes(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let count = state.sync_nodes_and_users(false).await?;
    Ok(Json(json!({
        "success": true,
        "message": format!("Successfully refreshed {} nodes", count),
        "count": count
    })))
}

pub async fn import_yaml(
    State(state): State<SharedState>,
    body: String,
) -> Result<Json<Value>> {
    let parsed_nodes = YamlParser::parse_str(&body)?;
    let count = parsed_nodes.len();

    // Import into state and DB
    let mut current_nodes = state.nodes.write().await;
    let mut current_users = state.users.write().await;
    let mut node_names: std::collections::HashSet<String> = current_nodes.iter().map(|n| n.name.clone()).collect();
    let mut node_keys: std::collections::HashSet<String> = current_nodes.iter().map(|n| n.dedup_key()).collect();

    let mut imported = 0;
    let mut skipped_dup_addr = 0;
    for node in parsed_nodes {
        let key = node.dedup_key();
        if node_names.contains(&node.name) {
            continue;
        }
        // same server:port+type already exists -> treat as duplicate
        if !node.server.is_empty() && node_keys.contains(&key) {
            skipped_dup_addr += 1;
            continue;
        }
        node_names.insert(node.name.clone());
        node_keys.insert(key);
        {
            let user = crate::generator::auth::AuthGenerator::create_user_for_node(&node.name);
            let stored = crate::store::db::StoredUser {
                node_name: user.node_name.clone(),
                username: user.username.clone(),
                password: user.password.clone(),
                created_at: user.created_at,
            };
            let _ = state.db.save_user(&stored).await;
            current_users.push(user);
            current_nodes.push(node);
            imported += 1;
        }
    }

    Ok(Json(json!({
        "success": true,
        "message": format!("Successfully parsed {} nodes, imported {} new nodes, skipped {} duplicate server:port nodes", count, imported, skipped_dup_addr),
        "total_parsed": count,
        "imported": imported,
        "skipped_duplicate_address": skipped_dup_addr
    })))
}

/// Triggers concurrent probe for all active nodes.
/// Requests outbound IP, country/city, and latency through meow-rs mixed port.
pub async fn probe_all_nodes(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    if state.is_probing.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Ok(Json(json!({
            "success": true,
            "message": "Probe already in progress",
            "probing": true
        })));
    }

    let nodes = state.nodes.read().await.clone();
    let users = state.users.read().await.clone();
    let mixed_port = state.config.meow.mixed_port;

    let user_map: std::collections::HashMap<_, _> = users
        .into_iter()
        .map(|u| (u.node_name, (u.username, u.password)))
        .collect();

    let total = nodes.len();
    let state_clone = state.clone();

    // Spawn probe in background with concurrency limit (16 concurrent requests)
    tokio::spawn(async move {
        use tokio::sync::Semaphore;
        use std::sync::Arc;

        let semaphore = Arc::new(Semaphore::new(16));
        let mut set = tokio::task::JoinSet::new();
        let meow = state_clone.config.meow.clone();

        for node in nodes {
            let sem = semaphore.clone();
            let node_name = node.name.clone();
            let st = state_clone.clone();
            let api_url = meow.api_url.clone();
            let secret = meow.api_secret.clone();
            let user_cred = user_map.get(&node.name).cloned();

            set.spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // If user credentials exist, run full probe (latency + outbound IP + download speed!)
                let mut res = if let Some((username, password)) = user_cred {
                    NodeProber::probe_node(mixed_port, &username, &password, 6).await
                } else {
                    let prev = st.probe_results.read().await.get(&node_name).cloned();
                    prev.unwrap_or_default()
                };

                // Also query kernel delay for precise kernel TTFB
                if let Ok(delay) = NodeProber::probe_kernel_delay(
                    &api_url,
                    secret.as_deref(),
                    &node_name,
                    3000,
                ).await {
                    res.latency_ms = Some(delay);
                    if let Some(last) = res.history.last_mut() {
                        last.latency_ms = Some(delay);
                    }
                }

                let check_outcome = crate::controller::health_monitor::CheckOutcome {
                    ok: res.error.is_none() && res.latency_ms.is_some(),
                    latency_ms: res.latency_ms,
                    error: res.error.clone(),
                };
                st.health.record(&node_name, check_outcome).await;
                st.update_probe_result(node_name, res).await;
            });
        }

        while let Some(_) = set.join_next().await {}
        state_clone.persist_health().await;
        let _ = state_clone.apply_and_reload().await;
        state_clone.is_probing.store(false, std::sync::atomic::Ordering::SeqCst);
        tracing::info!("Finished probing all {} nodes (latency, IP, and speed)", total);
    });

    Ok(Json(json!({
        "success": true,
        "message": format!("Started probing {} nodes in background", total),
        "total": total,
        "probing": true
    })))
}

pub async fn probe_single_node(
    axum::extract::Path(node): axum::extract::Path<String>,
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let users = state.users.read().await;
    let user = users.iter().find(|u| u.node_name == node).cloned();
    let mixed_port = state.config.meow.mixed_port;
    let meow = &state.config.meow;

    if let Some(u) = user {
        // Run full probe including latency, IP, and throughput speed test!
        let mut res = NodeProber::probe_node(mixed_port, &u.username, &u.password, 8).await;

        // If kernel delay is available, use it for exact kernel latency alignment
        if let Ok(delay) = NodeProber::probe_kernel_delay(
            &meow.api_url,
            meow.api_secret.as_deref(),
            &node,
            3000,
        ).await {
            res.latency_ms = Some(delay);
            if let Some(last) = res.history.last_mut() {
                last.latency_ms = Some(delay);
            }
        }

        let check_outcome = crate::controller::health_monitor::CheckOutcome {
            ok: res.error.is_none() && res.latency_ms.is_some(),
            latency_ms: res.latency_ms,
            error: res.error.clone(),
        };
        let (h, changed) = state.health.record(&node, check_outcome).await;
        state.persist_health_for(&[node.clone()]).await;
        if changed {
            let _ = state.apply_and_reload().await;
        }

        state.update_probe_result(node.clone(), res.clone()).await;
        let merged = state.probe_results.read().await.get(&node).cloned().unwrap_or(res);
        Ok(Json(json!({
            "success": true,
            "data": merged,
            "health": h
        })))
    } else {
        Err(crate::error::AppError::NodeNotFound(node))
    }
}


