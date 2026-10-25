//! Node lifecycle / health API.

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::HealthPolicy;
use crate::error::Result;
use crate::state::SharedState;

#[derive(Deserialize)]
pub struct ToggleNodeRequest {
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct PolicyRequest {
    pub enabled: Option<bool>,
    pub interval_secs: Option<u64>,
    pub timeout_ms: Option<u64>,
    pub fail_threshold: Option<u32>,
    pub recover_threshold: Option<u32>,
    pub down_retry_secs: Option<u64>,
    pub concurrency: Option<usize>,
    pub test_url: Option<String>,
}

/// GET /api/health/nodes — full node health snapshot + policy + counters.
pub async fn list_health(State(state): State<SharedState>) -> Result<Json<Value>> {
    let mut states = state.health_snapshot().await;
    // Only report nodes that still exist in the pool
    let live: std::collections::HashSet<String> =
        state.nodes.read().await.iter().map(|n| n.name.clone()).collect();
    states.retain(|h| live.contains(&h.node_name));

    let (up, recovering, down, disabled) = state.health.count_by_state().await;
    let policy = state.health.policy().await;

    // Aggregate into the region/node groups for the UI
    let unavailable: Vec<String> = states
        .iter()
        .filter(|h| !h.state.is_available())
        .map(|h| h.node_name.clone())
        .collect();

    let (next_check_at, next_check_in_secs) = state.health.earliest_next_check().await;

    Ok(Json(json!({
        "success": true,
        "sweeping": state.health.is_sweeping(),
        "next_check_at": next_check_at,
        "next_check_in_secs": next_check_in_secs,
        "policy": policy,
        "counts": {
            "total": states.len(),
            "up": up,
            "recovering": recovering,
            "down": down,
            "disabled": disabled,
            "unavailable": unavailable.len(),
        },
        "unavailable": unavailable,
        "data": states,
    })))
}

/// GET /api/health/summary — counters + policy only.
///
/// The dashboard polls health every 30s; shipping all ~1800 per-node records
/// each time (≈650 KB of JSON the page does not look at) was wasteful, so the
/// background poll uses this and the modal fetches the full snapshot instead.
pub async fn summary(State(state): State<SharedState>) -> Result<Json<Value>> {
    let (up, recovering, down, disabled) = state.health.count_by_state().await;
    let policy = state.health.policy().await;
    let total = state.health_snapshot().await.len();
    // node name -> state, only for nodes that are NOT healthy. The client treats
    // any name missing from this map as "up", keeping the payload ~20 KB instead
    // of the ~650 KB full snapshot.
    let states: serde_json::Map<String, Value> = state
        .health_snapshot()
        .await
        .into_iter()
        .filter(|h| !h.state.is_available() || h.state == crate::controller::health_monitor::HealthState::Recovering)
        .map(|h| (h.node_name, serde_json::to_value(h.state).unwrap_or(Value::String("up".into()))))
        .collect();
    let (next_check_at, next_check_in_secs) = state.health.earliest_next_check().await;

    Ok(Json(json!({
        "states": states,
        "success": true,
        "sweeping": state.health.is_sweeping(),
        "next_check_at": next_check_at,
        "next_check_in_secs": next_check_in_secs,
        "policy": policy,
        "counts": {
            "total": total,
            "up": up,
            "recovering": recovering,
            "down": down,
            "disabled": disabled,
            "unavailable": down + disabled,
        },
    })))
}

/// POST /api/health/check — trigger a full sweep right now.
pub async fn trigger_check(State(state): State<SharedState>) -> Result<Json<Value>> {
    let st = state.clone();
    tokio::spawn(async move {
        crate::controller::health_monitor::run_check_cycle(&st).await;
    });
    Ok(Json(json!({
        "success": true,
        "message": "Health check sweep started"
    })))
}

/// POST /api/health/nodes/{node}/check — re-check one node now.
pub async fn check_node(
    State(state): State<SharedState>,
    Path(node): Path<String>,
) -> Result<Json<Value>> {
    match state.recheck_node(&node).await? {
        Some(h) => Ok(Json(json!({ "success": true, "data": h }))),
        None => Err(crate::error::AppError::NodeNotFound(node)),
    }
}

/// POST /api/health/nodes/{node}/toggle — manual take-out / restore.
pub async fn toggle_node(
    State(state): State<SharedState>,
    Path(node): Path<String>,
    Json(req): Json<ToggleNodeRequest>,
) -> Result<Json<Value>> {
    let changed = state.set_node_enabled(&node, req.enabled).await?;
    let h = state.health.get(&node).await;
    Ok(Json(json!({
        "success": true,
        "changed": changed,
        "enabled": req.enabled,
        "data": h,
    })))
}

/// POST /api/health/policy — update the monitoring policy at runtime.
pub async fn update_policy(
    State(state): State<SharedState>,
    Json(req): Json<PolicyRequest>,
) -> Result<Json<Value>> {
    let mut p: HealthPolicy = state.health.policy().await;
    if let Some(v) = req.enabled {
        p.enabled = v;
    }
    if let Some(v) = req.interval_secs {
        p.interval_secs = v.max(5);
    }
    if let Some(v) = req.timeout_ms {
        p.timeout_ms = v.max(500);
    }
    if let Some(v) = req.fail_threshold {
        p.fail_threshold = v.max(1);
    }
    if let Some(v) = req.recover_threshold {
        p.recover_threshold = v.max(1);
    }
    if let Some(v) = req.down_retry_secs {
        p.down_retry_secs = v;
    }
    if let Some(v) = req.concurrency {
        p.concurrency = v.clamp(1, 256);
    }
    if let Some(v) = req.test_url {
        if !v.trim().is_empty() {
            p.test_url = v;
        }
    }
    state.update_health_policy(p.clone()).await;
    Ok(Json(json!({ "success": true, "policy": p })))
}
