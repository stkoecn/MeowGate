use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::Response,
    routing::{delete, get, post},
    Router,
};
use rust_embed::Embed;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use crate::api;
use crate::state::{AppState, SharedState};

#[derive(Embed)]
#[folder = "static/"]
pub struct Asset;

pub fn build_app(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers(Any);

    let api_router = Router::new()
        .route("/health", get(api::status::health))
        .route("/status", get(api::status::status))
        .route("/sources", get(api::sources::list_sources).post(api::sources::create_source))
        .route("/sources/scan", post(api::sources::scan_sources))
        .route("/sources/{id}/toggle", post(api::sources::toggle_source).put(api::sources::toggle_source).patch(api::sources::toggle_source))
        .route("/sources/{id}/toggle-base", post(api::sources::toggle_source_base).put(api::sources::toggle_source_base).patch(api::sources::toggle_source_base))
        .route("/sources/{id}", delete(api::sources::delete_source).put(api::sources::update_source).patch(api::sources::update_source))
        .route("/regions", get(api::regions::list_regions))
        .route("/regions/probe", post(api::regions::probe_all_regions))
        .route("/regions/{code}/probe", post(api::regions::probe_region))
        .route("/ip-groups", get(api::ip_groups::list_ip_groups))
        .route("/ip-groups/probe", post(api::ip_groups::probe_all_ip_groups))
        .route("/ip-groups/{ip}/probe", post(api::ip_groups::probe_ip_group))
        .route("/level-groups", get(api::level_groups::list_level_groups))
        .route("/level-groups/probe", post(api::level_groups::probe_all_level_groups))
        .route("/level-groups/{code}/probe", post(api::level_groups::probe_single_level_group))
        .route("/speed-groups", get(api::speed_groups::list_speed_groups))
        .route("/speed-groups/probe", post(api::speed_groups::probe_all_speed_groups))
        .route("/speed-groups/{code}/probe", post(api::speed_groups::probe_single_speed_group))
        .route("/inherited-groups", get(api::inherited_groups::list_inherited_groups))
        .route("/inherited-groups/probe", post(api::inherited_groups::probe_all_inherited_groups))
        .route("/inherited-groups/probe-all", post(api::inherited_groups::probe_all_inherited_groups))
        .route("/inherited-groups/{username}/probe", post(api::inherited_groups::probe_single_inherited_group))
        .route("/nodes", get(api::nodes::list_nodes))
        .route("/nodes/refresh", post(api::nodes::refresh_nodes))
        .route("/nodes/import", post(api::nodes::import_yaml))
        .route("/nodes/probe", post(api::nodes::probe_all_nodes))
        .route("/nodes/{node}/probe", post(api::nodes::probe_single_node))
        .route("/users", get(api::users::list_users))
        .route("/users/refresh", post(api::users::refresh_all_users))
        .route("/users/{node}/reset", post(api::users::reset_user))
        .route("/health/nodes", get(api::health::list_health))
        .route("/health/summary", get(api::health::summary))
        .route("/health/check", post(api::health::trigger_check))
        .route("/health/policy", post(api::health::update_policy))
        .route("/health/nodes/{node}/check", post(api::health::check_node))
        .route("/health/nodes/{node}/toggle", post(api::health::toggle_node))
        .route("/config/preview", get(api::config::preview_config))
        .route("/config/reload", post(api::config::reload_config))
        .route("/sub", get(api::subscription::clash_subscription_handler))
        .route("/system-proxy", get(api::sysproxy::get_proxy_status))
        .route("/system-proxy/toggle", post(api::sysproxy::toggle_proxy))
        .route("/auth/status", get(api::auth::auth_status))
        .route("/auth/login", post(api::auth::login))
        .route("/auth/logout", post(api::auth::logout))
        .layer(axum::middleware::from_fn_with_state(state.clone(), api::auth::auth_middleware));

    Router::new()
        .nest("/api", api_router)
        // Standalone Clash subscription links (Clash Verge / Stash / Shadowrocket / Loon / Quantumult X)
        .route("/sub", get(api::subscription::clash_subscription_handler))
        .route("/clash", get(api::subscription::clash_subscription_handler))
        // Clash / Mihomo native REST API reverse proxy (ZashBoard / Yacd / Metacubexd compatibility)
        .route("/version", get(api::clash_proxy::clash_proxy_handler))
        .route("/configs", get(api::clash_proxy::clash_proxy_handler).put(api::clash_proxy::clash_proxy_handler).patch(api::clash_proxy::clash_proxy_handler))
        .route("/proxies", get(api::clash_proxy::clash_proxy_handler))
        .route("/proxies/{*path}", get(api::clash_proxy::clash_proxy_handler).put(api::clash_proxy::clash_proxy_handler))
        .route("/rules", get(api::clash_proxy::clash_proxy_handler))
        .route("/connections", get(api::clash_proxy::clash_proxy_handler).delete(api::clash_proxy::clash_proxy_handler))
        .route("/connections/{*path}", delete(api::clash_proxy::clash_proxy_handler))
        .route("/traffic", get(api::clash_proxy::clash_proxy_stream_handler))
        .route("/logs", get(api::clash_proxy::clash_proxy_stream_handler))
        .route("/providers/proxies", get(api::clash_proxy::clash_proxy_handler))
        .route("/providers/proxies/{*path}", get(api::clash_proxy::clash_proxy_handler).put(api::clash_proxy::clash_proxy_handler))
        .route("/dns/query", get(api::clash_proxy::clash_proxy_handler))
        .fallback(static_handler)
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state)
}

pub async fn static_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();

    // 1. Handle root / or index.html navigation
    if path.is_empty() || path == "index.html" {
        if state.is_auth_enabled() {
            let query_token = uri.query().and_then(|q| {
                q.split('&').find_map(|pair| pair.strip_prefix("token="))
            });
            let authenticated = crate::api::auth::extract_and_verify(&state, &headers, query_token);
            if !authenticated {
                return Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "/login")
                    .body(Body::empty())
                    .unwrap();
            }
        }
        path = "index.html".to_string();
    }

    // 2. Handle /login navigation
    if path == "login" || path == "login.html" {
        if state.is_auth_enabled() {
            let authenticated = crate::api::auth::extract_and_verify(&state, &headers, None);
            if authenticated {
                return Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "/")
                    .body(Body::empty())
                    .unwrap();
            }
        } else {
            // Auth not enabled, redirect to home
            return Response::builder()
                .status(StatusCode::FOUND)
                .header(header::LOCATION, "/")
                .body(Body::empty())
                .unwrap();
        }
        path = "login.html".to_string();
    }

    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => {
            // Embedded flags (static/flags/*.png) live under /flags/<code>.png.
            // Never fall back to index.html for asset paths: a missing image
            // must 404 so the browser can hide it instead of rendering HTML.
            if path.contains('.') && !path.ends_with(".html") {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))
                    .body(Body::from(""))
                    .unwrap();
            }
            // Fallback to index.html for SPA if not found (with auth check)
            if state.is_auth_enabled() {
                let authenticated = crate::api::auth::extract_and_verify(&state, &headers, None);
                if !authenticated {
                    return Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, "/login")
                        .body(Body::empty())
                        .unwrap();
                }
            }
            if let Some(content) = Asset::get("index.html") {
                Response::builder()
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))
                    .body(Body::from(content.data))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("404 Not Found"))
                    .unwrap()
            }
        }
    }
}
