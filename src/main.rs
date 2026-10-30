mod api;
mod collector;
mod config;
mod controller;
mod error;
mod generator;
mod state;
mod store;
mod web;

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

    let app = web::build_app(state.clone());

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
