use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::error::Result;
use crate::state::SharedState;

pub async fn health() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "meowgate"
    }))
}

/// `/api/status` is polled by the dashboard, but two of its inputs are slow:
/// probing the Clash API (on Windows a refused `127.0.0.1:9090` costs ~2s and
/// Clash is usually not even running) and `discover_all_configs()`, which
/// enumerates every process and walks the profile directories.
///
/// Stale-while-revalidate: a cached payload is served instantly and refreshed in
/// the background once it is older than `STATUS_TTL`, so polling never waits on a
/// probe. Only the very first request pays for one.
const STATUS_TTL: Duration = Duration::from_secs(20);

pub async fn status(State(state): State<SharedState>) -> Result<Json<Value>> {
    let cached = state.status_cache.lock().await.clone();

    if let Some((at, payload)) = cached {
        if Instant::now().duration_since(at) < STATUS_TTL {
            return Ok(Json((*payload).clone()));
        }
        // Stale: answer now, refresh behind the request.
        if !state.status_refreshing.swap(true, Ordering::SeqCst) {
            let bg = state.clone();
            tokio::spawn(async move {
                let _guard = ResetWhenDropped(bg.clone());
                match build_status(&bg).await {
                    Ok(v) => *bg.status_cache.lock().await = Some((Instant::now(), v)),
                    Err(e) => tracing::warn!("background /api/status refresh failed: {e}"),
                }
            });
        }
        return Ok(Json((*payload).clone()));
    }

    let payload = build_status(&state).await?;
    *state.status_cache.lock().await = Some((Instant::now(), payload.clone()));
    Ok(Json((*payload).clone()))
}

struct ResetWhenDropped(SharedState);
impl Drop for ResetWhenDropped {
    fn drop(&mut self) {
        self.0.status_refreshing.store(false, Ordering::SeqCst);
    }
}

/// Primes the `/api/status` cache at startup so the first page load is instant
/// instead of paying for the Clash probe (~2s when Clash is not running).
pub async fn warm(state: &SharedState) {
    match build_status(state).await {
        Ok(v) => *state.status_cache.lock().await = Some((Instant::now(), v)),
        Err(e) => tracing::warn!("status warm-up failed: {e}"),
    }
}

async fn build_status(state: &SharedState) -> Result<Arc<Value>> {
    // Liveness probes run concurrently; they used to serialise, stalling the
    // handler for seconds whenever Clash was offline.
    let (meow_probe, clash_online) =
        tokio::join!(state.meow_checker.check(), state.clash_client.check_health());
    let (meow_online, meow_latency) = meow_probe;

    let auto_detect = state.config.clash.auto_detect_profiles;
    let discovered_files: Arc<Vec<String>> = if auto_detect {
        cached_profile_paths(state).await
    } else {
        Arc::new(Vec::new())
    };

    let nodes_count = state.nodes.read().await.len();
    let users_count = state.users.read().await.len();

    Ok(Arc::new(json!({
        "success": true,
        "clash": {
            "online": clash_online,
            "api_url": state.config.clash.api_url,
            "auto_detect": auto_detect,
            "discovered_profiles": discovered_files,
        },
        "meow": {
            "online": meow_online,
            "api_url": state.config.meow.api_url,
            "latency_ms": meow_latency,
            "gateway_port": state.config.meow.gateway_port,
            "gateway_bind": state.config.meow.gateway_bind,
            "mixed_port": state.config.meow.mixed_port,
        },
        "stats": {
            "nodes": nodes_count,
            "users": users_count,
        }
    })))
}

/// Cached profile-directory scan.
///
/// `discover_all_configs()` enumerates every process and walks several profile
/// trees. The layout changes rarely, but `/api/status` is polled continuously, so
/// rescanning on every poll was pure waste. Runs on a blocking thread.
const PROFILES_TTL: Duration = Duration::from_secs(300);

async fn cached_profile_paths(state: &SharedState) -> Arc<Vec<String>> {
    {
        let guard = state.profiles_cache.lock().await;
        if let Some((at, paths)) = guard.as_ref() {
            if Instant::now().duration_since(*at) < PROFILES_TTL {
                return paths.clone();
            }
        }
    }
    let found = Arc::new(
        tokio::task::spawn_blocking(|| {
            crate::collector::detector::ClashDetector::discover_all_configs()
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<String>>()
        })
        .await
        .unwrap_or_default(),
    );
    *state.profiles_cache.lock().await = Some((Instant::now(), found.clone()));
    found
}
