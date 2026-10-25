use axum::{
    extract::{Path, State},
    Json,
};
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::error::Result;

pub async fn list_users(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let users = state.users.read().await;
    Ok(Json(json!({
        "success": true,
        "count": users.len(),
        "data": *users
    })))
}

pub async fn refresh_all_users(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let count = state.sync_nodes_and_users(true).await?;
    Ok(Json(json!({
        "success": true,
        "message": format!("Successfully refreshed credentials for {} nodes", count),
        "count": count
    })))
}

pub async fn reset_user(
    Path(node): Path<String>,
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let updated = state.reset_node_password(&node).await?;
    Ok(Json(json!({
        "success": true,
        "message": format!("Successfully reset password for node '{}'", node),
        "data": updated
    })))
}
