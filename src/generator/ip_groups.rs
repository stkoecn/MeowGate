use std::collections::BTreeMap;

use crate::collector::{Node, User};
use crate::controller::prober::NodeProbeResult;

/// An aggregated IP group of nodes sharing the same probed outbound IP
#[derive(Debug, Clone, serde::Serialize)]
pub struct IpGroup {
    pub ip: String,                   // Raw egress IP, e.g. "109.75.46.151"
    pub sanitized_ip: String,         // Egress IP with [.:-] replaced by _, e.g. "109_75_46_151"
    pub location: Option<String>,     // e.g. "埃里温, 亚美尼亚"
    pub country_code: Option<String>, // e.g. "AM"
    pub flag: String,                 // e.g. "🇦🇲"
    pub node_count: usize,
    pub healthy_count: usize,
    pub avg_latency_ms: Option<u64>,
    pub users: Vec<IpGroupUser>,
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IpGroupUser {
    pub username: String,     // e.g. "IP_109_75_46_151"
    pub strategy: String,     // "auto" | "fallback"
    pub group_name: String,   // e.g. "ip-109-75-46-151"
}

pub fn sanitize_ip_for_user(ip: &str) -> String {
    ip.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

pub fn sanitize_ip_for_group(ip: &str) -> String {
    ip.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn build_ip_groups_filtered(
    nodes: &[Node],
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
    unavailable: &std::collections::HashSet<String>,
) -> Vec<IpGroup> {
    let mut by_ip: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut ip_metadata: std::collections::HashMap<String, (Option<String>, Option<String>)> =
        std::collections::HashMap::new();

    for node in nodes {
        if unavailable.contains(&node.name) {
            continue;
        }
        if let Some(probe) = probes_map.get(&node.name) {
            if let Some(ref ip) = probe.outbound_ip {
                let trimmed = ip.trim();
                if !trimmed.is_empty() && trimmed != "127.0.0.1" && trimmed != "0.0.0.0" {
                    by_ip.entry(trimmed.to_string()).or_default().push(node.name.clone());
                    ip_metadata
                        .entry(trimmed.to_string())
                        .or_insert_with(|| (probe.country_code.clone(), probe.location.clone()));
                }
            }
        }
    }

    let mut groups = Vec::new();
    for (ip, mut node_names) in by_ip {
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

        // Sort nodes: healthy + low latency first
        node_names.sort_by(|a, b| {
            let la = probes_map.get(a).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            let lb = probes_map.get(b).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            la.cmp(&lb).then_with(|| a.cmp(b))
        });

        let sanitized_user = sanitize_ip_for_user(&ip);
        let sanitized_group = sanitize_ip_for_group(&ip);

        let users = vec![
            IpGroupUser {
                username: format!("IP_{}", sanitized_user),
                strategy: "auto".to_string(),
                group_name: format!("ip-{}", sanitized_group),
            },
            IpGroupUser {
                username: format!("IP_{}_fb", sanitized_user),
                strategy: "fallback".to_string(),
                group_name: format!("ip-{}-fb", sanitized_group),
            },
        ];

        let (country_code, location) = ip_metadata.get(&ip).cloned().unwrap_or((None, None));
        let flag = country_code
            .as_deref()
            .map(crate::collector::country::code_to_flag)
            .unwrap_or_default();

        groups.push(IpGroup {
            ip,
            sanitized_ip: sanitized_user,
            location,
            country_code,
            flag,
            node_count: node_names.len(),
            healthy_count: healthy,
            avg_latency_ms: lat_sum.checked_div(lat_cnt),
            users,
            nodes: node_names,
        });
    }

    // Sort IP groups: by node_count desc, then IP asc
    groups.sort_by(|a, b| b.node_count.cmp(&a.node_count).then_with(|| a.ip.cmp(&b.ip)));
    groups
}

pub fn ip_group_name(sanitized_ip: &str, fallback: bool) -> String {
    let clean = sanitize_ip_for_group(sanitized_ip);
    if fallback {
        format!("ip-{}-fb", clean)
    } else {
        format!("ip-{}", clean)
    }
}

/// Synthetic users to inject into authentication:
/// Supports both "IP_1_2_3_4" and lowercase "ip_1_2_3_4" aliases for convenience.
pub fn ip_users(groups: &[IpGroup]) -> Vec<User> {
    let mut users = Vec::new();
    for g in groups {
        for u in &g.users {
            // Main uppercase user: IP_1_2_3_4
            users.push(User {
                node_name: format!("[ip]{}", g.ip),
                username: u.username.clone(),
                password: u.username.clone(),
                created_at: chrono::Utc::now(),
            });
            // Lowercase alias: ip_1_2_3_4
            let lower = u.username.to_lowercase();
            if lower != u.username {
                users.push(User {
                    node_name: format!("[ip]{}", g.ip),
                    username: lower.clone(),
                    password: lower,
                    created_at: chrono::Utc::now(),
                });
            }
        }
    }
    users
}
