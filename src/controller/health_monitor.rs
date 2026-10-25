//! Active health-checking / node lifecycle management.
//!
//! Design follows the proven pattern used by HAProxy / Envoy / Clash-Verge style
//! gateways:
//!
//! * **Active probing** — every `interval_secs` each proxy is tested through the
//!   kernel's own `/proxies/{name}/delay` API (so rules/`REJECT` placeholders do
//!   not interfere with the measurement).
//! * **Hysteresis / state machine** — a node is only taken *out* after
//!   `fail_threshold` consecutive failures and only brought back after
//!   `recover_threshold` consecutive successes.  This prevents flapping caused by
//!   a single packet loss (exactly what `fallback`/`url-test` cannot do, because
//!   they react to a single sample).
//! * **Backoff** — a DOWN node is re-tested with `down_retry_secs` spacing so an
//!   expensive graveyard of dead nodes does not slow every full sweep down, while
//!   recovered nodes are still discovered and put back automatically.
//! * **Effective set** — every config generation excludes DOWN / manually
//!   disabled nodes from *all* proxy groups (per-node `node-X`, region `url-test`
//!   and region `fallback`), so the policy applies gateway-wide with no partial
//!   state.
//! * **Change-triggered reload** — whenever the effective up/down set changes the
//!   YAML is regenerated and hot-reloaded (debounced) so routes update at once.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::{HealthPolicy, MeowConfig};

static LAST_RELOAD_UNIX: AtomicU64 = AtomicU64::new(0);
static PENDING_RELOAD: AtomicBool = AtomicBool::new(false);

/// Triggers a debounced config reload. To prevent kernel freezing when nodes flap rapidly,
/// auto-reloads are throttled to at most once every 30 seconds.
pub async fn trigger_debounced_reload(state: &crate::state::SharedState) {
    let now = Utc::now().timestamp() as u64;
    let last = LAST_RELOAD_UNIX.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 30 {
        PENDING_RELOAD.store(true, Ordering::Relaxed);
        tracing::debug!("health change queued for debounced reload (cooldown active)");
        return;
    }

    LAST_RELOAD_UNIX.store(now, Ordering::Relaxed);
    PENDING_RELOAD.store(false, Ordering::Relaxed);
    tracing::info!("routable node set changed -> regenerating gateway config (debounced)");
    if let Err(e) = state.apply_and_reload().await {
        tracing::warn!("Re-generate after health change failed: {}", e);
    }
}

pub async fn flush_pending_reload_if_due(state: &crate::state::SharedState) {
    if !PENDING_RELOAD.load(Ordering::Relaxed) {
        return;
    }
    let now = Utc::now().timestamp() as u64;
    let last = LAST_RELOAD_UNIX.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= 30 {
        LAST_RELOAD_UNIX.store(now, Ordering::Relaxed);
        PENDING_RELOAD.store(false, Ordering::Relaxed);
        tracing::info!("flushing pending gateway config reload");
        if let Err(e) = state.apply_and_reload().await {
            tracing::warn!("Flushing gateway reload failed: {}", e);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// Passing checks.
    Up,
    /// Failed but not enough consecutive failures yet (still routed).
    Recovering,
    /// Tripped: removed from every proxy group.
    Down,
    /// Manually disabled by the operator (removed from every group).
    Disabled,
}

impl HealthState {
    pub fn is_available(&self) -> bool {
        matches!(self, HealthState::Up | HealthState::Recovering)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_name: String,
    pub state: HealthState,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub consecutive_successes: u32,
    #[serde(default)]
    pub total_checks: u64,
    #[serde(default)]
    pub total_failures: u64,
    #[serde(default)]
    pub last_latency_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_checked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_change_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub down_since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub manual_disabled: bool,
    #[serde(default)]
    pub last_transition: Option<String>,
    /// Next scheduled probe (rolling scheduler, Envoy style).
    #[serde(default)]
    pub next_check_at: Option<DateTime<Utc>>,
}

impl NodeHealth {
    pub fn new(node_name: &str) -> Self {
        Self {
            node_name: node_name.to_string(),
            state: HealthState::Up,
            consecutive_failures: 0,
            consecutive_successes: 0,
            total_checks: 0,
            total_failures: 0,
            last_latency_ms: None,
            last_error: None,
            last_checked_at: None,
            last_change_at: None,
            down_since: None,
            manual_disabled: false,
            last_transition: None,
            next_check_at: None,
        }
    }
}

/// Result of a single health probe.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// The health registry: current state of every node plus the active policy.
pub struct HealthMonitor {
    policy: RwLock<HealthPolicy>,
    states: RwLock<HashMap<String, NodeHealth>>,
    client: reqwest::Client,
    /// Guards against overlapping sweeps (a full sweep of thousands of nodes can
    /// outlast the tick interval).
    sweeping: std::sync::atomic::AtomicBool,
}

impl HealthMonitor {
    pub fn new(policy: HealthPolicy) -> Self {
        Self {
            policy: RwLock::new(policy),
            states: RwLock::new(HashMap::new()),
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default(),
            sweeping: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Tries to acquire the single-flight sweep lock.
    fn begin_sweep(&self) -> bool {
        !self
            .sweeping
            .swap(true, std::sync::atomic::Ordering::SeqCst)
    }

    fn end_sweep(&self) {
        self.sweeping
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn is_sweeping(&self) -> bool {
        self.sweeping.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn policy(&self) -> HealthPolicy {
        self.policy.read().await.clone()
    }

    pub async fn set_policy(&self, policy: HealthPolicy) {
        let interval_changed = {
            let mut p = self.policy.write().await;
            let changed = p.interval_secs != policy.interval_secs || p.down_retry_secs != policy.down_retry_secs;
            *p = policy.clone();
            changed
        };

        // If interval or down retry changed, immediately reschedule existing nodes
        // so the new timing takes effect right now instead of waiting for the old timer!
        if interval_changed {
            let now = chrono::Utc::now();
            let mut guard = self.states.write().await;
            for n in guard.values_mut() {
                if let Some(last) = n.last_checked_at {
                    if n.state.is_available() {
                        let target = last + chrono::Duration::seconds(policy.interval_secs as i64);
                        n.next_check_at = Some(target);
                    } else {
                        let retry_secs = if policy.down_retry_secs > 0 { policy.down_retry_secs } else { policy.interval_secs };
                        let down_target = last + chrono::Duration::seconds(retry_secs as i64);
                        n.next_check_at = Some(down_target);
                    }
                } else {
                    n.next_check_at = Some(now);
                }
            }
        }
    }

    pub async fn all_states(&self) -> Vec<NodeHealth> {
        let mut v: Vec<NodeHealth> = self.states.read().await.values().cloned().collect();
        v.sort_by(|a, b| a.node_name.cmp(&b.node_name));
        v
    }

    pub async fn get(&self, node: &str) -> Option<NodeHealth> {
        self.states.read().await.get(node).cloned()
    }

    /// Names that must be excluded from routing right now.
    pub async fn unavailable(&self) -> HashSet<String> {
        self.states
            .read()
            .await
            .values()
            .filter(|h| !h.state.is_available())
            .map(|h| h.node_name.clone())
            .collect()
    }

    pub async fn restore_persisted(&self, rows: Vec<(String, String)>) {
        let mut m = self.states.write().await;
        let mut n = 0;
        for (name, json) in rows {
            if let Ok(h) = serde_json::from_str::<NodeHealth>(&json) {
                m.insert(name, h);
                n += 1;
            }
        }
        if n > 0 {
            tracing::info!("Restored {} persisted node health record(s)", n);
        }
    }

    /// Ensures every current node has a health record and drops stale ones.
    pub async fn retain_nodes(&self, nodes: &[String]) {
        let mut m = self.states.write().await;
        let keep: HashSet<&String> = nodes.iter().collect();
        m.retain(|k, _| keep.contains(k));
        for n in nodes {
            m.entry(n.clone()).or_insert_with(|| NodeHealth::new(n));
        }
    }

    /// Applies one probe outcome through the hysteresis state machine.
    /// Returns `(state, transitioned)` where `transitioned` means the node's
    /// *routability* changed (so the caller should regenerate the config).
    pub async fn record(&self, node: &str, outcome: CheckOutcome) -> (NodeHealth, bool) {
        let policy = self.policy.read().await.clone();
        let mut m = self.states.write().await;
        let h = m
            .entry(node.to_string())
            .or_insert_with(|| NodeHealth::new(node));

        let before = h.state;
        h.total_checks += 1;
        h.last_checked_at = Some(Utc::now());
        h.last_latency_ms = outcome.latency_ms;

        if outcome.ok {
            h.consecutive_failures = 0;
            h.consecutive_successes = h.consecutive_successes.saturating_add(1);
            h.last_error = None;
            if !h.manual_disabled {
                let was_down = !before.is_available();
                let enough = h.consecutive_successes >= policy.recover_threshold.max(1);
                if enough && h.state != HealthState::Up {
                    h.state = HealthState::Up;
                    h.down_since = None;
                    h.last_change_at = Some(Utc::now());
                    h.last_transition = Some(if was_down {
                        format!("recovered after {} success(es)", h.consecutive_successes)
                    } else {
                        format!("healthy after {} success(es)", h.consecutive_successes)
                    });
                }
            }
        } else {
            h.total_failures += 1;
            h.consecutive_successes = 0;
            h.consecutive_failures = h.consecutive_failures.saturating_add(1);
            h.last_error = outcome.error.clone();
            if !h.manual_disabled {
                if h.consecutive_failures >= policy.fail_threshold.max(1) {
                    if h.state != HealthState::Down {
                        h.state = HealthState::Down;
                        h.down_since = Some(Utc::now());
                        h.last_change_at = Some(Utc::now());
                        h.last_transition = Some(format!(
                            "tripped after {} consecutive failure(s)",
                            h.consecutive_failures
                        ));
                    }
                } else if h.state == HealthState::Up {
                    // Degraded but still routed: do not flap on a single failure.
                    h.state = HealthState::Recovering;
                    h.last_change_at = Some(Utc::now());
                    h.last_transition = Some(format!(
                        "degraded: {}/{} failure(s) before trip",
                        h.consecutive_failures, policy.fail_threshold
                    ));
                }
            }
        }

        // Rolling schedule: healthy nodes are re-checked on the normal interval,
        // tripped nodes with exponential-ish backoff so a large graveyard of dead
        // nodes does not slow the healthy majority down.
        let base = if h.state.is_available() {
            policy.interval_secs.max(5)
        } else {
            policy.down_retry_secs.max(policy.interval_secs).max(5)
        };
        // jitter: +/-10% to avoid every node becoming due at the same instant
        let jitter = (base as f64 * 0.1 * (rand::random::<f64>() * 2.0 - 1.0)) as i64;
        let next = base as i64 + jitter;
        h.next_check_at = Some(Utc::now() + chrono::Duration::seconds(next.max(5)));

        let after = h.state;
        let changed = before.is_available() != after.is_available();
        (h.clone(), changed)
    }

    /// Manual operator action. Returns true when routability changed.
    pub async fn set_manual_disabled(&self, node: &str, disabled: bool) -> bool {
        let mut m = self.states.write().await;
        let h = m
            .entry(node.to_string())
            .or_insert_with(|| NodeHealth::new(node));
        let before = h.state.is_available();
        h.manual_disabled = disabled;
        if disabled {
            h.state = HealthState::Disabled;
            h.down_since = Some(Utc::now());
            h.last_change_at = Some(Utc::now());
            h.last_transition = Some("manually disabled by operator".to_string());
        } else {
            h.state = HealthState::Up;
            h.down_since = None;
            h.consecutive_failures = 0;
            h.last_change_at = Some(Utc::now());
            h.last_transition = Some("manually re-enabled by operator".to_string());
        }
        before != h.state.is_available()
    }

    pub async fn count_by_state(&self) -> (usize, usize, usize, usize) {
        let m = self.states.read().await;
        let mut up = 0;
        let mut recovering = 0;
        let mut down = 0;
        let mut disabled = 0;
        for h in m.values() {
            match h.state {
                HealthState::Up => up += 1,
                HealthState::Recovering => recovering += 1,
                HealthState::Down => down += 1,
                HealthState::Disabled => disabled += 1,
            }
        }
        (up, recovering, down, disabled)
    }

    /// Probe one node through the kernel delay API, falling back to a real
    /// request through the node's dedicated mixed-port user.
    pub async fn check_node(
        &self,
        meow: &MeowConfig,
        node: &str,
        fallback_credential: Option<(String, String)>,
    ) -> CheckOutcome {
        let policy = self.policy.read().await.clone();
        let timeout = policy.timeout_ms.max(500);

        if let Some(outcome) = self.check_via_kernel_api(meow, node, timeout).await {
            return outcome;
        }
        if let Some((user, pass)) = fallback_credential {
            let res = crate::controller::prober::NodeProber::probe_node(
                meow.mixed_port,
                &user,
                &pass,
                (timeout / 1000).max(3),
            )
            .await;
            return CheckOutcome {
                ok: res.error.is_none() && res.latency_ms.is_some(),
                latency_ms: res.latency_ms,
                error: res.error,
            };
        }
        CheckOutcome {
            ok: false,
            latency_ms: None,
            error: Some("kernel delay API unavailable and no credential to probe".to_string()),
        }
    }

    /// `GET /proxies/{name}/delay` — the canonical Clash/Mihomo health probe.
    /// Returns `None` when the API itself is unreachable / unsupported, so the
    /// caller can fall back.
    async fn check_via_kernel_api(
        &self,
        meow: &MeowConfig,
        node: &str,
        timeout_ms: u64,
    ) -> Option<CheckOutcome> {
        let policy = self.policy.read().await.clone();
        let base = meow.api_url.trim_end_matches('/').to_string();
        let url = format!(
            "{}/proxies/{}/delay?timeout={}&url={}",
            base,
            urlencode(node),
            timeout_ms,
            urlencode(&policy.test_url)
        );
        let mut req = self.client.get(&url);
        if let Some(secret) = &meow.api_secret {
            if !secret.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", secret));
            }
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("kernel delay API unreachable for {}: {}", node, e);
                return None;
            }
        };

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if status.as_u16() == 404 {
            // Unknown proxy -> let the caller fall back to a direct probe.
            tracing::debug!("kernel reports unknown proxy {}: {}", node, body);
            return None;
        }
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| format!("HTTP {}", status));
            return Some(CheckOutcome {
                ok: false,
                latency_ms: None,
                error: Some(msg),
            });
        }
        match serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("delay").and_then(|d| d.as_u64()))
        {
            Some(delay) => Some(CheckOutcome {
                ok: true,
                latency_ms: Some(delay),
                error: None,
            }),
            None => Some(CheckOutcome {
                ok: false,
                latency_ms: None,
                error: Some(format!("unexpected delay payload: {}", body)),
            }),
        }
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

impl HealthMonitor {
    /// Nodes that are due for a probe, oldest-due first.
    pub async fn due_nodes(&self, limit: usize) -> Vec<String> {
        let now = Utc::now();
        let m = self.states.read().await;
        let mut due: Vec<(Option<DateTime<Utc>>, String)> = m
            .values()
            .filter(|h| !h.manual_disabled)
            .filter(|h| match h.next_check_at {
                None => true,
                Some(t) => t <= now,
            })
            .map(|h| (h.next_check_at, h.node_name.clone()))
            .collect();
        due.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        due.into_iter().take(limit).map(|(_, n)| n).collect()
    }

    /// Force every node to be considered due (manual "check now").
    pub async fn force_all_due(&self) {
        let mut m = self.states.write().await;
        for h in m.values_mut() {
            h.next_check_at = None;
        }
    }

    /// Returns the earliest next_check_at among all non-disabled nodes, and remaining seconds until that check.
    pub async fn earliest_next_check(&self) -> (Option<DateTime<Utc>>, Option<i64>) {
        let now = Utc::now();
        let m = self.states.read().await;
        let mut min_t: Option<DateTime<Utc>> = None;
        let mut has_due = false;

        for h in m.values() {
            if h.manual_disabled {
                continue;
            }
            match h.next_check_at {
                None => has_due = true,
                Some(t) => {
                    if t <= now {
                        has_due = true;
                    } else if min_t.map(|cur| t < cur).unwrap_or(true) {
                        min_t = Some(t);
                    }
                }
            }
        }

        if has_due {
            // A node is due for probe right now (0 seconds remaining)
            (min_t.or(Some(now)), Some(0))
        } else if let Some(t) = min_t {
            let secs = (t - now).num_seconds().max(0);
            (Some(t), Some(secs))
        } else {
            (None, None)
        }
    }
}

/// Probes up to `limit` due nodes (one rolling batch). Returns true when the
/// routable set changed and the gateway config therefore had to be reloaded.
pub async fn run_due_batch(state: &crate::state::SharedState, limit: usize) -> bool {
    use tokio::sync::Semaphore;

    let policy = state.health.policy().await;
    if !policy.enabled {
        return false;
    }

    // While the kernel is not listening, every probe fails twice over (control
    // API + mixed port), which burns CPU and floods the log. Wait for it.
    let (kernel_up, _) = state.meow_checker.check().await;
    if !kernel_up {
        tracing::debug!("health sweep deferred: gateway control API is offline");
        return false;
    }

    let names = state.health.due_nodes(limit.max(1)).await;
    if names.is_empty() {
        return false;
    }

    let users = state.users.read().await.clone();
    let cred: HashMap<String, (String, String)> = users
        .into_iter()
        .map(|u| (u.node_name, (u.username, u.password)))
        .collect();

    let meow = state.config.meow.clone();
    let sem = Arc::new(Semaphore::new(policy.concurrency.max(1)));
    let mut set = tokio::task::JoinSet::new();
    let probed = names.clone();

    for name in names {
        let st = state.clone();
        let sem = sem.clone();
        let meow = meow.clone();
        let c = cred.get(&name).cloned();
        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let outcome = st.health.check_node(&meow, &name, c).await;
            let (h, changed) = st.health.record(&name, outcome.clone()).await;
            if changed {
                tracing::info!(
                    "node health transition: {} -> {:?} ({})",
                    h.node_name,
                    h.state,
                    h.last_transition.clone().unwrap_or_default()
                );
            }

            // Sync health check outcome to node probe result and connectivity history (10-slot rolling squares)
            let history_entry = crate::controller::prober::ProbeHistoryEntry {
                ok: outcome.ok,
                latency_ms: outcome.latency_ms, speed_bps: None, checked_at: Utc::now(),
            };
            let probe_res = crate::controller::prober::NodeProbeResult {
                latency_ms: outcome.latency_ms, speed_bps: None, speed_formatted: None,
                outbound_ip: None,
                location: None,
                country_code: None,
                error: outcome.error.clone(),
                checked_at: Some(Utc::now()),
                history: vec![history_entry],
            };
            st.update_probe_result(name.clone(), probe_res).await;

            changed
        });
    }

    let mut changed = false;
    while let Some(res) = set.join_next().await {
        if let Ok(ch) = res {
            changed |= ch;
        }
    }

    // Persist only the nodes touched here: rewriting all 1800+ rows per batch
    // was a large amount of pointless SQLite work.
    state.persist_health_for(&probed).await;

    if changed {
        trigger_debounced_reload(state).await;
    }
    changed
}

/// Manual "check everything now": marks all nodes due then drains the queue.
pub async fn run_check_cycle(state: &crate::state::SharedState) -> bool {
    if !state.health.begin_sweep() {
        tracing::debug!("health sweep already running, ignoring duplicate trigger");
        return false;
    }
    let _guard = SweepGuard(&state.health);

    state.health.force_all_due().await;
    let policy = state.health.policy().await;
    let batch = policy.concurrency.max(1) * 4;
    let mut changed = false;
    let mut rounds = 0usize;
    loop {
        let due = state.health.due_nodes(1).await;
        if due.is_empty() {
            break;
        }
        changed |= run_due_batch(state, batch).await;
        rounds += 1;
        if rounds > 10_000 {
            tracing::warn!("health sweep aborted after too many batches");
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    state.persist_health().await;
    changed
}

struct SweepGuard<'a>(&'a HealthMonitor);
impl Drop for SweepGuard<'_> {
    fn drop(&mut self) {
        self.0.end_sweep();
    }
}

/// Background scheduler: runs `run_check_cycle` on the configured interval.
pub async fn monitor_loop(state: crate::state::SharedState) {
    tracing::info!("node health monitor started");
    // Prime the schedule so the first tick after startup starts probing.
    state.health.force_all_due().await;
    let mut logged_wait = false;
    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let policy = state.health.policy().await;
        if !policy.enabled {
            continue;
        }
        let (kernel_up, _) = state.meow_checker.check().await;
        if !kernel_up {
            if !logged_wait {
                tracing::info!("health monitor waiting for the gateway kernel to come online");
                logged_wait = true;
            }
            continue;
        }
        logged_wait = false;
        // Drain as many due batches as available capacity allows, then sleep.
        for _ in 0..8 {
            let due = state.health.due_nodes(1).await;
            if due.is_empty() {
                break;
            }
            run_due_batch(&state, policy.concurrency.max(1) * 2).await;
        }
        flush_pending_reload_if_due(&state).await;
    }
}

/// Convenience helper so callers can express "is this path inside the gateway".
#[allow(dead_code)]
pub fn config_is_local(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> HealthPolicy {
        HealthPolicy {
            enabled: true,
            interval_secs: 60,
            timeout_ms: 5000,
            fail_threshold: 3,
            recover_threshold: 2,
            down_retry_secs: 120,
            concurrency: 64,
            test_url: "http://cp.cloudflare.com/generate_204".into(),
        }
    }

    #[tokio::test]
    async fn test_hysteresis_does_not_flap() {
        let m = HealthMonitor::new(policy());
        // one failure must NOT remove the node
        let (h, changed) = m
            .record(
                "n1",
                CheckOutcome {
                    ok: false,
                    latency_ms: None,
                    error: Some("timeout".into()),
                },
            )
            .await;
        assert_eq!(h.state, HealthState::Recovering);
        assert!(!changed);
        assert!(h.state.is_available());

        // two more failures trip it
        m.record("n1", CheckOutcome { ok: false, latency_ms: None, error: Some("timeout".into()) }).await;
        let (h, changed) = m.record("n1", CheckOutcome { ok: false, latency_ms: None, error: Some("timeout".into()) }).await;
        assert_eq!(h.state, HealthState::Down);
        assert!(changed);
        assert!(m.unavailable().await.contains("n1"));

        // a single success is not enough to restore
        let (h, changed) = m.record("n1", CheckOutcome { ok: true, latency_ms: Some(120), error: None }).await;
        assert_eq!(h.state, HealthState::Down);
        assert!(!changed);

        // second success restores it
        let (h, changed) = m.record("n1", CheckOutcome { ok: true, latency_ms: Some(90), error: None }).await;
        assert_eq!(h.state, HealthState::Up);
        assert!(changed);
        assert!(m.unavailable().await.is_empty());
    }

    #[tokio::test]
    async fn test_manual_disable_and_restore() {
        let m = HealthMonitor::new(policy());
        m.record("n1", CheckOutcome { ok: true, latency_ms: Some(10), error: None }).await;
        assert!(m.set_manual_disabled("n1", true).await);
        assert!(m.unavailable().await.contains("n1"));
        // probes must not silently re-enable a manually disabled node
        let (h, changed) = m.record("n1", CheckOutcome { ok: true, latency_ms: Some(10), error: None }).await;
        assert_eq!(h.state, HealthState::Disabled);
        assert!(!changed);
        assert!(m.set_manual_disabled("n1", false).await);
        assert!(m.unavailable().await.is_empty());
    }

    #[test]
    fn test_urlencode_unicode_names() {
        let s = urlencode("🇯🇵 免费-日本1-Ver.7");
        assert!(!s.contains(' '));
        assert!(s.contains("%20"));
    }
}
