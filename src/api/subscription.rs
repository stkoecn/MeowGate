use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use crate::state::SharedState;
use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct SubQuery {
    pub mode: Option<String>,
    pub filter: Option<String>,
    pub healthy_only: Option<bool>,
}

/// Handler for Clash subscription URLs (/sub, /clash, /api/sub).
///
/// Returns standalone, ready-to-use Clash/Mihomo client subscription YAML with
/// standard headers (profile-update-interval, Subscription-Userinfo, Profile-Title).
///
/// Query options:
/// - `?mode=clash` (default): standalone client subscription with PROXY, AUTO, Level, Region groups and clean rules.
/// - `?filter=healthy` / `?healthy_only=true` / `?mode=healthy`: filter proxies to healthy nodes only.
/// - `?mode=gateway` / `?mode=raw`: raw gateway configuration including IN-USER rules.
pub async fn clash_subscription_handler(
    State(state): State<SharedState>,
    Query(query): Query<SubQuery>,
) -> impl IntoResponse {
    let mode = query.mode.as_deref().unwrap_or("clash");

    let is_gateway = mode == "gateway" || mode == "raw";
    let healthy_only = query.filter.as_deref() == Some("healthy")
        || mode == "healthy"
        || query.healthy_only == Some(true)
        || query.filter.as_deref() == Some("1");

    let yaml_content = if is_gateway {
        match state.generate_config().await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to generate gateway configuration: {}", e),
                )
                    .into_response()
            }
        }
    } else {
        match state.generate_clash_subscription_with_filter(healthy_only).await {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to generate subscription: {}", e),
                )
                    .into_response()
            }
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/yaml; charset=utf-8"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline; filename=\"MeowGate.yaml\""),
    );
    headers.insert("Profile-Update-Interval", HeaderValue::from_static("24"));
    headers.insert("Profile-Title", HeaderValue::from_static("MeowGate"));
    headers.insert(
        "Subscription-Userinfo",
        HeaderValue::from_static("upload=0; download=0; total=1099511627776; expire=2051222400"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );

    (StatusCode::OK, headers, yaml_content).into_response()
}
