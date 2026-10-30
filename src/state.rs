use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use crate::config::AppConfig;
use crate::collector::{Node, User};
use crate::collector::clash_api::ClashApiClient;
use crate::collector::yaml_parser::YamlParser;
use crate::generator::auth::AuthGenerator;
use crate::generator::template::TemplateRenderer;
use crate::controller::reload::MeowReloader;
use crate::controller::health::MeowHealthChecker;
use crate::controller::manager::CoreManager;
use crate::controller::health_monitor::{HealthMonitor, NodeHealth};
use crate::controller::prober::NodeProbeResult;
use crate::controller::traffic_tracker::NodeTrafficStats;
use crate::store::db::{Database, StoredUser, StoredSource};
use chrono::Utc;
use crate::error::Result;
use std::collections::HashMap;

pub struct AppState {
    pub config: AppConfig,
    pub db: Database,
    pub nodes: RwLock<Vec<Node>>,
    pub users: RwLock<Vec<User>>,
    pub probe_results: RwLock<HashMap<String, NodeProbeResult>>,
    pub traffic_stats: RwLock<HashMap<String, NodeTrafficStats>>,
    pub core_manager: CoreManager,
    pub health: HealthMonitor,
    pub clash_client: ClashApiClient,
    pub meow_reloader: MeowReloader,
    pub meow_checker: MeowHealthChecker,
    /// Memoised `/api/status` payload — its inputs are expensive to probe.
    pub status_cache: Mutex<Option<(std::time::Instant, Arc<serde_json::Value>)>>,
    /// Memoised profile-directory scan (see `api::status::cached_profile_paths`).
    pub profiles_cache: Mutex<Option<ProfileScan>>,
    /// Guards against stacking background `/api/status` refreshes.
    pub status_refreshing: std::sync::atomic::AtomicBool,
    /// Guards against stacking background node probe sweeps.
    pub is_probing: std::sync::atomic::AtomicBool,
    /// JWT / HMAC session auth manager for Web UI & API authentication.
    pub auth_mgr: Arc<crate::controller::auth_token::AuthManager>,
}

/// Cached `/api/status` profile scan: when it ran + what it found.
pub type ProfileScan = (std::time::Instant, Arc<Vec<String>>);

pub type SharedState = Arc<AppState>;

impl AppState {
    pub async fn new(config: AppConfig) -> Result<SharedState> {
        let db = Database::init(&config.database.url).await?;
        let config_health = config.health.clone();

        let clash_client = ClashApiClient::new(
            &config.clash.api_url,
            config.clash.api_secret.clone(),
        );

        let meow_reloader = MeowReloader::new(
            &config.meow.api_url,
            config.meow.api_secret.clone(),
        );

        let meow_checker = MeowHealthChecker::new(
            &config.meow.api_url,
            config.meow.api_secret.clone(),
        );

        let auth_mgr = Arc::new(crate::controller::auth_token::AuthManager::new(&config.auth.jwt_secret));

        let state = Arc::new(Self {
            config,
            db,
            nodes: RwLock::new(Vec::new()),
            users: RwLock::new(Vec::new()),
            probe_results: RwLock::new(HashMap::new()),
            traffic_stats: RwLock::new(HashMap::new()),
            core_manager: CoreManager::new(),
            health: HealthMonitor::new(config_health),
            clash_client,
            meow_reloader,
            meow_checker,
            status_cache: Mutex::new(None),
            profiles_cache: Mutex::new(None),
            status_refreshing: std::sync::atomic::AtomicBool::new(false),
            is_probing: std::sync::atomic::AtomicBool::new(false),
            auth_mgr,
        });

        // Initialize default sources if database is empty
        state.init_default_sources().await?;

        // Initialize nodes & users
        if let Err(e) = state.sync_nodes_and_users(false).await {
            tracing::warn!("Initial node sync notice: {}", e);
        }

        // Restore persisted probe history + node health records + traffic stats
        state.load_persisted_probes().await;
        state.load_persisted_health().await;
        state.load_persisted_traffic().await;
        state.load_persisted_health_policy().await;

        // Seed health records for every known node so the first sweep is complete
        {
            let names: Vec<String> = state.nodes.read().await.iter().map(|n| n.name.clone()).collect();
            state.health.retain_nodes(&names).await;
        }

        // Apply initial configuration and ensure proxy core is running
        if let Err(e) = state.apply_and_reload().await {
            tracing::warn!("Initial apply_and_reload notice: {}", e);
        }

        Ok(state)
    }

    pub async fn init_default_sources(&self) -> Result<()> {
        let existing = self.db.get_all_sources().await?;
        let mut existing_map: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        for s in &existing {
            existing_map.insert(s.id.clone(), s.enabled);
        }

        // Clean up any legacy monolithic "auto_detect" sources so all sources are individual files or gateways
        for s in &existing {
            if s.source_type == "auto_detect" || s.id == "default-auto-detect" {
                let _ = self.db.delete_source(&s.id).await;
            }
        }

        // Register default-clash-api if not present
        if !existing_map.contains_key("default-clash-api") {
            let _ = self.db.save_source(&StoredSource {
                id: "default-clash-api".to_string(),
                name: "本机 Clash REST API".to_string(),
                source_type: "clash_api".to_string(),
                target: self.config.clash.api_url.clone(),
                secret: self.config.clash.api_secret.clone(),
                enabled: false,
                is_base: false,
                created_at: Utc::now(),
            }).await;
        }

        // Only on initial setup (first boot with empty sources), scan once
        if existing.is_empty() || (existing.len() == 1 && existing_map.contains_key("default-clash-api")) {
            let _ = self.refresh_discovered_sources().await;
        }

        Ok(())
    }

    /// Scans local Clash profiles and registers discrete sources.
    /// Pure comparison against currently registered sources — zero blacklists, zero tombstones.
    /// Returns: (discovered_count, newly_added_count)
    pub async fn refresh_discovered_sources(&self) -> Result<(usize, usize)> {
        let existing = self.db.get_all_sources().await?;
        let mut existing_ids = std::collections::HashSet::new();
        let mut existing_targets = std::collections::HashSet::new();
        let mut existing_canonicals = std::collections::HashSet::new();

        for s in &existing {
            existing_ids.insert(s.id.clone());
            existing_targets.insert(s.target.clone());
            let p = std::path::Path::new(&s.target);
            if let Ok(c) = std::fs::canonicalize(p) {
                existing_canonicals.insert(c);
            }
        }

        let mut custom_dirs = Vec::new();
        if let Some(ref dir) = self.config.clash.profiles_dir {
            custom_dirs.push(dir.clone());
        }

        let profiles = crate::collector::detector::ClashDetector::discover_profile_sources_with_custom(&custom_dirs);
        let discovered_count = profiles.len();
        let mut newly_added = 0;

        for p in profiles {
            let target_str = p.path.to_string_lossy().to_string();
            let canonical_p = std::fs::canonicalize(&p.path).unwrap_or_else(|_| p.path.clone());

            // Only add profiles that are not currently in the sources table
            let is_already_registered = existing_ids.contains(&p.id)
                || existing_targets.contains(&target_str)
                || existing_canonicals.contains(&canonical_p);

            if !is_already_registered {
                self.db.save_source(&StoredSource {
                    id: p.id.clone(),
                    name: p.name,
                    source_type: "clash_file".to_string(),
                    target: target_str.clone(),
                    secret: None,
                    enabled: true,
                    is_base: false,
                    created_at: Utc::now(),
                }).await?;

                existing_ids.insert(p.id);
                existing_targets.insert(target_str);
                existing_canonicals.insert(canonical_p);
                newly_added += 1;
            }
        }

        Ok((discovered_count, newly_added))
    }

    /// Fetches nodes in parallel from all enabled sources:
    /// - auto_detect (scans local processes + standard profiles directories)
    /// - clash_api (RESTful :9090 or custom host/port with secret)
    /// - clash_dir (scans custom directory for .yaml/.yml profiles)
    /// - clash_file (specific .yaml configuration file)
    ///
    /// All nodes are aggregated and deduplicated.
    pub async fn sync_nodes_and_users(&self, force_refresh_creds: bool) -> Result<usize> {
        let sources = self.db.get_all_sources().await?;
        let enabled_sources: Vec<StoredSource> = sources.into_iter().filter(|s| s.enabled).collect();

        let mut aggregated_nodes: Vec<Node> = Vec::new();
        let mut node_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut node_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

        let mut add_node = |mut node: Node, is_base: bool| {
            let key = node.dedup_key();
            // 1. If exact same physical node + same user credential already exists, it is a duplicate -> skip
            // Note: If the source has is_base = true (e.g. multi-channel load-balancing us1-01..us1-10 with identical server:port),
            // we preserve each distinct channel rather than collapsing them into one!
            if !node.server.is_empty() && node_keys.contains(&key) && !is_base {
                return;
            }

            // 2. If node name is already taken (e.g. multi-user scenario where different user subscriptions name the node identically):
            // Disambiguate name so both users' nodes are preserved AND Mihomo kernel does not fail on duplicate proxy name!
            if node_names.contains(&node.name) {
                let suffix = node.credential_id()
                    .map(|c| {
                        let h = format!("{:04x}", (c.bytes().fold(0u32, |acc, b| acc.wrapping_add(b as u32)) ^ (c.len() as u32)) & 0xffff);
                        format!(" [{}]", h)
                    })
                    .unwrap_or_else(|| " [2]".to_string());

                let mut disambiguated = format!("{}{}", node.name, suffix);
                let mut idx = 2;
                while node_names.contains(&disambiguated) {
                    disambiguated = format!("{} [{}]", node.name, idx);
                    idx += 1;
                }
                node.name = disambiguated;
            }

            node_names.insert(node.name.clone());
            node_keys.insert(key);
            aggregated_nodes.push(node);
        };

        for source in &enabled_sources {
            let is_base = source.is_base;
            tracing::info!("Collecting nodes from enabled source: [{}] {} (type: {}, is_base: {})", source.id, source.name, source.source_type, is_base);
            match source.source_type.as_str() {
                "clash_api" => {
                    let client = ClashApiClient::new(&source.target, source.secret.clone());
                    match client.fetch_nodes().await {
                        Ok(api_nodes) => {
                            for node in api_nodes {
                                add_node(node, is_base);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to fetch from clash_api source '{}' ({}): {}", source.name, source.target, e);
                        }
                    }
                }
                "clash_dir" => {
                    let dir = std::path::Path::new(&source.target);
                    if dir.exists() && dir.is_dir() {
                        for entry in walkdir::WalkDir::new(dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
                            let path = entry.path();
                            if path.is_file() {
                                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                                    let ext_l = ext.to_lowercase();
                                    if ext_l == "yaml" || ext_l == "yml" {
                                        if let Ok(parsed) = YamlParser::parse_file(path) {
                                            for node in parsed {
                                                add_node(node, is_base);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        tracing::warn!("clash_dir source directory does not exist: {}", source.target);
                    }
                }
                "clash_file" => {
                    let path = std::path::Path::new(&source.target);
                    if path.exists() && path.is_file() {
                        if let Ok(parsed) = YamlParser::parse_file(path) {
                            for node in parsed {
                                add_node(node, is_base);
                            }
                        }
                    } else {
                        tracing::warn!("clash_file does not exist: {}", source.target);
                    }
                }
                other => {
                    tracing::warn!("Unknown source type: {}", other);
                }
            }
        }

        let nodes = aggregated_nodes;

        // 2. Fetch existing users from DB
        let stored_users = self.db.get_all_users().await?;
        let mut user_map: std::collections::HashMap<String, StoredUser> = stored_users
            .into_iter()
            .map(|u| (u.node_name.clone(), u))
            .collect();

        let mut users = Vec::new();

        for node in &nodes {
            let existing_opt = user_map.remove(&node.name);
            let needs_new_user = force_refresh_creds || match &existing_opt {
                Some(existing) => {
                    // Migrate legacy usernames to compact "region+hash" (e.g. us86a50b4f, hk39c0ea95)
                    let expected = AuthGenerator::sanitize_username(&node.name);
                    existing.username != expected
                }
                None => true,
            };

            if needs_new_user {
                let user = AuthGenerator::create_user_for_node(&node.name);
                let stored = StoredUser {
                    node_name: user.node_name.clone(),
                    username: user.username.clone(),
                    password: user.password.clone(),
                    created_at: user.created_at,
                };
                self.db.save_user(&stored).await?;
                users.push(user);
            } else if let Some(existing) = existing_opt {
                users.push(User {
                    node_name: existing.node_name,
                    username: existing.username,
                    password: existing.password,
                    created_at: existing.created_at,
                });
            }
        }

        let node_count = nodes.len();

        // Prune stale credentials for nodes removed by dedup / source toggling
        let keep: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
        match self.db.prune_users_except(&keep).await {
            Ok(removed) if removed > 0 => tracing::info!("Pruned {} stale credential(s) after dedup", removed),
            Err(e) => tracing::warn!("Failed to prune stale credentials: {}", e),
            _ => {}
        }
        // Prune persisted probes of removed nodes (keep synthetic region, IP, and level group entries)
        let mut keep: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
        keep.extend(
            self.probe_results
                .read()
                .await
                .keys()
                .filter(|k| k.starts_with("[region]") || k.starts_with("[ip]") || k.starts_with("[level]"))
                .cloned(),
        );
        if let Err(e) = self.db.prune_probes_except(&keep).await {
            tracing::warn!("Failed to prune stale probes: {}", e);
        }
        {
            let keep_set: std::collections::HashSet<String> = keep.iter().cloned().collect();
            let mut p = self.probe_results.write().await;
            p.retain(|k, _| keep_set.contains(k));
        }
        // Prune traffic stats for removed nodes
        {
            let node_set: std::collections::HashSet<String> = nodes.iter().map(|n| n.name.clone()).collect();
            let mut ts = self.traffic_stats.write().await;
            ts.retain(|k, _| node_set.contains(k));
        }
        if force_refresh_creds {
            crate::controller::prober::clear_prober_cache();
        }
        // Health registry: seed new nodes, drop removed ones, prune persisted rows
        {
            let names: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
            self.health.retain_nodes(&names).await;
            if let Err(e) = self.db.prune_health_except(&names).await {
                tracing::warn!("Failed to prune stale health records: {}", e);
            }
        }
        {
            let mut n_lock = self.nodes.write().await;
            *n_lock = nodes;
            let mut u_lock = self.users.write().await;
            *u_lock = users;
        }

        Ok(node_count)
    }

    /// Retrieves the effective base configuration string.
    /// Priority:
    /// 1. Any enabled source with `is_base: true` (reads from file path or URL)
    /// 2. If none, falls back to config.meow.base_config_path
    pub async fn get_effective_base_config(&self) -> Option<String> {
        let sources = self.db.get_all_sources().await.unwrap_or_default();
        let mut combined_bases = Vec::new();
        for s in sources {
            if s.enabled && s.is_base {
                let target = s.target.trim();
                let path = std::path::Path::new(target);
                if path.exists() && path.is_file() {
                    if let Ok(content) = std::fs::read_to_string(path) {
                        combined_bases.push(content);
                    }
                }
            }
        }

        if !combined_bases.is_empty() {
            return Some(combined_bases.join("\n---\n"));
        }

        if let Some(path) = &self.config.meow.base_config_path {
            if path.exists() {
                return std::fs::read_to_string(path).ok();
            }
        }
        None
    }

    /// Generates YAML config content (includes region auto/failover groups)
    pub async fn generate_config(&self) -> Result<String> {
        let nodes = self.nodes.read().await.clone();
        let users = self.users.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let unavailable = self.health.unavailable().await;

        let region_groups = crate::generator::groups::build_region_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let ip_groups = crate::generator::ip_groups::build_ip_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let level_groups = crate::generator::level_groups::build_level_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let speed_groups = self.list_speed_groups().await;

        let base_config = self.get_effective_base_config().await;

        TemplateRenderer::render_with_all_groups_cached(
            &nodes,
            None,
            &users,
            &region_groups,
            &ip_groups,
            &level_groups,
            &speed_groups,
            &unavailable,
            base_config.as_deref(),
            self.config.meow.mixed_port,
            &self.config.meow.default_target,
        )
    }

    /// Generates standalone Clash client subscription YAML without IN-USER rules
    #[allow(dead_code)]
    pub async fn generate_clash_subscription(&self) -> Result<String> {
        self.generate_clash_subscription_with_filter(false).await
    }

    /// Generates Clash client subscription YAML, optionally filtering proxies to healthy nodes only
    pub async fn generate_clash_subscription_with_filter(&self, healthy_only: bool) -> Result<String> {
        let mut nodes = self.nodes.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let unavailable = self.health.unavailable().await;

        if healthy_only {
            nodes.retain(|n| !unavailable.contains(&n.name));
        }

        let region_groups = crate::generator::groups::build_region_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let ip_groups = crate::generator::ip_groups::build_ip_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let level_groups = crate::generator::level_groups::build_level_groups_filtered(
            &nodes,
            &probes,
            &unavailable,
        );
        let speed_groups = self.list_speed_groups().await;

        let base_config = self.get_effective_base_config().await;

        TemplateRenderer::render_clash_subscription(
            &nodes,
            None,
            &region_groups,
            &ip_groups,
            &level_groups,
            &speed_groups,
            &unavailable,
            base_config.as_deref(),
        )
    }


    /// Current region grouping (for the Web UI regions page)
    pub async fn list_regions(&self) -> Vec<crate::generator::groups::RegionGroup> {
        let nodes = self.nodes.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let unavailable = self.health.unavailable().await;
        crate::generator::groups::build_region_groups_filtered(&nodes, &probes, &unavailable)
    }

    /// Current IP grouping (for the Web UI IP groups page)
    pub async fn list_ip_groups(&self) -> Vec<crate::generator::ip_groups::IpGroup> {
        let nodes = self.nodes.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let unavailable = self.health.unavailable().await;
        crate::generator::ip_groups::build_ip_groups_filtered(&nodes, &probes, &unavailable)
    }

    /// Current level grouping (for the Web UI level groups page)
    pub async fn list_level_groups(&self) -> Vec<crate::generator::level_groups::LevelGroup> {
        let nodes = self.nodes.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let unavailable = self.health.unavailable().await;
        crate::generator::level_groups::build_level_groups_filtered(&nodes, &probes, &unavailable)
    }

    /// Current speed grouping (for the Web UI speed groups page)
    pub async fn list_speed_groups(&self) -> Vec<crate::generator::speed_groups::SpeedGroup> {
        let nodes = self.nodes.read().await.clone();
        let probes = self.probe_results.read().await.clone();
        let traffic = self.traffic_stats.read().await.clone();
        let unavailable = self.health.unavailable().await;
        crate::generator::speed_groups::build_speed_groups_filtered(&nodes, &probes, &traffic, &unavailable)
    }

    /// Generates and writes config to disk, then triggers reload if meow-rs is running
    pub async fn apply_and_reload(&self) -> Result<(String, bool)> {
        let yaml_str = self.generate_config().await?;
        let output_path = &self.config.meow.config_output_path;
        
        if let Some(parent) = output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        std::fs::write(output_path, &yaml_str)?;
        let abs_path = std::fs::canonicalize(output_path).unwrap_or_else(|_| output_path.to_path_buf());
        let abs_path_str = abs_path.to_string_lossy().to_string();
        tracing::info!("Wrote generated meow-rs config to {}", abs_path_str);

        // 1. First attempt to reload via API using in-memory payload (zero disk re-read)
        if let Ok(()) = self.meow_reloader.reload_with_payload(&abs_path, &yaml_str).await {
            return Ok((abs_path_str, true));
        }

        // Brief retry after 250ms in case kernel was temporarily busy processing
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if let Ok(()) = self.meow_reloader.reload_with_payload(&abs_path, &yaml_str).await {
            return Ok((abs_path_str, true));
        }

        // 2. Only if API is genuinely unreachable (kernel down), auto-start core process
        tracing::info!("meow-rs API unreachable, attempting to auto-start embedded core process...");
        if self.core_manager.start_or_restart(&abs_path) {
            // Wait 1.5 seconds for core to initialize
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            Ok((abs_path_str, true))
        } else {
            Ok((abs_path_str, false))
        }
    }

    /// Reset single node's password
    pub async fn reset_node_password(&self, node_name: &str) -> Result<User> {
        let new_user = AuthGenerator::create_user_for_node(node_name);
        let stored = StoredUser {
            node_name: new_user.node_name.clone(),
            username: new_user.username.clone(),
            password: new_user.password.clone(),
            created_at: new_user.created_at,
        };
        self.db.save_user(&stored).await?;
        crate::controller::prober::clear_prober_cache();

        // Update in memory
        let mut u_lock = self.users.write().await;
        if let Some(pos) = u_lock.iter().position(|u| u.node_name == node_name) {
            u_lock[pos] = new_user.clone();
        } else {
            u_lock.push(new_user.clone());
        }

        Ok(new_user)
    }

    /// Resets all probes
    #[allow(dead_code)]
    pub async fn clear_probes(&self) {
        let mut p = self.probe_results.write().await;
        p.clear();
    }

    /// Update probe result for a node, keeping the last MAX_HISTORY probe outcomes.
    /// Persisted to SQLite so history survives restarts.
    pub async fn update_probe_result(&self, node_name: String, mut res: NodeProbeResult) {
        let merged;
        {
            let mut p = self.probe_results.write().await;
            if let Some(prev) = p.get(&node_name) {
                // If caller passed a single incremental entry (e.g. from background health sweep)
                // that doesn't already contain prev history, append it:
                if res.history.len() == 1 && (prev.history.is_empty() || prev.history.last().map(|e| e.checked_at) != res.history.first().map(|e| e.checked_at)) {
                    let mut history = prev.history.clone();
                    history.append(&mut res.history);
                    res.history = history;
                }
                if res.outbound_ip.is_none() {
                    res.outbound_ip = prev.outbound_ip.clone();
                }
                if res.location.is_none() {
                    res.location = prev.location.clone();
                }
                if res.country_code.is_none() {
                    res.country_code = prev.country_code.clone();
                }
            }
            if res.history.len() > crate::controller::prober::MAX_HISTORY {
                let excess = res.history.len() - crate::controller::prober::MAX_HISTORY;
                res.history.drain(..excess);
            }
            merged = res.clone();
            p.insert(node_name.clone(), res.clone());
        }
        // update traffic stats if probe measured speed
        if let Some(bps) = res.speed_bps {
            let stats_to_save = {
                let mut t = self.traffic_stats.write().await;
                let entry = t.entry(node_name.clone()).or_insert_with(NodeTrafficStats::default);
                entry.last_speed_bps = bps;
                entry.last_speed_formatted = Some(crate::controller::prober::format_speed_bps(bps));
                if bps > entry.peak_speed_bps {
                    entry.peak_speed_bps = bps;
                    entry.peak_speed_formatted = Some(crate::controller::prober::format_speed_bps(bps));
                }
                serde_json::to_string(entry).unwrap_or_default()
            };
            if !stats_to_save.is_empty() {
                let db = self.db.clone();
                let n_name = node_name.clone();
                tokio::spawn(async move {
                    let _ = db.save_traffic_stats(&n_name, &stats_to_save).await;
                });
            }
        }
        // persist
        if let Ok(json) = serde_json::to_string(&merged) {
            if let Err(e) = self.db.save_probe(&node_name, &json).await {
                tracing::warn!("Failed to persist probe result for {}: {}", node_name, e);
            }
        }
    }

    /// Persists all current health records (called after each full sweep).
    pub async fn persist_health(&self) {
        persist_health_rows(&self.db, self.health.all_states().await).await;
    }

    /// Polls /connections every 2 seconds to track live and historical traffic,
    /// speed, and connections per node.
    pub async fn run_connection_tracker(self: Arc<Self>) {
        let mut seen_conns: HashMap<String, (u64, u64)> = HashMap::new();
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_default();

        let api_url = format!("{}/connections", self.config.meow.api_url.trim_end_matches('/'));

        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let resp = match client.get(&api_url).send().await {
                Ok(r) if r.status().is_success() => r,
                _ => continue,
            };

            let json: serde_json::Value = match resp.json().await {
                Ok(j) => j,
                Err(_) => continue,
            };

            let conns = match json.get("connections").and_then(|c| c.as_array()) {
                Some(arr) if !arr.is_empty() => arr,
                _ => {
                    seen_conns.clear();
                    let mut stats_guard = self.traffic_stats.write().await;
                    for entry in stats_guard.values_mut() {
                        entry.active_conns = 0;
                        entry.last_speed_bps = 0;
                        entry.last_speed_formatted = None;
                    }
                    continue;
                }
            };

            let now = Utc::now();
            let mut active_node_counts: HashMap<String, usize> = HashMap::new();
            let mut new_seen: HashMap<String, (u64, u64)> = HashMap::new();

            for c in conns {
                let id = match c.get("id").and_then(|i| i.as_str()) {
                    Some(s) => s.to_string(),
                    None => continue,
                };

                let download = c.get("download").and_then(|d| d.as_u64()).unwrap_or(0);
                let upload = c.get("upload").and_then(|u| u.as_u64()).unwrap_or(0);

                let node_name = c.get("chains")
                    .and_then(|ch| ch.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.strip_prefix("node-").unwrap_or(s).to_string());

                let target_node = match node_name {
                    Some(n) if n != "DIRECT" && n != "REJECT" && n != "PROXY" && n != "AUTO" && !n.starts_with("level-") && !n.starts_with("region-") && !n.starts_with("ip-") => n,
                    _ => continue,
                };

                *active_node_counts.entry(target_node.clone()).or_insert(0) += 1;

                let (prev_dl, _prev_ul) = seen_conns.get(&id).cloned().unwrap_or((0, 0));
                let delta_dl = download.saturating_sub(prev_dl);
                let is_new = !seen_conns.contains_key(&id);
                new_seen.insert(id, (download, upload));

                let cur_speed_bps = delta_dl / 2;

                let mut stats_guard = self.traffic_stats.write().await;
                let entry = stats_guard.entry(target_node.clone()).or_insert_with(NodeTrafficStats::default);

                if is_new {
                    entry.conn_count += 1;
                }

                if delta_dl > 0 {
                    entry.total_download += delta_dl;
                    entry.total_download_formatted = crate::controller::traffic_tracker::format_bytes(entry.total_download);
                }

                if cur_speed_bps > 0 {
                    entry.last_speed_bps = cur_speed_bps;
                    entry.last_speed_formatted = Some(crate::controller::prober::format_speed_bps(cur_speed_bps));
                    if cur_speed_bps > entry.peak_speed_bps {
                        entry.peak_speed_bps = cur_speed_bps;
                        entry.peak_speed_formatted = Some(crate::controller::prober::format_speed_bps(cur_speed_bps));
                    }
                }
                entry.last_active = Some(now);
            }

            let mut stats_guard = self.traffic_stats.write().await;
            for (node, entry) in stats_guard.iter_mut() {
                let active = active_node_counts.get(node).copied().unwrap_or(0);
                entry.active_conns = active;
                if active == 0 {
                    entry.last_speed_bps = 0;
                    entry.last_speed_formatted = None;
                }
            }

            seen_conns = new_seen;
        }
    }

    /// Persists only the given nodes (called after each rolling batch).
    pub async fn persist_health_for(&self, nodes: &[String]) {
        let all = self.health.all_states().await;
        let keep: std::collections::HashSet<&String> = nodes.iter().collect();
        let rows: Vec<NodeHealth> = all
            .into_iter()
            .filter(|h| keep.contains(&h.node_name))
            .collect();
        persist_health_rows(&self.db, rows).await;
    }

    /// Restores node health records persisted before restart.
    pub async fn load_persisted_health(&self) {
        match self.db.load_all_health().await {
            Ok(rows) => self.health.restore_persisted(rows).await,
            Err(e) => tracing::warn!("Failed to load persisted health: {}", e),
        }
    }

    /// Health snapshot for the Web UI / API.
    pub async fn health_snapshot(&self) -> Vec<NodeHealth> {
        self.health.all_states().await
    }

    /// Manual take-out / restore of a node. Triggers a reload when routability changed.
    pub async fn set_node_enabled(&self, node: &str, enabled: bool) -> Result<bool> {
        let changed = self.health.set_manual_disabled(node, !enabled).await;
        if changed {
            self.persist_health().await;
            self.apply_and_reload().await?;
        }
        Ok(changed)
    }

    /// Applies a new health policy (called from the settings API) and saves to SQLite.
    pub async fn update_health_policy(&self, policy: crate::config::HealthPolicy) {
        self.health.set_policy(policy.clone()).await;
        if let Ok(json_str) = serde_json::to_string(&policy) {
            let _ = self.db.save_setting("health_policy", &json_str).await;
        }
    }

    /// Restores any previously saved health policy from SQLite so user custom intervals persist across restarts.
    pub async fn load_persisted_health_policy(&self) {
        if let Ok(Some(json_str)) = self.db.get_setting("health_policy").await {
            if let Ok(saved_policy) = serde_json::from_str::<crate::config::HealthPolicy>(&json_str) {
                tracing::info!(
                    "Restored custom health policy from database: interval={}s, down_retry={}s, timeout={}ms, fail_threshold={}",
                    saved_policy.interval_secs,
                    saved_policy.down_retry_secs,
                    saved_policy.timeout_ms,
                    saved_policy.fail_threshold
                );
                self.health.set_policy(saved_policy).await;
            }
        }
    }

    /// Cancels the manual override and re-checks a single node immediately.
    pub async fn recheck_node(&self, node: &str) -> Result<Option<NodeHealth>> {
        let found = self.nodes.read().await.iter().any(|n| n.name == node);
        if !found {
            return Ok(None);
        }
        let cred = self
            .users
            .read()
            .await
            .iter()
            .find(|u| u.node_name == node)
            .map(|u| (u.username.clone(), u.password.clone()));
        let meow = self.config.meow.clone();
        let outcome = self.health.check_node(&meow, node, cred).await;
        let (h, changed) = self.health.record(node, outcome).await;
        self.persist_health_for(&[node.to_string()]).await;
        if changed {
            self.apply_and_reload().await?;
        }
        Ok(Some(h))
    }

    /// Load persisted probe results from DB into memory (called at startup)
    pub async fn load_persisted_probes(&self) {
        match self.db.load_all_probes().await {
            Ok(rows) => {
                let mut p = self.probe_results.write().await;
                let mut count = 0;
                for (name, json) in rows {
                    if let Ok(res) = serde_json::from_str::<NodeProbeResult>(&json) {
                        p.insert(name, res);
                        count += 1;
                    }
                }
                if count > 0 {
                    tracing::info!("Restored {} persisted probe result(s) from SQLite", count);
                }
            }
            Err(e) => tracing::warn!("Failed to load persisted probes: {}", e),
        }
    }

    /// Load persisted traffic stats from DB into memory (called at startup)
    pub async fn load_persisted_traffic(&self) {
        match self.db.load_all_traffic_stats().await {
            Ok(rows) => {
                let mut t = self.traffic_stats.write().await;
                let mut count = 0;
                for (name, json) in rows {
                    if let Ok(stats) = serde_json::from_str::<NodeTrafficStats>(&json) {
                        t.insert(name, stats);
                        count += 1;
                    }
                }
                if count > 0 {
                    tracing::info!("Restored {} persisted traffic stats from SQLite", count);
                }
            }
            Err(e) => tracing::warn!("Failed to load persisted traffic stats: {}", e),
        }
    }

    /// Whether administrative authentication is enabled.
    pub fn is_auth_enabled(&self) -> bool {
        !self.config.auth.admin_password.trim().is_empty()
    }

    /// Checks if the provided password matches the admin password.
    pub fn verify_password(&self, pass: &str) -> bool {
        if !self.is_auth_enabled() {
            return true;
        }
        self.config.auth.admin_password == pass
    }

    /// Verifies if a token, secret, or password is valid.
    pub fn verify_token_or_secret(&self, token_or_pass: &str) -> bool {
        if !self.is_auth_enabled() {
            return true;
        }
        let token_or_pass = token_or_pass.trim();
        if token_or_pass.is_empty() {
            return false;
        }
        // 1. Exact admin password match
        if self.config.auth.admin_password == token_or_pass {
            return true;
        }
        // 2. Cryptographic token match
        self.auth_mgr.verify_token(token_or_pass)
    }
}

/// Serialises and bulk-writes health records (shared by full sweep + rolling batch).
async fn persist_health_rows(db: &Database, rows: Vec<NodeHealth>) {
    let encoded: Vec<(String, String)> = rows
        .into_iter()
        .filter_map(|h| serde_json::to_string(&h).ok().map(|j| (h.node_name, j)))
        .collect();
    if let Err(e) = db.save_health_many(&encoded).await {
        tracing::warn!("Failed to persist {} health record(s): {}", encoded.len(), e);
    }
}
