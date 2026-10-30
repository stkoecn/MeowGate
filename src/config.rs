use std::path::PathBuf;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSourceConfig {
    pub id: String,
    pub name: String,
    pub source_type: String, // "auto_detect", "clash_api", "clash_dir", "clash_file"
    pub target: String,
    pub secret: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_server")]
    pub server: ServerConfig,

    #[serde(default = "default_database")]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub clash: ClashConfig,

    #[serde(default)]
    pub meow: MeowConfig,

    #[serde(default)]
    pub sources: Vec<NodeSourceConfig>,

    #[serde(default = "default_auto_refresh")]
    pub auto_refresh_secs: u64,

    #[serde(default)]
    pub health: HealthPolicy,

    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    /// Web management console & API access password.
    /// If empty, authentication is disabled (local/LAN mode).
    /// If set, all Web UI and API requests must be authenticated.
    #[serde(default)]
    pub admin_password: String,

    /// Optional fixed subscription token.
    /// If empty, a permanent token is generated and persisted in SQLite.
    #[serde(default)]
    pub sub_token: String,

    /// JWT / HMAC secret for signing authentication session tokens.
    /// If empty, a persistent key is stored in SQLite so session tokens survive restarts.
    #[serde(default)]
    pub jwt_secret: String,
}

/// Active node health-checking / lifecycle policy (see controller::health_monitor).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthPolicy {
    /// Enable the background health monitor + node lifecycle management.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Full sweep interval in seconds.
    #[serde(default = "default_health_interval")]
    pub interval_secs: u64,
    /// Per-probe timeout in milliseconds.
    #[serde(default = "default_health_timeout")]
    pub timeout_ms: u64,
    /// Consecutive failures before a node is taken out of all groups.
    #[serde(default = "default_fail_threshold")]
    pub fail_threshold: u32,
    /// Consecutive successes before a DOWN node is restored.
    #[serde(default = "default_recover_threshold")]
    pub recover_threshold: u32,
    /// Minimum seconds between re-checks of a DOWN node (backoff).
    #[serde(default = "default_down_retry")]
    pub down_retry_secs: u64,
    /// Max concurrent probes.
    #[serde(default = "default_health_concurrency")]
    pub concurrency: usize,
    /// URL used for the delay probe.
    #[serde(default = "default_test_url")]
    pub test_url: String,
}

fn default_health_interval() -> u64 { 300 }
fn default_health_timeout() -> u64 { 2500 }
fn default_fail_threshold() -> u32 { 2 }
fn default_recover_threshold() -> u32 { 1 }
fn default_down_retry() -> u64 { 600 }
fn default_health_concurrency() -> usize { 32 }
fn default_test_url() -> String { "https://cp.cloudflare.com/generate_204".to_string() }

impl Default for HealthPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: default_health_interval(),
            timeout_ms: default_health_timeout(),
            fail_threshold: default_fail_threshold(),
            recover_threshold: default_recover_threshold(),
            down_retry_secs: default_down_retry(),
            concurrency: default_health_concurrency(),
            test_url: default_test_url(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: String,
}

fn default_server() -> ServerConfig {
    ServerConfig {
        bind_addr: "0.0.0.0:8080".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

fn default_database() -> DatabaseConfig {
    DatabaseConfig {
        url: "sqlite://meowgate.db?mode=rwc".to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClashConfig {
    pub api_url: String,
    pub api_secret: Option<String>,
    pub config_path: Option<PathBuf>,
    #[serde(default = "default_auto_detect")]
    pub auto_detect_profiles: bool,
    #[serde(default)]
    pub profiles_dir: Option<PathBuf>,
}

fn default_auto_detect() -> bool {
    true
}

impl Default for ClashConfig {
    fn default() -> Self {
        Self {
            api_url: "http://127.0.0.1:9090".to_string(),
            api_secret: None,
            config_path: None,
            auto_detect_profiles: true,
            profiles_dir: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeowConfig {
    pub api_url: String,
    pub api_secret: Option<String>,
    pub config_output_path: PathBuf,
    pub base_config_path: Option<PathBuf>,
    /// The single public gateway port (HTTP proxy + SOCKS5 + Web UI mux
    /// entrypoint). Internal kernel inbounds listen on `http_port` /
    /// `socks_port` behind it.
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,
    /// Interface the public gateway port binds to.
    #[serde(default = "default_gateway_bind")]
    pub gateway_bind: String,
    /// Internal kernel mixed inbound (HTTP + SOCKS5) on loopback, hidden behind
    /// the public gateway port.
    #[serde(default = "default_mixed_port")]
    pub mixed_port: u16,
    pub default_target: String,
}

fn default_gateway_port() -> u16 {
    9565
}

fn default_gateway_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_mixed_port() -> u16 {
    9567
}

impl MeowConfig {
    /// Address the public multiplexer should listen on.
    pub fn gateway_addr(&self) -> String {
        format!("{}:{}", self.gateway_bind, self.gateway_port)
    }
}

impl Default for MeowConfig {
    fn default() -> Self {
        Self {
            api_url: "http://127.0.0.1:9091".to_string(),
            api_secret: None,
            config_output_path: PathBuf::from("meow-config.yaml"),
            base_config_path: None,
            gateway_port: default_gateway_port(),
            gateway_bind: default_gateway_bind(),
            mixed_port: default_mixed_port(),
            default_target: "DIRECT".to_string(),
        }
    }
}

fn default_auto_refresh() -> u64 {
    0 // 0 means disabled by default
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server: default_server(),
            database: default_database(),
            clash: ClashConfig::default(),
            meow: MeowConfig::default(),
            sources: vec![
                NodeSourceConfig {
                    id: "default-auto-detect".to_string(),
                    name: "本机 Clash 进程与订阅全量探测".to_string(),
                    source_type: "auto_detect".to_string(),
                    target: "".to_string(),
                    secret: None,
                    enabled: true,
                },
                NodeSourceConfig {
                    id: "default-clash-api".to_string(),
                    name: "本机 Clash REST API".to_string(),
                    source_type: "clash_api".to_string(),
                    target: "http://127.0.0.1:9090".to_string(),
                    secret: None,
                    enabled: true,
                },
            ],
            auto_refresh_secs: 0,
            health: HealthPolicy::default(),
            auth: AuthConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_default(path: Option<&str>) -> Self {
        let config_file = path.unwrap_or("config.toml");
        let mut cfg = if std::path::Path::new(config_file).exists() {
            match std::fs::read_to_string(config_file) {
                Ok(content) => match toml::from_str(&content) {
                    Ok(c) => {
                        tracing::info!("Loaded configuration from {}", config_file);
                        c
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse {}: {}, using defaults", config_file, e);
                        AppConfig::default()
                    }
                },
                Err(e) => {
                    tracing::warn!("Failed to read {}: {}, using defaults", config_file, e);
                    AppConfig::default()
                }
            }
        } else {
            tracing::info!("Config file {} not found, generating default config.toml", config_file);
            let default_cfg = AppConfig::default();
            if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
                let _ = std::fs::write(config_file, toml_str);
            }
            default_cfg
        };

        // Override admin password from environment variable if present
        if let Ok(env_pass) = std::env::var("MEOWGATE_ADMIN_PASSWORD") {
            if !env_pass.trim().is_empty() {
                cfg.auth.admin_password = env_pass.trim().to_string();
            }
        }

        cfg
    }
}
