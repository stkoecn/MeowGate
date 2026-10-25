use axum::{extract::State, Json};
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::error::Result;

pub async fn preview_config(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let yaml = state.generate_config().await?;
    Ok(Json(json!({
        "success": true,
        "config": yaml
    })))
}

pub async fn reload_config(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let (saved_path, reloaded) = state.apply_and_reload().await?;
    if reloaded {
        Ok(Json(json!({
            "success": true,
            "reloaded": true,
            "path": saved_path,
            "message": "配置已成功写入并完成内核热重载"
        })))
    } else {
        Ok(Json(json!({
            "success": true,
            "reloaded": false,
            "path": saved_path,
            "message": format!("配置已成功写入到 {}（meow-rs 内核当前未启动或未监听 :9091，启动后将直接生效）", saved_path)
        })))
    }
}
