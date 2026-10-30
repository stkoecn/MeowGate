#![allow(clippy::needless_borrows_for_generic_args)]

use crate::collector::User;
use std::collections::HashSet;

/// An inherited proxy-group parsed from base configuration sources (where `is_base: true`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InheritedGroup {
    pub name: String,              // Original group name in base_config, e.g. "us1", "us2"
    pub group_type: String,        // "load-balance", "url-test", "fallback", "select", etc.
    pub strategy: Option<String>,  // e.g. "round-robin", "consistent-hashing" for load-balance
    pub source_name: Option<String>, // Name of the source where it came from, e.g. "vps.yaml"
    pub username: String,          // Primary username for IN-USER, e.g. "us1"
    pub password: String,          // Same as username strictly
    pub aliases: Vec<String>,      // e.g. ["g_us1"]
    pub node_count: usize,         // Total members in this group
    pub healthy_count: usize,      // Healthy/available members in this group
    pub nodes: Vec<String>,        // Member proxy names
    pub strategy_desc: String,     // e.g. "负载均衡 (round-robin)", "自动测速 (url-test)"
}

/// Sanitizes a proxy group name into a clean, safe ASCII identifier suitable for Basic Auth.
pub fn sanitize_group_username(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            s.push(c.to_ascii_lowercase());
        } else if c.is_whitespace() || c == ':' || c == '/' || c == '.' {
            s.push('_');
        }
    }
    let trimmed = s.trim_matches('_').to_string();
    if trimmed.is_empty() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        name.hash(&mut hasher);
        let h = (hasher.finish() & 0xffffff) as u32;
        format!("g_{:06x}", h)
    } else {
        trimmed
    }
}

/// Generates synthetic `User` credentials for all inherited groups (username and aliases).
pub fn inherited_users(groups: &[InheritedGroup]) -> Vec<User> {
    let mut users = Vec::new();
    let now = chrono::Utc::now();
    for g in groups {
        users.push(User {
            node_name: g.name.clone(),
            username: g.username.clone(),
            password: g.username.clone(),
            created_at: now,
        });
        for alias in &g.aliases {
            if alias != &g.username {
                users.push(User {
                    node_name: g.name.clone(),
                    username: alias.clone(),
                    password: alias.clone(),
                    created_at: now,
                });
            }
        }
    }
    users
}

/// Extracts inherited proxy groups from a base config YAML string.
pub fn extract_inherited_groups(
    base_config: Option<&str>,
    unavailable: &HashSet<String>,
    source_name: Option<&str>,
) -> Vec<InheritedGroup> {
    let mut groups = Vec::new();
    let mut seen_names = HashSet::new();

    crate::generator::template::TemplateRenderer::for_each_base_doc(base_config, |map| {
        if let Some(serde_yaml::Value::Sequence(seq)) =
            map.get(&serde_yaml::Value::String("proxy-groups".to_string()))
        {
            for g_val in seq {
                if let serde_yaml::Value::Mapping(g) = g_val {
                    let name = g
                        .get(&serde_yaml::Value::String("name".to_string()))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if name.is_empty() {
                        continue;
                    }
                    // Skip standard builtin groups like GLOBAL, DIRECT, REJECT, PASS
                    if matches!(
                        name.to_ascii_uppercase().as_str(),
                        "GLOBAL" | "DIRECT" | "REJECT" | "PASS"
                    ) {
                        continue;
                    }
                    if !seen_names.insert(name.to_string()) {
                        continue;
                    }

                    let g_type = g
                        .get(&serde_yaml::Value::String("type".to_string()))
                        .and_then(|v| v.as_str())
                        .unwrap_or("select")
                        .to_string();

                    let strategy = g
                        .get(&serde_yaml::Value::String("strategy".to_string()))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let mut node_names = Vec::new();
                    if let Some(serde_yaml::Value::Sequence(proxies_seq)) =
                        g.get(&serde_yaml::Value::String("proxies".to_string()))
                    {
                        for p in proxies_seq {
                            if let Some(p_str) = p.as_str() {
                                node_names.push(p_str.to_string());
                            }
                        }
                    }

                    let node_count = node_names.len();
                    let healthy_count = node_names
                        .iter()
                        .filter(|n| !unavailable.contains(*n))
                        .count();

                    let username = sanitize_group_username(name);
                    let mut aliases = Vec::new();
                    let g_alias = format!("g_{}", username);
                    if g_alias != username {
                        aliases.push(g_alias);
                    }
                    let name_lower = name.to_ascii_lowercase();
                    if name_lower != username && !aliases.contains(&name_lower) {
                        aliases.push(name_lower);
                    }

                    let strategy_desc = match g_type.as_str() {
                        "load-balance" => {
                            if let Some(ref st) = strategy {
                                format!("负载均衡 ({})", st)
                            } else {
                                "负载均衡 (load-balance)".to_string()
                            }
                        }
                        "url-test" => "自动测速 (url-test)".to_string(),
                        "fallback" => "故障转移 (fallback)".to_string(),
                        "select" => "手动选择 (select)".to_string(),
                        "relay" => "链式代理 (relay)".to_string(),
                        other => other.to_string(),
                    };

                    groups.push(InheritedGroup {
                        name: name.to_string(),
                        group_type: g_type,
                        strategy,
                        source_name: source_name.map(|s| s.to_string()),
                        password: username.clone(),
                        username,
                        aliases,
                        node_count,
                        healthy_count,
                        nodes: node_names,
                        strategy_desc,
                    });
                }
            }
        }
    });

    groups
}
