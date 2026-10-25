use crate::collector::{Node, User};
use crate::controller::prober::NodeProbeResult;
use crate::controller::traffic_tracker::NodeTrafficStats;

/// A speed tier / bandwidth group
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeedGroup {
    pub tier: usize,                 // 1 (fast), 2 (mid), 3 (slow), 4 (untested)
    pub code: String,                // "fast", "mid", "slow", "untested"
    pub name: String,                // "高速节点 (≥1 MB/s)"
    pub desc: String,                // Description
    pub tag: String,                 // e.g. "高", "中", "低", "待"
    pub color: String,               // CSS color token / hex
    pub min_speed_bps: Option<u64>,  // Min speed in bps
    pub max_speed_bps: Option<u64>,  // Max speed in bps
    pub node_count: usize,
    pub healthy_count: usize,
    pub avg_speed_bps: Option<u64>,
    pub avg_speed_formatted: Option<String>,
    pub avg_latency_ms: Option<u64>,
    pub users: Vec<SpeedGroupUser>,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpeedGroupUser {
    pub username: String,     // "fast", "fast-fb", "mid", "mid-fb", "slow", "slow-fb"
    pub strategy: String,     // "auto" | "fallback"
    pub group_name: String,   // e.g. "🚀 高速节点 (自动选优)"
}

/// Helper to get effective measured or recorded download speed of a node
pub fn get_node_effective_speed(
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
    traffic_map: &std::collections::HashMap<String, NodeTrafficStats>,
    node_name: &str,
) -> Option<u64> {
    let probe_speed = probes_map.get(node_name).and_then(|p| p.speed_bps);
    let traffic_speed = traffic_map.get(node_name).and_then(|t| {
        if t.peak_speed_bps > 0 {
            Some(t.peak_speed_bps)
        } else if t.last_speed_bps > 0 {
            Some(t.last_speed_bps)
        } else {
            None
        }
    });

    match (probe_speed, traffic_speed) {
        (Some(p), Some(t)) => Some(p.max(t)),
        (Some(p), None) => Some(p),
        (None, Some(t)) => Some(t),
        (None, None) => None,
    }
}

pub fn format_speed(bps: u64) -> String {
    if bps >= 1024 * 1024 * 1024 {
        format!("{:.1} GB/s", bps as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bps >= 1024 * 1024 {
        format!("{:.1} MB/s", bps as f64 / (1024.0 * 1024.0))
    } else if bps >= 1024 {
        format!("{:.0} KB/s", bps as f64 / 1024.0)
    } else if bps > 0 {
        format!("{} B/s", bps)
    } else {
        "-".to_string()
    }
}

/// Builds speed groups filtered by routable nodes.
pub fn build_speed_groups_filtered(
    nodes: &[Node],
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
    traffic_map: &std::collections::HashMap<String, NodeTrafficStats>,
    unavailable: &std::collections::HashSet<String>,
) -> Vec<SpeedGroup> {
    // Brackets:
    // 1. 高速 (fast): >= 1 MB/s (1024*1024 bps)
    // 2. 中速 (mid): 300 KB/s ~ 1 MB/s (300*1024 .. 1024*1024)
    // 3. 低速 (slow): < 300 KB/s (1 .. 300*1024)
    // 4. 待测 (untested): None / unmeasured
    let tier_defs: [(usize, &str, &str, &str, &str, &str, Option<u64>, Option<u64>); 4] = [
        (1, "fast", "高速节点", "高速流畅 (≥1 MB/s)", "高", "var(--color-success)", Some(1024 * 1024), None),
        (2, "mid", "中速节点", "稳定可用 (300K~1MB/s)", "中", "var(--color-warning)", Some(300 * 1024), Some(1024 * 1024)),
        (3, "slow", "低速节点", "限速受限 (<300 KB/s)", "低", "var(--color-danger)", Some(1), Some(300 * 1024)),
        (4, "untested", "未测速待定", "等待连接或测速", "待", "var(--text-dim)", None, None),
    ];

    let mut tier_nodes: Vec<Vec<String>> = vec![Vec::new(); tier_defs.len()];

    for node in nodes {
        if unavailable.contains(&node.name) {
            continue;
        }

        let speed = get_node_effective_speed(probes_map, traffic_map, &node.name);

        match speed {
            Some(s) if s >= 1024 * 1024 => tier_nodes[0].push(node.name.clone()),
            Some(s) if s >= 300 * 1024 => tier_nodes[1].push(node.name.clone()),
            Some(s) if s > 0 => tier_nodes[2].push(node.name.clone()),
            _ => tier_nodes[3].push(node.name.clone()),
        }
    }

    let mut groups = Vec::new();

    for (idx, (tier, code, title, desc, tag, color, min_speed, max_speed)) in tier_defs.into_iter().enumerate() {
        let mut node_names = tier_nodes[idx].clone();
        let mut healthy = 0usize;
        let mut speed_sum = 0u64;
        let mut speed_cnt = 0u64;
        let mut lat_sum = 0u64;
        let mut lat_cnt = 0u64;

        for n in &node_names {
            if let Some(p) = probes_map.get(n) {
                if p.error.is_none() {
                    healthy += 1;
                }
                if let Some(l) = p.latency_ms {
                    lat_sum += l;
                    lat_cnt += 1;
                }
            }
            if let Some(s) = get_node_effective_speed(probes_map, traffic_map, n) {
                speed_sum += s;
                speed_cnt += 1;
            }
        }

        // Sort nodes by speed descending (fastest first), then latency ascending
        node_names.sort_by(|a, b| {
            let sa = get_node_effective_speed(probes_map, traffic_map, a).unwrap_or(0);
            let sb = get_node_effective_speed(probes_map, traffic_map, b).unwrap_or(0);
            sb.cmp(&sa).then_with(|| {
                let la = probes_map.get(a).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
                let lb = probes_map.get(b).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
                la.cmp(&lb)
            }).then_with(|| a.cmp(b))
        });

        let users = vec![
            SpeedGroupUser {
                username: code.to_string(),
                strategy: "auto".to_string(),
                group_name: speed_group_name(code, false),
            },
            SpeedGroupUser {
                username: format!("{}-fb", code),
                strategy: "fallback".to_string(),
                group_name: speed_group_name(code, true),
            },
        ];

        let avg_speed = if speed_cnt > 0 { Some(speed_sum / speed_cnt) } else { None };

        groups.push(SpeedGroup {
            tier,
            code: code.to_string(),
            name: format!("{} ({})", title, desc),
            desc: desc.to_string(),
            tag: tag.to_string(),
            color: color.to_string(),
            min_speed_bps: min_speed,
            max_speed_bps: max_speed,
            node_count: node_names.len(),
            healthy_count: healthy,
            avg_speed_bps: avg_speed,
            avg_speed_formatted: avg_speed.map(format_speed),
            avg_latency_ms: if lat_cnt > 0 { Some(lat_sum / lat_cnt) } else { None },
            users,
            nodes: node_names,
        });
    }

    groups
}

pub fn speed_group_name(code: &str, fallback: bool) -> String {
    let clean = code.to_lowercase();
    match (clean.as_str(), fallback) {
        ("fast", false) => "🚀 高速节点 (自动选优)".to_string(),
        ("fast", true) => "🛡️ 高速节点 (故障转移)".to_string(),
        ("mid", false) => "⚡ 中速节点 (自动选优)".to_string(),
        ("mid", true) => "🛡️ 中速节点 (故障转移)".to_string(),
        ("slow", false) => "🐢 低速节点 (自动选优)".to_string(),
        ("slow", true) => "🛡️ 低速节点 (故障转移)".to_string(),
        (_, false) => "⚪ 未测速节点 (自动选优)".to_string(),
        (_, true) => "🛡️ 未测速节点 (故障转移)".to_string(),
    }
}

/// Synthetic users to inject into authentication:
/// Supports "fast" / "fast-fb", "mid" / "mid-fb", "slow" / "slow-fb", "untested" / "untested-fb"
/// and aliases ("sp_fast", "high", "FAST", etc.).
pub fn speed_users(groups: &[SpeedGroup]) -> Vec<User> {
    let mut users = Vec::new();
    for g in groups {
        for u in &g.users {
            users.push(User {
                node_name: format!("[speed]{}", g.code),
                username: u.username.clone(),
                password: u.username.clone(),
                created_at: chrono::Utc::now(),
            });
            // Register friendly aliases
            for alias in speed_user_aliases(&u.username) {
                users.push(User {
                    node_name: format!("[speed]{}", g.code),
                    username: alias.clone(),
                    password: alias,
                    created_at: chrono::Utc::now(),
                });
            }
        }
    }
    users
}

/// Returns friendly aliases for speed usernames
pub fn speed_user_aliases(username: &str) -> Vec<String> {
    let clean = username.to_lowercase();
    let mut out = Vec::new();

    let is_fb = clean.ends_with("-fb");
    let base = if is_fb { clean.trim_end_matches("-fb") } else { clean.as_str() };

    let mut base_aliases = Vec::new();
    match base {
        "fast" => {
            base_aliases.push("sp_fast".to_string());
            base_aliases.push("speed_fast".to_string());
            base_aliases.push("high".to_string());
            base_aliases.push("sp1".to_string());
        }
        "mid" => {
            base_aliases.push("sp_mid".to_string());
            base_aliases.push("speed_mid".to_string());
            base_aliases.push("medium".to_string());
            base_aliases.push("sp2".to_string());
        }
        "slow" => {
            base_aliases.push("sp_slow".to_string());
            base_aliases.push("speed_slow".to_string());
            base_aliases.push("low".to_string());
            base_aliases.push("sp3".to_string());
        }
        "untested" => {
            base_aliases.push("sp_untested".to_string());
            base_aliases.push("pending".to_string());
        }
        _ => {}
    }

    for b in base_aliases {
        let full = if is_fb { format!("{}-fb", b) } else { b };
        if full != clean {
            out.push(full);
        }
    }

    // Also support uppercase
    let upper = username.to_uppercase();
    if upper != username && !out.contains(&upper) {
        out.push(upper);
    }

    out
}
