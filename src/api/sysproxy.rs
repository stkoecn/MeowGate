use axum::{extract::State, Json};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::state::SharedState;
use crate::error::Result;
use crate::controller::sysproxy::{get_system_proxy_status, set_system_proxy};

#[derive(Deserialize)]
pub struct ToggleProxyRequest {
    pub enabled: Option<bool>,
}

/// GET /api/system-proxy
/// Returns current OS system proxy status (Windows / Linux)
pub async fn get_proxy_status(
    State(state): State<SharedState>,
) -> Result<Json<Value>> {
    let port = state.config.meow.gateway_port;
    let status = get_system_proxy_status(port);
    Ok(Json(json!({
        "success": true,
        "data": status
    })))
}

/// POST /api/system-proxy/toggle
/// Toggles or sets OS system proxy status
pub async fn toggle_proxy(
    State(state): State<SharedState>,
    payload: Option<Json<ToggleProxyRequest>>,
) -> Result<Json<Value>> {
    let port = state.config.meow.gateway_port;
    let current = get_system_proxy_status(port);

    let target_enable = match payload.and_then(|Json(p)| p.enabled) {
        Some(e) => e,
        None => !current.enabled,
    };

    match set_system_proxy(port, target_enable) {
        Ok(new_status) => {
            let msg = if new_status.enabled {
                if new_status.platform == "linux" {
                    format!("系统代理已开启 (127.0.0.1:{})，新开终端将自动生效", port)
                } else {
                    format!("系统代理已开启 (127.0.0.1:{})", port)
                }
            } else if new_status.platform == "linux" {
                "系统代理已关闭（若已有开启的终端窗口，可新开终端或执行 unset http_proxy 立即生效）".to_string()
            } else {
                "系统代理已关闭".to_string()
            };
            Ok(Json(json!({
                "success": true,
                "message": msg,
                "data": new_status
            })))
        }
        Err(e) => {
            Err(crate::error::AppError::Anyhow(anyhow::anyhow!("设置系统代理失败: {}", e)))
        }
    }
}
