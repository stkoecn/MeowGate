mod api;
mod collector;
mod config;
mod controller;
mod error;
mod generator;
mod state;
mod store;
mod web;

use axum::{
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::AppConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Ensure MeowGate and any spawned threads/subprocesses never route through a local system/global proxy (e.g. Clash Verge at 7890)
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("http_proxy");
    std::env::remove_var("HTTPS_PROXY");
    std::env::remove_var("https_proxy");
    std::env::remove_var("ALL_PROXY");
    std::env::remove_var("all_proxy");

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "meowgate=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting MeowGate service...");

    let mut config = AppConfig::load_or_default(None);
    let auto_refresh_secs = config.auto_refresh_secs;

    // The single public gateway port carries the Web UI *and* both proxy
    // protocols. The kernel's own inbound stays on loopback, so it is invisible
    // to clients. If the kernel is already running (e.g. verge-mihomo), use its
    // configured port directly; if not and the port is busy, auto-migrate.
    let configured_mixed = config.meow.mixed_port;
    let mihomo_probe_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
        .unwrap_or_default();
    let mihomo_active = mihomo_probe_client
        .get(format!("{}/version", config.meow.api_url))
        .send()
        .await
        .is_ok();

    if !mihomo_active {
        let resolved_mixed = controller::ports::resolve(configured_mixed);
        if resolved_mixed != configured_mixed {
            tracing::warn!(
                "Internal kernel port 127.0.0.1:{} is busy — using 127.0.0.1:{} instead",
                configured_mixed,
                resolved_mixed
            );
            config.meow.mixed_port = resolved_mixed;
        }
    }

    // The single public gateway port (Web UI + HTTP/SOCKS5 proxy).
    // `server.bind_addr` remains the Web UI bind for compatibility, but the
    // gateway port is what clients should use.
    let gateway_addr = config.meow.gateway_addr();
    let gateway_mixed_port = config.meow.mixed_port;

    let state = AppState::new(config).await?;

    // Pre-fill the /api/status cache in the background: its first build costs
    // ~2s because the Clash API probe times out when Clash is not running.
    {
        let warm_state = state.clone();
        tokio::spawn(async move {
            api::status::warm(&warm_state).await;
        });
    }

    // Auto refresh background task if configured
    if auto_refresh_secs > 0 {
        let task_state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(auto_refresh_secs));
            loop {
                interval.tick().await;
                tracing::info!("Auto-refresh tick: syncing nodes and applying configuration...");
                if let Err(e) = task_state.sync_nodes_and_users(false).await {
                    tracing::error!("Auto-refresh sync error: {}", e);
                } else if let Err(e) = task_state.apply_and_reload().await {
                    tracing::error!("Auto-refresh reload error: {}", e);
                }
            }
        });
    }

    // Node health monitor: periodic sweeps + automatic config regeneration
    {
        let monitor_state = state.clone();
        tokio::spawn(async move {
            crate::controller::health_monitor::monitor_loop(monitor_state).await;
        });
    }

    // Historical connection and real-time traffic tracker
    {
        let tracker_state = state.clone();
        tokio::spawn(async move {
            tracker_state.run_connection_tracker().await;
        });
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_router = Router::new()
        .route("/health", get(api::status::health))
        .route("/status", get(api::status::status))
        .route("/sources", get(api::sources::list_sources).post(api::sources::create_source))
        .route("/sources/scan", post(api::sources::scan_sources))
        .route("/sources/{id}/toggle", post(api::sources::toggle_source).put(api::sources::toggle_source))
        .route("/sources/{id}", axum::routing::delete(api::sources::delete_source).put(api::sources::update_source))
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
        .route("/system-proxy/toggle", post(api::sysproxy::toggle_proxy));

    let app = Router::new()
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
        .fallback(web::static_handler)
        .layer(tower_http::compression::CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    // Serve the Web app through the mux so proxy traffic and HTTP API traffic
    // share one externally reachable port.
    let listener =
        crate::controller::mux::GatewayMux::bind(&gateway_addr, gateway_mixed_port).await?;
    tracing::info!(
        "MeowGate ready — open http://{} for the console, or use it as an HTTP/SOCKS5 proxy with <user>:<user> credentials",
        gateway_addr
    );

    axum::serve(listener, app).await?;

    Ok(())
}
