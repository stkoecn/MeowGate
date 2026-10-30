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
    #[serde(default, deserialize_with = "de_flexible_bool")]
    pub healthy_only: Option<bool>,
    #[serde(default, deserialize_with = "de_flexible_bool")]
    pub filter_healthy: Option<bool>,
    pub token: Option<String>,
}

fn de_flexible_bool<'de, D>(deserializer: D) -> std::result::Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Visitor;
    struct BoolVisitor;
    impl<'de> Visitor<'de> for BoolVisitor {
        type Value = Option<bool>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a boolean or string representation of a boolean")
        }
        fn visit_bool<E>(self, v: bool) -> std::result::Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_str<E>(self, v: &str) -> std::result::Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            match v.to_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Ok(Some(true)),
                "false" | "0" | "no" | "off" => Ok(Some(false)),
                _ => Ok(None),
            }
        }
        fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
            Ok(None)
        }
    }
    deserializer.deserialize_any(BoolVisitor)
}

/// Handler for Clash subscription URLs (/sub, /clash, /api/sub).
///
/// Returns standalone, ready-to-use Clash/Mihomo client subscription YAML with
/// standard headers (profile-update-interval, Subscription-Userinfo, Profile-Title).
///
/// Query options:
/// - `?mode=direct` / `?mode=clash` (default): standalone client subscription with real node servers/ports.
/// - `?mode=gateway`: client subscription where all proxies route through MeowGate port 9565.
/// - `?filter_healthy=true` / `?filter=healthy` / `?healthy_only=true`: filter proxies to healthy nodes only.
/// - `?mode=raw` / `?mode=kernel`: raw gateway kernel configuration including IN-USER rules.
pub async fn clash_subscription_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<SubQuery>,
) -> impl IntoResponse {
    if state.is_auth_enabled() {
        let authenticated = crate::api::auth::extract_and_verify(&state, &headers, query.token.as_deref());
        if !authenticated {
            return (
                StatusCode::UNAUTHORIZED,
                "Unauthorized: valid subscription token or authentication required\n",
            )
                .into_response();
        }
    }

    let mode = query.mode.as_deref().unwrap_or("clash");

    let healthy_only = query.filter_healthy == Some(true)
        || query.healthy_only == Some(true)
        || query.filter.as_deref() == Some("healthy")
        || query.filter.as_deref() == Some("1")
        || mode == "healthy";

    let gateway_host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(':').next())
        .filter(|h| !h.is_empty())
        .unwrap_or("127.0.0.1");

    let yaml_content = if mode == "gateway" {
        match state
            .generate_clash_subscription_gateway_with_filter(gateway_host, healthy_only)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to generate gateway subscription: {}", e),
                )
                    .into_response()
            }
        }
    } else if mode == "raw" || mode == "kernel" {
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
