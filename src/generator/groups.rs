use std::collections::BTreeMap;

use crate::collector::{Node, User};
use crate::collector::country::CountryInfer;
use crate::controller::prober::NodeProbeResult;

/// An aggregated region/country group of nodes
#[derive(Debug, Clone, serde::Serialize)]
pub struct RegionGroup {
    pub code: String,        // e.g. "US"
    pub flag: String,        // e.g. "🇺"
    pub node_count: usize,
    pub healthy_count: usize,
    pub avg_latency_ms: Option<u64>,
    /// username -> proxy connect string purpose
    pub users: Vec<RegionUser>,
    pub nodes: Vec<String>,  // node names
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RegionUser {
    pub username: String,     // "US" or "US-fb"
    pub strategy: String,     // "auto" | "fallback"
    pub group_name: String,   // meow proxy-group name
}

pub const AUTO_SUFFIX: &str = "";
pub const FB_SUFFIX: &str = "-fb";

/// Build region groups from nodes + probe results.
#[allow(dead_code)]
pub fn build_region_groups(
    nodes: &[Node],
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
) -> Vec<RegionGroup> {
    build_region_groups_filtered(nodes, probes_map, &std::collections::HashSet::new())
}

/// Same as `build_region_groups` but skips nodes that are not routable
/// (health-checked DOWN or manually disabled). Those nodes are also absent from
/// the generated `proxies` list, so they must not appear in any group.
pub fn build_region_groups_filtered(
    nodes: &[Node],
    probes_map: &std::collections::HashMap<String, NodeProbeResult>,
    unavailable: &std::collections::HashSet<String>,
) -> Vec<RegionGroup> {
    let mut by_country: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for node in nodes {
        if unavailable.contains(&node.name) {
            continue;
        }
        // Prefer probe-derived country code, then fall back to name inference
        let probe = probes_map.get(&node.name);
        let code = probe
            .and_then(|p| p.country_code.clone())
            .or_else(|| CountryInfer::from_name(&node.name));

        if let Some(code) = code {
            let code = code.to_uppercase();
            if code.len() == 2 && code.chars().all(|c| c.is_ascii_alphabetic()) {
                by_country.entry(code).or_default().push(node.name.clone());
            }
        }
    }

    let mut groups = Vec::new();
    for (code, mut node_names) in by_country {
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
        // sort nodes: healthy+low latency first
        node_names.sort_by(|a, b| {
            let la = probes_map.get(a).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            let lb = probes_map.get(b).and_then(|p| p.latency_ms).unwrap_or(u64::MAX);
            la.cmp(&lb).then_with(|| a.cmp(b))
        });
        let users = vec![
            RegionUser {
                username: format!("{}{}", code.to_lowercase(), AUTO_SUFFIX),
                strategy: "auto".to_string(),
                group_name: region_group_name(&code, false),
            },
            RegionUser {
                username: format!("{}{}", code.to_lowercase(), FB_SUFFIX),
                strategy: "fallback".to_string(),
                group_name: region_group_name(&code, true),
            },
        ];
        groups.push(RegionGroup {
            flag: crate::collector::country::code_to_flag(&code),
            avg_latency_ms: if lat_cnt > 0 { Some(lat_sum / lat_cnt) } else { None },
            healthy_count: healthy,
            node_count: node_names.len(),
            code,
            users,
            nodes: node_names,
        });
    }
    groups
}

pub fn region_group_name(code: &str, fallback: bool) -> String {
    let flag = crate::collector::country::code_to_flag(code);
    let cname = crate::collector::country::country_name(code);
    if fallback {
        format!("{} {} (故障转移)", flag, cname)
    } else {
        format!("{} {} (自动选优)", flag, cname)
    }
}

/// The synthetic users that should be injected into `authentication`
/// (username == password per project policy).
pub fn region_users(groups: &[RegionGroup]) -> Vec<User> {
    let mut users = Vec::new();
    for g in groups {
        for u in &g.users {
            users.push(User {
                node_name: format!("[region]{}", g.code),
                username: u.username.clone(),
                password: u.username.clone(),
                created_at: chrono::Utc::now(),
            });
        }
    }
    users
}
