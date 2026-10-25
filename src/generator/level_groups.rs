#![allow(clippy::type_complexity)]

use crate::collector::{Node, User};
use crate::controller::prober::NodeProbeResult;

/// A latency tier / level group
#[derive(Debug, Clone, serde::Serialize)]
pub struct LevelGroup {
    pub level: usize,                 // 1, 2, 3, 4, 5
    pub code: String,                 // "lv1", "lv2", etc.
    pub name: String,                 // "Level 1 (极速 <200ms)"
    pub desc: String,                 // Description
    pub tag: String,                  // e.g. "绿", "黄", "紫", "红"
    pub color: String,                // CSS color token / hex
    pub min_latency_ms: Option<u64>,  // Min latency (inclusive)
    pub max_latency_ms: Option<u64>,  // Max latency (exclusive)
    pub node_count: usize,
    pub healthy_count: usize,
    pub avg_latency_ms: Option<u64>,
    pub users: Vec<LevelGroupUser>,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LevelGroupUser {
    pub username: String,     // "lv1", "lv1-fb"
    pub strategy: String,     // "auto" | "fallback"
    pub group_name: String,   // "level-lv1", "level-lv1-fb"
}

/// Builds level groups filtered by routable nodes.
pub fn build_level_groups_filtered(
    nodes: &[Node],
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
    unavailable: &std::collections::HashSet<String>,
) -> Vec<LevelGroup> {
    // Define the level brackets:
    // Level 1: < 200ms (Green)
    // Level 2: 200ms ~ 500ms (Yellow)
    // Level 3: 500ms ~ 1000ms (Purple)
    // Level 4: >= 1000ms (Red / Slow)
    // Level 5: Unprobed / Unknown (Grey)
    let tier_defs: [(usize, &str, &str, &str, &str, &str, Option<u64>, Option<u64>); 5] = [
        (1, "lv1", "Level 1", "极速响应 (<200ms)", "绿", "var(--color-success)", Some(0), Some(200)),
        (2, "lv2", "Level 2", "稳定良好 (200~500ms)", "黄", "var(--color-warning)", Some(200), Some(500)),
        (3, "lv3", "Level 3", "普通可用 (500~1000ms)", "紫", "var(--color-slow)", Some(500), Some(1000)),
        (4, "lv4", "Level 4", "长延时 (>1000ms)", "红", "var(--color-danger)", Some(1000), None),
        (5, "lv5", "Level 5", "未测速待定", "灰", "var(--text-dim)", None, None),
    ];

    let mut tier_nodes: Vec<Vec<String>> = vec![Vec::new(); tier_defs.len()];

    for node in nodes {
        if unavailable.contains(&node.name) {
            continue;
        }

        let probe = probes_map.get(&node.name);
        let lat = probe.and_then(|p| p.latency_ms);

        match lat {
            Some(l) if l < 200 => tier_nodes[0].push(node.name.clone()),
            Some(l) if l < 500 => tier_nodes[1].push(node.name.clone()),
            Some(l) if l < 1000 => tier_nodes[2].push(node.name.clone()),
            Some(_) => tier_nodes[3].push(node.name.clone()),
            None => tier_nodes[4].push(node.name.clone()),
        }
    }

    let mut groups = Vec::new();

    for (idx, (level, code, title, desc, tag, color, min_lat, max_lat)) in tier_defs.into_iter().enumerate() {
        let mut node_names = tier_nodes[idx].clone();
        let mut healthy = 0usize;
        let mut lat_sum = 0u64;
        let mut lat_cnt = 0u64;

        for n in &node_names {
            if let Some(p) = probes_map.get(n) {
                if p.error.is_none() {
                    healthy += 1;
                    if let Some(l) = p.latency_ms {
                        lat_sum += l;
                        lat_cnt += 1;
                    }
                }
            }
        }

        // Sort nodes by latency ascending, then name
        node_names.sort_by(|a, b| {
            let la = probes_map.get(a).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            let lb = probes_map.get(b).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            la.cmp(&lb).then_with(|| a.cmp(b))
        });

        let users = vec![
            LevelGroupUser {
                username: code.to_string(),
                strategy: "auto".to_string(),
                group_name: level_group_name(code, false),
            },
            LevelGroupUser {
                username: format!("{}-fb", code),
                strategy: "fallback".to_string(),
                group_name: level_group_name(code, true),
            },
        ];

        groups.push(LevelGroup {
            level,
            code: code.to_string(),
            name: format!("{} ({})", title, desc),
            desc: desc.to_string(),
            tag: tag.to_string(),
            color: color.to_string(),
            min_latency_ms: min_lat,
            max_latency_ms: max_lat,
            node_count: node_names.len(),
            healthy_count: healthy,
            avg_latency_ms: lat_sum.checked_div(lat_cnt),
            users,
            nodes: node_names,
        });
    }

    groups
}

pub fn level_group_name(code: &str, fallback: bool) -> String {
    let clean = code.to_lowercase();
    match (clean.as_str(), fallback) {
        ("lv1", false) => "🟢 LV1 · 极速响应 (<200ms)".to_string(),
        ("lv1", true) => "🛡️ LV1 · 极速转移 (<200ms)".to_string(),
        ("lv2", false) => "🟡 LV2 · 稳定良好 (200~500ms)".to_string(),
        ("lv2", true) => "🛡️ LV2 · 稳定转移 (200~500ms)".to_string(),
        ("lv3", false) => "🟣 LV3 · 普通可用 (500~1000ms)".to_string(),
        ("lv3", true) => "🛡️ LV3 · 普通转移 (500~1000ms)".to_string(),
        ("lv4", false) => "🔴 LV4 · 长延时节点 (>1000ms)".to_string(),
        ("lv4", true) => "🛡️ LV4 · 长延时转移 (>1000ms)".to_string(),
        (_, false) => "⚪ LV5 · 未测速待定".to_string(),
        (_, true) => "🛡️ LV5 · 待定转移".to_string(),
    }
}

/// Synthetic users to inject into authentication:
/// Supports "lv1" / "lv1-fb" and uppercase "LV1" / "LV1-FB" aliases.
pub fn level_users(groups: &[LevelGroup]) -> Vec<User> {
    let mut users = Vec::new();
    for g in groups {
        for u in &g.users {
            users.push(User {
                node_name: format!("[level]{}", g.code),
                username: u.username.clone(),
                password: u.username.clone(),
                created_at: chrono::Utc::now(),
            });
            let upper = u.username.to_uppercase();
            if upper != u.username {
                users.push(User {
                    node_name: format!("[level]{}", g.code),
                    username: upper.clone(),
                    password: upper,
                    created_at: chrono::Utc::now(),
                });
            }
        }
    }
    users
}
