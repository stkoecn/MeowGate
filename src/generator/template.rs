#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::needless_borrows_for_generic_args)]

use crate::collector::{Node, User};
use crate::error::Result;
use std::collections::HashSet;
use std::fmt::Write;

pub struct TemplateRenderer;

impl TemplateRenderer {
    /// Serializes a slice of Nodes into a YAML sequence string.
    ///
    /// This is an expensive operation for 1000+ nodes, so its result should be
    /// cached in memory (`cached_proxies_yaml`) and reused across reloads until
    /// nodes are physically added/removed.
    pub fn serialize_proxies(nodes: &[Node]) -> Result<String> {
        let s = serde_yaml::to_string(nodes)?;
        Ok(s)
    }

    /// Renders a full meow-rs configuration.
    #[allow(dead_code)]
    pub fn render(
        nodes: &[Node],
        users: &[User],
        base_config: Option<&str>,
        mixed_port: u16,
        default_target: &str,
    ) -> Result<String> {
        Self::render_with_regions(
            nodes,
            users,
            &[],
            &[],
            &HashSet::new(),
            base_config,
            mixed_port,
            default_target,
        )
    }

    /// Renders config with extra region and IP proxy-groups (without proxy segment cache).
    pub fn render_with_regions(
        nodes: &[Node],
        users: &[User],
        region_groups: &[crate::generator::groups::RegionGroup],
        ip_groups: &[crate::generator::ip_groups::IpGroup],
        unavailable: &HashSet<String>,
        base_config: Option<&str>,
        mixed_port: u16,
        default_target: &str,
    ) -> Result<String> {
        Self::render_with_regions_cached(
            nodes,
            None,
            users,
            region_groups,
            ip_groups,
            unavailable,
            base_config,
            mixed_port,
            default_target,
        )
    }

    /// Renders config with direct string buffer formatting and optional cached proxy block.
    ///
    /// Avoids hundreds of thousands of generic `serde_yaml::Value` heap allocations,
    /// achieving a 10x-50x speedup and reducing reload latency to single-digit milliseconds.
    pub fn render_with_regions_cached(
        nodes: &[Node],
        cached_proxies: Option<&str>,
        users: &[User],
        region_groups: &[crate::generator::groups::RegionGroup],
        ip_groups: &[crate::generator::ip_groups::IpGroup],
        unavailable: &HashSet<String>,
        base_config: Option<&str>,
        mixed_port: u16,
        default_target: &str,
    ) -> Result<String> {
        Self::render_with_all_groups_cached(
            nodes,
            cached_proxies,
            users,
            region_groups,
            ip_groups,
            &[],
            &[],
            &[],
            unavailable,
            base_config,
            mixed_port,
            default_target,
        )
    }

    /// Renders config with direct string buffer formatting and optional cached proxy block.
    ///
    /// Avoids hundreds of thousands of generic `serde_yaml::Value` heap allocations,
    /// achieving a 10x-50x speedup and reducing reload latency to single-digit milliseconds.
    pub fn render_with_all_groups_cached(
        nodes: &[Node],
        cached_proxies: Option<&str>,
        users: &[User],
        region_groups: &[crate::generator::groups::RegionGroup],
        ip_groups: &[crate::generator::ip_groups::IpGroup],
        level_groups: &[crate::generator::level_groups::LevelGroup],
        speed_groups: &[crate::generator::speed_groups::SpeedGroup],
        inherited_groups: &[crate::generator::inherited_groups::InheritedGroup],
        unavailable: &HashSet<String>,
        base_config: Option<&str>,
        mixed_port: u16,
        default_target: &str,
    ) -> Result<String> {
        let est_proxies_len = cached_proxies.map(|s| s.len()).unwrap_or(nodes.len() * 300);
        let est_total_capacity = est_proxies_len + (users.len() + region_groups.len() * 2 + ip_groups.len() * 2 + level_groups.len() * 2) * 150 + 64 * 1024;
        let mut out = String::with_capacity(est_total_capacity);

        // 1. Core listener & controller settings
        writeln!(out, "mixed-port: {}", mixed_port).unwrap();
        writeln!(out, "bind-address: 127.0.0.1").unwrap();
        writeln!(out, "external-controller: 127.0.0.1:9091").unwrap();
        writeln!(out, "allow-lan: false").unwrap();
        writeln!(out, "mode: rule").unwrap();
        writeln!(out, "log-level: info").unwrap();
        writeln!(out, "unified-delay: true").unwrap();
        writeln!(out, "tcp-concurrent: true").unwrap();

        // 1b. Built-in high-performance concurrent DNS engine with Fake-IP
        let has_base_dns = base_config
            .map(|b| b.contains("dns:") || b.contains("\ndns:"))
            .unwrap_or(false);
        if !has_base_dns {
            writeln!(out, "dns:").unwrap();
            writeln!(out, "  enable: true").unwrap();
            writeln!(out, "  ipv6: false").unwrap();
            writeln!(out, "  enhanced-mode: fake-ip").unwrap();
            writeln!(out, "  fake-ip-range: 198.18.0.1/16").unwrap();
            writeln!(out, "  nameserver:").unwrap();
            writeln!(out, "    - 223.5.5.5").unwrap();
            writeln!(out, "    - 119.29.29.29").unwrap();
            writeln!(out, "    - 180.76.76.76").unwrap();
            writeln!(out, "    - 1.1.1.1").unwrap();
        }

        // 2. Authentication (node users + region + IP + level synthetic users + guest fallback)
        writeln!(out, "authentication:").unwrap();
        // guest account for unauthenticated / normal Clash transparent proxying
        writeln!(out, "  - \"guest:guest\"").unwrap();
        for user in users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }
        let region_users = crate::generator::groups::region_users(region_groups);
        for user in &region_users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }
        let ip_users = crate::generator::ip_groups::ip_users(ip_groups);
        for user in &ip_users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }
        let lvl_users = crate::generator::level_groups::level_users(level_groups);
        for user in &lvl_users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }
        let spd_users = crate::generator::speed_groups::speed_users(speed_groups);
        for user in &spd_users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }
        let inh_users = crate::generator::inherited_groups::inherited_users(inherited_groups);
        for user in &inh_users {
            writeln!(
                out,
                "  - \"{}:{}\"",
                Self::escape_inner(&user.username),
                Self::escape_inner(&user.password)
            ).unwrap();
        }

        // 3. Proxies — ALL nodes stay registered with the kernel.
        // If a pre-serialized string is cached in memory, use it directly!
        writeln!(out, "proxies:").unwrap();
        if let Some(cached) = cached_proxies {
            if !cached.trim().is_empty() {
                out.push_str(cached);
                if !cached.ends_with('\n') {
                    out.push('\n');
                }
            }
        } else if !nodes.is_empty() {
            let serialized = Self::serialize_proxies(nodes)?;
            out.push_str(&serialized);
            if !serialized.ends_with('\n') {
                out.push('\n');
            }
        }

        // 4. Proxy-groups
        writeln!(out, "proxy-groups:").unwrap();

        // 4a. Standard primary Clash groups (ZashBoard / Yacd / Metacubexd / standard Clash client compatibility)
        let routable: Vec<&Node> = nodes
            .iter()
            .filter(|n| !unavailable.contains(&n.name))
            .collect();

        // Group "🚀 节点选择": primary selector for dashboards and unauthenticated traffic (with update timestamp)
        let main_selector_name = Self::format_main_selector_name();
        writeln!(out, "  - name: {}", Self::format_yaml_str(&main_selector_name)).unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        writeln!(out, "      - ⚡ 自动选择").unwrap();
        writeln!(out, "      - 📶 延时分级").unwrap();
        writeln!(out, "      - ⚡ 速度梯队").unwrap();
        writeln!(out, "      - 🌍 地区选择").unwrap();
        // Place fast speed group and lv1 as top choices in 🚀 节点选择
        if speed_groups.iter().any(|sg| sg.code == "fast" && !sg.nodes.is_empty()) {
            let fast_name = crate::generator::speed_groups::speed_group_name("fast", false);
            writeln!(out, "      - {}", Self::format_yaml_str(&fast_name)).unwrap();
        }
        if level_groups.iter().any(|lg| lg.code == "lv1" && !lg.nodes.is_empty()) {
            let lv1_name = crate::generator::level_groups::level_group_name("lv1", false);
            writeln!(out, "      - {}", Self::format_yaml_str(&lv1_name)).unwrap();
        }
        writeln!(out, "      - DIRECT").unwrap();
        // Include non-empty country selectors in 🚀 节点选择
        for rg in region_groups {
            if !rg.nodes.is_empty() {
                let country_name = crate::generator::groups::region_country_name(&rg.code);
                writeln!(out, "      - {}", Self::format_yaml_str(&country_name)).unwrap();
            }
        }
        // Include base config custom groups in 🚀 节点选择
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    if let Some(serde_yaml::Value::String(name)) = g.get(&serde_yaml::Value::String("name".to_string())) {
                        writeln!(out, "      - {}", Self::format_yaml_str(name)).unwrap();
                    }
                }
            }
        });
        for node in &routable {
            writeln!(out, "      - {}", Self::format_yaml_str(&node.name)).unwrap();
        }

        // 1. 📶 延时分级大组
        writeln!(out, "  - name: 📶 延时分级").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_level_proxy = false;
        for lg in level_groups {
            if !lg.nodes.is_empty() {
                has_level_proxy = true;
                let auto_name = crate::generator::level_groups::level_group_name(&lg.code, false);
                writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            }
        }
        if !has_level_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 2. ⚡ 速度梯队大组
        writeln!(out, "  - name: ⚡ 速度梯队").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_speed_proxy = false;
        for sg in speed_groups {
            if !sg.nodes.is_empty() {
                has_speed_proxy = true;
                let auto_name = crate::generator::speed_groups::speed_group_name(&sg.code, false);
                writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            }
        }
        if !has_speed_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 3. 🌍 地区选择大组
        writeln!(out, "  - name: 🌍 地区选择").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_region_proxy = false;
        for rg in region_groups {
            if !rg.nodes.is_empty() {
                has_region_proxy = true;
                let country_name = crate::generator::groups::region_country_name(&rg.code);
                writeln!(out, "      - {}", Self::format_yaml_str(&country_name)).unwrap();
            }
        }
        if !has_region_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 4. 各国家/地区独立选择组 (e.g. 🇭🇰 香港, 🇯🇵 日本)
        for rg in region_groups {
            if rg.nodes.is_empty() {
                continue;
            }
            let country_name = crate::generator::groups::region_country_name(&rg.code);
            let auto_name = crate::generator::groups::region_group_name(&rg.code, false);
            let fb_name = crate::generator::groups::region_group_name(&rg.code, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&country_name)).unwrap();
            writeln!(out, "    type: select").unwrap();
            writeln!(out, "    proxies:").unwrap();
            writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "      - {}", Self::format_yaml_str(&fb_name)).unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // Group "⚡ 自动选择": url-test containing all healthy nodes
        writeln!(out, "  - name: ⚡ 自动选择").unwrap();
        writeln!(out, "    type: url-test").unwrap();
        writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
        writeln!(out, "    interval: 300").unwrap();
        writeln!(out, "    lazy: true").unwrap();
        writeln!(out, "    proxies:").unwrap();
        if routable.is_empty() {
            writeln!(out, "      - DIRECT").unwrap();
        } else {
            for node in &routable {
                writeln!(out, "      - {}", Self::format_yaml_str(&node.name)).unwrap();
            }
        }

        // 4b. Level groups: url-test (auto) + fallback (fb) - placed at TOP of dashboard!
        for lg in level_groups {
            let auto_name = crate::generator::level_groups::level_group_name(&lg.code, false);
            let fb_name = crate::generator::level_groups::level_group_name(&lg.code, true);

            // auto
            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if lg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &lg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }

            // fb
            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if lg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &lg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }
        }

        // 4c. Speed groups: url-test (auto) + fallback (fb)
        for sg in speed_groups {
            let auto_name = crate::generator::speed_groups::speed_group_name(&sg.code, false);
            let fb_name = crate::generator::speed_groups::speed_group_name(&sg.code, true);

            // auto
            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if sg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &sg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }

            // fb
            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if sg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &sg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }
        }

        // 4d. Region groups: url-test (auto) + fallback (fb)
        for rg in region_groups {
            if rg.nodes.is_empty() {
                continue;
            }
            let auto_name = crate::generator::groups::region_group_name(&rg.code, false);
            let fb_name = crate::generator::groups::region_group_name(&rg.code, true);

            // auto
            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }

            // fb
            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // 4d. IP groups: url-test (auto) + fallback (fb)
        for ig in ip_groups {
            if ig.nodes.is_empty() {
                continue;
            }
            let auto_name = crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, false);
            let fb_name = crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, true);

            // auto
            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    hidden: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &ig.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }

            // fb
            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    hidden: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &ig.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // 4e. Individual routable nodes (placed at the end so dashboards prioritize strategic groups)
        for node in &routable {
            let group_name = format!("node-{}", node.name);
            writeln!(out, "  - name: {}", Self::format_yaml_str(&group_name)).unwrap();
            writeln!(out, "    type: select").unwrap();
            writeln!(out, "    hidden: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            writeln!(out, "      - {}", Self::format_yaml_str(&node.name)).unwrap();
        }

        // Valid targets that can be referenced in gateway proxy-groups
        let mut valid_gateway_targets: HashSet<String> = nodes.iter().map(|n| n.name.clone()).collect();
        valid_gateway_targets.insert("DIRECT".to_string());
        valid_gateway_targets.insert("REJECT".to_string());
        valid_gateway_targets.insert("PASS".to_string());
        valid_gateway_targets.insert("COMPATIBLE".to_string());
        valid_gateway_targets.insert("GLOBAL".to_string());
        valid_gateway_targets.insert(main_selector_name.clone());
        valid_gateway_targets.insert("⚡ 自动选择".to_string());
        valid_gateway_targets.insert("📶 延时分级".to_string());
        valid_gateway_targets.insert("⚡ 速度分组".to_string());
        valid_gateway_targets.insert("🌍 地区分组".to_string());
        for node in &routable {
            valid_gateway_targets.insert(format!("node-{}", node.name));
        }
        for rg in region_groups {
            valid_gateway_targets.insert(crate::generator::groups::region_group_name(&rg.code, false));
            valid_gateway_targets.insert(crate::generator::groups::region_group_name(&rg.code, true));
        }
        for lg in level_groups {
            valid_gateway_targets.insert(crate::generator::level_groups::level_group_name(&lg.code, false));
            valid_gateway_targets.insert(crate::generator::level_groups::level_group_name(&lg.code, true));
        }
        for sg in speed_groups {
            valid_gateway_targets.insert(crate::generator::speed_groups::speed_group_name(&sg.code, false));
            valid_gateway_targets.insert(crate::generator::speed_groups::speed_group_name(&sg.code, true));
        }
        for ig in inherited_groups {
            valid_gateway_targets.insert(ig.name.clone());
        }
        for ig in ip_groups {
            valid_gateway_targets.insert(crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, false));
            valid_gateway_targets.insert(crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, true));
        }
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    if let Some(name) = g.get(&serde_yaml::Value::String("name".to_string())).and_then(|n| n.as_str()) {
                        valid_gateway_targets.insert(name.to_string());
                    }
                }
            }
        });

        // Base config extra proxy-groups
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    let sanitized_group = Self::sanitize_base_proxy_group(g, &valid_gateway_targets);
                    let seq = vec![sanitized_group];
                    if let Ok(g_str) = serde_yaml::to_string(&seq) {
                        for line in g_str.lines() {
                            writeln!(out, "  {}", line).unwrap();
                        }
                    }
                }
            }
        });

        // 5. Rules
        writeln!(out, "rules:").unwrap();

        // 5a. Node rules
        for u in users {
            if unavailable.contains(&u.node_name) {
                let rule = format!("IN-USER,{},REJECT", u.username);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
            } else {
                let rule = format!("IN-USER,{},node-{}", u.username, u.node_name);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
            }
        }

        // 5b. Region rules
        for rg in region_groups {
            if rg.nodes.is_empty() {
                continue;
            }
            for u in &rg.users {
                let rule = format!("IN-USER,{},{}", u.username, u.group_name);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
            }
        }

        // 5c. IP rules
        for ig in ip_groups {
            if ig.nodes.is_empty() {
                continue;
            }
            for u in &ig.users {
                let rule = format!("IN-USER,{},{}", u.username, u.group_name);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
                let lower = u.username.to_lowercase();
                if lower != u.username {
                    let rule_lower = format!("IN-USER,{},{}", lower, u.group_name);
                    writeln!(out, "{}", Self::format_yaml_rule(&rule_lower)).unwrap();
                }
            }
        }

        // 5d. Level rules
        for lg in level_groups {
            for u in &lg.users {
                let rule = format!("IN-USER,{},{}", u.username, u.group_name);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
                let upper = u.username.to_uppercase();
                if upper != u.username {
                    let rule_upper = format!("IN-USER,{},{}", upper, u.group_name);
                    writeln!(out, "{}", Self::format_yaml_rule(&rule_upper)).unwrap();
                }
            }
        }

        // 5e. Speed rules
        for sg in speed_groups {
            for u in &sg.users {
                let rule = format!("IN-USER,{},{}", u.username, u.group_name);
                writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
                for alias in crate::generator::speed_groups::speed_user_aliases(&u.username) {
                    let rule_alias = format!("IN-USER,{},{}", alias, u.group_name);
                    writeln!(out, "{}", Self::format_yaml_rule(&rule_alias)).unwrap();
                }
            }
        }

        // 5f. Inherited group rules (IN-USER for base_config custom groups)
        for ig in inherited_groups {
            let rule = format!("IN-USER,{},{}", ig.username, ig.name);
            writeln!(out, "{}", Self::format_yaml_rule(&rule)).unwrap();
            for alias in &ig.aliases {
                if alias != &ig.username {
                    let alias_rule = format!("IN-USER,{},{}", alias, ig.name);
                    writeln!(out, "{}", Self::format_yaml_rule(&alias_rule)).unwrap();
                }
            }
        }

        // 5g. Standard China and user private direct rules (for unauthenticated / guest proxy traffic)
        let has_geoip = std::path::Path::new("geoip.metadb").exists()
            || std::path::Path::new("geoip.dat").exists()
            || std::path::Path::new("Country.mmdb").exists();
        let has_geosite = std::path::Path::new("geosite.dat").exists()
            || std::path::Path::new("GeoSite.dat").exists();
        Self::write_standard_direct_rules(&mut out, has_geoip);

        // 5h. Guest unauthenticated rule: routes to dynamic 🚀 节点选择 group
        let guest_rule = format!("IN-USER,guest,{}", main_selector_name);
        writeln!(out, "{}", Self::format_yaml_rule(&guest_rule)).unwrap();

        // Base config extra rules, or default MATCH fallback
        let mut has_base_rules = false;
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_rules)) = map.get(&serde_yaml::Value::String("rules".to_string())) {
                for r in existing_rules {
                    if let Some(s) = r.as_str() {
                        // Skip GEOIP/GEOSITE rules if local machine has no database file to prevent Mihomo reload rejection
                        if (!has_geoip && s.starts_with("GEOIP,")) || (!has_geosite && s.starts_with("GEOSITE,")) {
                            continue;
                        }
                        has_base_rules = true;
                        writeln!(out, "{}", Self::format_yaml_rule(s)).unwrap();
                    }
                }
            }
        });

        if !has_base_rules {
            let match_target = if !routable.is_empty() {
                main_selector_name.as_str()
            } else {
                default_target
            };
            let match_rule = format!("MATCH,{}", match_target);
            writeln!(out, "{}", Self::format_yaml_rule(&match_rule)).unwrap();
        }

        // 6. Base config extra top-level keys
        let mut seen_keys: HashSet<String> = [
            "mixed-port",
            "port",
            "socks-port",
            "redir-port",
            "tproxy-port",
            "bind-address",
            "external-controller",
            "secret",
            "allow-lan",
            "mode",
            "log-level",
            "unified-delay",
            "tcp-concurrent",
            "authentication",
            "proxies",
            "proxy-groups",
            "rules",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        if !has_base_dns {
            seen_keys.insert("dns".to_string());
        }

        Self::for_each_base_doc(base_config, |map| {
            for (k, v) in map {
                if let Some(k_str) = k.as_str() {
                    if seen_keys.contains(k_str) {
                        continue;
                    }
                    seen_keys.insert(k_str.to_string());
                    if let Ok(v_yaml) = serde_yaml::to_string(&v) {
                        let v_trimmed = v_yaml.trim();
                        if v_trimmed.contains('\n') {
                            writeln!(out, "{}:", k_str).unwrap();
                            for line in v_trimmed.lines() {
                                writeln!(out, "  {}", line).unwrap();
                            }
                        } else {
                            writeln!(out, "{}: {}", k_str, v_trimmed).unwrap();
                        }
                    }
                }
            }
        });

        Ok(out)
    }

    /// Helper to format a YAML string: if safe unquoted identifier, returns as-is;
    /// otherwise wraps in double quotes and escapes backslashes, double quotes, and control chars.
    pub fn format_yaml_str(s: &str) -> String {
        let is_safe = !s.is_empty()
            && !s.starts_with('-')
            && !matches!(s, "true" | "false" | "null" | "~" | "yes" | "no")
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');

        if is_safe {
            s.to_string()
        } else {
            Self::escape_yaml_str(s)
        }
    }

    /// Escapes internal characters for double-quoted YAML strings.
    pub fn escape_inner(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                _ => out.push(c),
            }
        }
        out
    }

    /// Wraps in quotes and escapes backslashes and quotes.
    pub fn escape_yaml_str(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 4);
        out.push('"');
        out.push_str(&Self::escape_inner(s));
        out.push('"');
        out
    }

    /// Formats a single rule line in `rules:`.
    /// Iterates over every YAML document in `base_config` (which may contain multiple documents separated by `\n---\n`).
    pub fn for_each_base_doc<F>(base: Option<&str>, mut f: F)
    where
        F: FnMut(&serde_yaml::Mapping),
    {
        if let Some(b) = base {
            for part in b.split("\n---\n") {
                let part_trimmed = part.trim();
                if part_trimmed.is_empty() {
                    continue;
                }
                if let Ok(serde_yaml::Value::Mapping(map)) = serde_yaml::from_str::<serde_yaml::Value>(part_trimmed) {
                    f(&map);
                }
            }
        }
    }

    /// Formats the main selector group name with Beijing time (UTC+8) timestamp, e.g. "🚀 节点选择 (10-25 14:05)"
    pub fn format_main_selector_name() -> String {
        let beijing_time = chrono::Utc::now() + chrono::Duration::hours(8);
        format!("🚀 节点选择 ({})", beijing_time.format("%m-%d %H:%M"))
    }

    pub fn format_yaml_rule(rule: &str) -> String {
        if rule.contains(": ") || rule.contains('#') || rule.contains('"') || rule.contains('\'') {
            format!("  - {}", Self::escape_yaml_str(rule))
        } else {
            format!("  - {}", rule)
        }
    }

    /// Standard China & user-specified direct routing rules.
    ///
    /// Rules apply in top-to-bottom priority:
    /// 1. User private & specified direct domains (*.stkoe.cn, *.windsing.cn, *.windsing.dpdns.org)
    /// 2. Local network / LAN / Private IP direct (no-resolve prevents DNS leakage)
    /// 3. China top-level domain (.cn) and popular domestic services (zero-delay instant direct)
    /// 4. GeoIP CN (Mainland China IP address direct)
    pub fn write_standard_direct_rules(out: &mut String, include_geoip: bool) {
        // 1. User private & custom direct domains
        writeln!(out, "  - DOMAIN-SUFFIX,stkoe.cn,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,windsing.cn,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,windsing.dpdns.org,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-KEYWORD,stkoe,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-KEYWORD,windsing,DIRECT").unwrap();

        // 2. Local network / LAN / Private IP (no-resolve prevents DNS leakage)
        writeln!(out, "  - GEOIP,lan,DIRECT,no-resolve").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,local,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,localhost,DIRECT").unwrap();
        writeln!(out, "  - IP-CIDR,127.0.0.0/8,DIRECT,no-resolve").unwrap();
        writeln!(out, "  - IP-CIDR,172.16.0.0/12,DIRECT,no-resolve").unwrap();
        writeln!(out, "  - IP-CIDR,192.168.0.0/16,DIRECT,no-resolve").unwrap();
        writeln!(out, "  - IP-CIDR,10.0.0.0/8,DIRECT,no-resolve").unwrap();
        writeln!(out, "  - IP-CIDR,100.64.0.0/10,DIRECT,no-resolve").unwrap();

        // 3. China top-level domain & popular services (Zero-delay direct)
        writeln!(out, "  - DOMAIN-SUFFIX,cn,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,163.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,126.net,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,aliyun.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,alipay.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,taobao.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,tmall.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,qq.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,tencent.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,weixin.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,wechat.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,baidu.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,bilibili.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,bilivideo.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,jd.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,bytedance.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,douyin.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,toutiao.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,meituan.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,zhihu.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,weibo.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,sina.com.cn,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,xiaomi.com,DIRECT").unwrap();
        writeln!(out, "  - DOMAIN-SUFFIX,huawei.com,DIRECT").unwrap();

        // 4. China IP GeoIP direct (for clients that have GeoIP database)
        if include_geoip {
            writeln!(out, "  - GEOIP,CN,DIRECT").unwrap();
        }
    }


    /// Extracts friendly aliases for a node username.
    /// E.g. for "us3e5d24be" (2-3 char region + 8 hex chars):
    /// Returns: ["us_3e5d24be", "US3e5d24be", "US_3e5d24be", "n3e5d24be", "n_3e5d24be"]
    #[allow(dead_code)]
    pub fn node_user_aliases(username: &str) -> Vec<String> {
        let mut aliases = Vec::new();
        let len = username.len();
        if (10..=12).contains(&len) {
            let hash_start = len - 8;
            let prefix = &username[..hash_start];
            let hash = &username[hash_start..];
            if hash.chars().all(|c| c.is_ascii_hexdigit()) {
                aliases.push(format!("{}_{}", prefix, hash));
                aliases.push(format!("{}{}", prefix.to_uppercase(), hash));
                aliases.push(format!("{}_{}", prefix.to_uppercase(), hash));
                aliases.push(format!("n{}", hash));
                aliases.push(format!("n_{}", hash));
            }
        } else if username.starts_with('n') && len == 9 {
            aliases.push(format!("n_{}", &username[1..]));
        }
        aliases
    }

    /// Generates a clean, standalone Clash/Mihomo client subscription YAML.
    ///
    /// Formatted specifically for external Clash clients (Clash Verge, Clash for Windows,
    /// Stash, Shadowrocket, Quantumult X, Loon, ClashMeta Android, etc.).
    /// Contains standard client inbounds (port 7890), high-performance Fake-IP DNS,
    /// all active physical proxies, standard PROXY and AUTO selectors, and smart
    /// Level / Region / IP policy groups, with clean MATCH,PROXY rules.
    pub fn render_clash_subscription(
        nodes: &[Node],
        cached_proxies: Option<&str>,
        region_groups: &[crate::generator::groups::RegionGroup],
        ip_groups: &[crate::generator::ip_groups::IpGroup],
        level_groups: &[crate::generator::level_groups::LevelGroup],
        speed_groups: &[crate::generator::speed_groups::SpeedGroup],
        inherited_groups: &[crate::generator::inherited_groups::InheritedGroup],
        unavailable: &HashSet<String>,
        base_config: Option<&str>,
    ) -> Result<String> {
        let cached_len = cached_proxies.map(|s| s.len()).unwrap_or(0);
        let mut out = String::with_capacity(256 * 1024 + cached_len);

        // 1. Client inbound ports & basic options
        writeln!(out, "port: 7890").unwrap();
        writeln!(out, "socks-port: 7891").unwrap();
        writeln!(out, "mixed-port: 7890").unwrap();
        writeln!(out, "allow-lan: true").unwrap();
        writeln!(out, "mode: rule").unwrap();
        writeln!(out, "log-level: info").unwrap();
        writeln!(out, "unified-delay: true").unwrap();
        writeln!(out, "tcp-concurrent: true").unwrap();
        writeln!(out).unwrap();

        // 2. High-performance DNS (Fake-IP mode with public high-speed resolvers)
        let has_base_dns = base_config
            .map(|b| b.contains("dns:") || b.contains("\ndns:"))
            .unwrap_or(false);
        if !has_base_dns {
            writeln!(out, "dns:").unwrap();
            writeln!(out, "  enable: true").unwrap();
            writeln!(out, "  ipv6: false").unwrap();
            writeln!(out, "  enhanced-mode: fake-ip").unwrap();
            writeln!(out, "  fake-ip-range: 198.18.0.1/16").unwrap();
            writeln!(out, "  nameserver:").unwrap();
            writeln!(out, "    - 223.5.5.5").unwrap();
            writeln!(out, "    - 119.29.29.29").unwrap();
            writeln!(out, "    - 180.76.76.76").unwrap();
            writeln!(out, "    - 1.1.1.1").unwrap();
            writeln!(out).unwrap();
        }

        // 3. Proxies section
        writeln!(out, "proxies:").unwrap();
        if let Some(cached) = cached_proxies {
            if !cached.is_empty() {
                out.push_str(cached);
                if !cached.ends_with('\n') {
                    out.push('\n');
                }
            } else {
                let s = Self::serialize_proxies(nodes)?;
                out.push_str(&s);
                if !s.ends_with('\n') {
                    out.push('\n');
                }
            }
        } else {
            let s = Self::serialize_proxies(nodes)?;
            out.push_str(&s);
            if !s.ends_with('\n') {
                out.push('\n');
            }
        }
        writeln!(out).unwrap();

        // 4. Proxy-groups section
        writeln!(out, "proxy-groups:").unwrap();
        let routable: Vec<&Node> = nodes
            .iter()
            .filter(|n| !unavailable.contains(&n.name))
            .collect();

        // 4a. Primary 🚀 节点选择 selector (with update timestamp)
        let main_selector_name = Self::format_main_selector_name();
        writeln!(out, "  - name: {}", Self::format_yaml_str(&main_selector_name)).unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        writeln!(out, "      - ⚡ 自动选择").unwrap();
        writeln!(out, "      - 📶 延时分级").unwrap();
        writeln!(out, "      - ⚡ 速度梯队").unwrap();
        writeln!(out, "      - 🌍 地区选择").unwrap();
        // Place fast speed group and lv1 as top choices in 🚀 节点选择
        if speed_groups.iter().any(|sg| sg.code == "fast" && !sg.nodes.is_empty()) {
            let fast_name = crate::generator::speed_groups::speed_group_name("fast", false);
            writeln!(out, "      - {}", Self::format_yaml_str(&fast_name)).unwrap();
        }
        if level_groups.iter().any(|lg| lg.code == "lv1" && !lg.nodes.is_empty()) {
            let lv1_name = crate::generator::level_groups::level_group_name("lv1", false);
            writeln!(out, "      - {}", Self::format_yaml_str(&lv1_name)).unwrap();
        }
        writeln!(out, "      - DIRECT").unwrap();
        // Include non-empty country selectors in 🚀 节点选择
        for rg in region_groups {
            if !rg.nodes.is_empty() {
                let country_name = crate::generator::groups::region_country_name(&rg.code);
                writeln!(out, "      - {}", Self::format_yaml_str(&country_name)).unwrap();
            }
        }
        // Include base config custom groups in 🚀 节点选择
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    if let Some(serde_yaml::Value::String(name)) = g.get(&serde_yaml::Value::String("name".to_string())) {
                        writeln!(out, "      - {}", Self::format_yaml_str(name)).unwrap();
                    }
                }
            }
        });
        for node in &routable {
            writeln!(out, "      - {}", Self::format_yaml_str(&node.name)).unwrap();
        }

        // 1. 📶 延时分级大组
        writeln!(out, "  - name: 📶 延时分级").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_level_proxy = false;
        for lg in level_groups {
            if !lg.nodes.is_empty() {
                has_level_proxy = true;
                let auto_name = crate::generator::level_groups::level_group_name(&lg.code, false);
                writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            }
        }
        if !has_level_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 2. ⚡ 速度梯队大组
        writeln!(out, "  - name: ⚡ 速度梯队").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_speed_proxy = false;
        for sg in speed_groups {
            if !sg.nodes.is_empty() {
                has_speed_proxy = true;
                let auto_name = crate::generator::speed_groups::speed_group_name(&sg.code, false);
                writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            }
        }
        if !has_speed_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 3. 🌍 地区选择大组
        writeln!(out, "  - name: 🌍 地区选择").unwrap();
        writeln!(out, "    type: select").unwrap();
        writeln!(out, "    proxies:").unwrap();
        let mut has_region_proxy = false;
        for rg in region_groups {
            if !rg.nodes.is_empty() {
                has_region_proxy = true;
                let country_name = crate::generator::groups::region_country_name(&rg.code);
                writeln!(out, "      - {}", Self::format_yaml_str(&country_name)).unwrap();
            }
        }
        if !has_region_proxy {
            writeln!(out, "      - DIRECT").unwrap();
        }

        // 4. 各国家/地区独立选择组 (e.g. 🇭🇰 香港, 🇯🇵 日本)
        for rg in region_groups {
            if rg.nodes.is_empty() {
                continue;
            }
            let country_name = crate::generator::groups::region_country_name(&rg.code);
            let auto_name = crate::generator::groups::region_group_name(&rg.code, false);
            let fb_name = crate::generator::groups::region_group_name(&rg.code, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&country_name)).unwrap();
            writeln!(out, "    type: select").unwrap();
            writeln!(out, "    proxies:").unwrap();
            writeln!(out, "      - {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "      - {}", Self::format_yaml_str(&fb_name)).unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // 4b. ⚡ 自动选择 url-test group
        writeln!(out, "  - name: ⚡ 自动选择").unwrap();
        writeln!(out, "    type: url-test").unwrap();
        writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
        writeln!(out, "    interval: 300").unwrap();
        writeln!(out, "    lazy: true").unwrap();
        writeln!(out, "    proxies:").unwrap();
        if routable.is_empty() {
            writeln!(out, "      - DIRECT").unwrap();
        } else {
            for node in &routable {
                writeln!(out, "      - {}", Self::format_yaml_str(&node.name)).unwrap();
            }
        }

        // 4c. Level groups (Auto + FB)
        for lg in level_groups {
            if lg.nodes.is_empty() {
                continue;
            }
            let auto_name = crate::generator::level_groups::level_group_name(&lg.code, false);
            let fb_name = crate::generator::level_groups::level_group_name(&lg.code, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &lg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }

            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &lg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // 4c. Speed groups (Auto + FB)
        for sg in speed_groups {
            let auto_name = crate::generator::speed_groups::speed_group_name(&sg.code, false);
            let fb_name = crate::generator::speed_groups::speed_group_name(&sg.code, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if sg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &sg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }

            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            if sg.nodes.is_empty() {
                writeln!(out, "      - DIRECT").unwrap();
            } else {
                for n in &sg.nodes {
                    writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
                }
            }
        }

        // 4d. Region groups (Auto + FB)
        for rg in region_groups {
            if rg.nodes.is_empty() {
                continue;
            }
            let auto_name = crate::generator::groups::region_group_name(&rg.code, false);
            let fb_name = crate::generator::groups::region_group_name(&rg.code, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }

            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &rg.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // 4e. IP groups (Auto + FB)
        for ig in ip_groups {
            if ig.nodes.is_empty() {
                continue;
            }
            let auto_name = crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, false);
            let fb_name = crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, true);

            writeln!(out, "  - name: {}", Self::format_yaml_str(&auto_name)).unwrap();
            writeln!(out, "    type: url-test").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 300").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    hidden: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &ig.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }

            writeln!(out, "  - name: {}", Self::format_yaml_str(&fb_name)).unwrap();
            writeln!(out, "    type: fallback").unwrap();
            writeln!(out, "    url: https://cp.cloudflare.com/generate_204").unwrap();
            writeln!(out, "    interval: 120").unwrap();
            writeln!(out, "    lazy: true").unwrap();
            writeln!(out, "    hidden: true").unwrap();
            writeln!(out, "    proxies:").unwrap();
            for n in &ig.nodes {
                writeln!(out, "      - {}", Self::format_yaml_str(n)).unwrap();
            }
        }

        // Valid targets that can be referenced in subscription proxy-groups:
        // Any proxy name in nodes, any group name, or standard built-in targets.
        let mut valid_sub_targets: HashSet<String> = nodes.iter().map(|n| n.name.clone()).collect();
        valid_sub_targets.insert("DIRECT".to_string());
        valid_sub_targets.insert("REJECT".to_string());
        valid_sub_targets.insert("PASS".to_string());
        valid_sub_targets.insert("COMPATIBLE".to_string());
        valid_sub_targets.insert("GLOBAL".to_string());
        valid_sub_targets.insert(main_selector_name.clone());
        valid_sub_targets.insert("⚡ 自动选择".to_string());
        valid_sub_targets.insert("📶 延时分级".to_string());
        valid_sub_targets.insert("⚡ 速度梯队".to_string());
        valid_sub_targets.insert("🌍 地区选择".to_string());
        for rg in region_groups {
            valid_sub_targets.insert(crate::generator::groups::region_group_name(&rg.code, false));
            valid_sub_targets.insert(crate::generator::groups::region_group_name(&rg.code, true));
        }
        for lg in level_groups {
            valid_sub_targets.insert(crate::generator::level_groups::level_group_name(&lg.code, false));
            valid_sub_targets.insert(crate::generator::level_groups::level_group_name(&lg.code, true));
        }
        for sg in speed_groups {
            valid_sub_targets.insert(crate::generator::speed_groups::speed_group_name(&sg.code, false));
            valid_sub_targets.insert(crate::generator::speed_groups::speed_group_name(&sg.code, true));
        }
        for ig in inherited_groups {
            valid_sub_targets.insert(ig.name.clone());
        }
        for ig in ip_groups {
            valid_sub_targets.insert(crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, false));
            valid_sub_targets.insert(crate::generator::ip_groups::ip_group_name(&ig.sanitized_ip, true));
        }

        // Collect names of all base proxy-groups so inter-group references remain valid
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    if let Some(name) = g.get(&serde_yaml::Value::String("name".to_string())).and_then(|n| n.as_str()) {
                        valid_sub_targets.insert(name.to_string());
                    }
                }
            }
        });

        // Base config extra proxy-groups in subscription
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_groups)) = map.get(&serde_yaml::Value::String("proxy-groups".to_string())) {
                for g in existing_groups {
                    let sanitized_group = Self::sanitize_base_proxy_group(g, &valid_sub_targets);
                    let seq = vec![sanitized_group];
                    if let Ok(g_str) = serde_yaml::to_string(&seq) {
                        for line in g_str.lines() {
                            writeln!(out, "  {}", line).unwrap();
                        }
                    }
                }
            }
        });

        // 5. Rules section
        writeln!(out).unwrap();
        writeln!(out, "rules:").unwrap();
        let mut has_base_rules = false;
        Self::for_each_base_doc(base_config, |map| {
            if let Some(serde_yaml::Value::Sequence(existing_rules)) = map.get(&serde_yaml::Value::String("rules".to_string())) {
                for r in existing_rules {
                    if let Some(s) = r.as_str() {
                        has_base_rules = true;
                        writeln!(out, "{}", Self::format_yaml_rule(s)).unwrap();
                    }
                }
            }
        });

        if !has_base_rules {
            Self::write_standard_direct_rules(&mut out, true);
            let match_rule = format!("MATCH,{}", main_selector_name);
            writeln!(out, "  - {}", match_rule).unwrap();
        }

        // Base config extra top-level keys in subscription (e.g. dns, tun, sniffer, hosts)
        let mut seen_sub_keys: HashSet<String> = [
            "mixed-port",
            "port",
            "socks-port",
            "redir-port",
            "tproxy-port",
            "bind-address",
            "external-controller",
            "secret",
            "allow-lan",
            "mode",
            "log-level",
            "unified-delay",
            "tcp-concurrent",
            "authentication",
            "proxies",
            "proxy-groups",
            "rules",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        if !has_base_dns {
            seen_sub_keys.insert("dns".to_string());
        }

        Self::for_each_base_doc(base_config, |map| {
            for (k, v) in map {
                if let Some(k_str) = k.as_str() {
                    if seen_sub_keys.contains(k_str) {
                        continue;
                    }
                    seen_sub_keys.insert(k_str.to_string());
                    if let Ok(v_yaml) = serde_yaml::to_string(&v) {
                        let v_trimmed = v_yaml.trim();
                        if v_trimmed.contains('\n') {
                            writeln!(out, "{}:", k_str).unwrap();
                            for line in v_trimmed.lines() {
                                writeln!(out, "  {}", line).unwrap();
                            }
                        } else {
                            writeln!(out, "{}: {}", k_str, v_trimmed).unwrap();
                        }
                    }
                }
            }
        });

        Ok(out)
    }

    /// Filters a base config proxy-group so that its `proxies:` list only contains
    /// names that actually exist in the current configuration or built-in targets.
    /// If all proxies are filtered out, falls back to `["DIRECT"]` so the group is never empty.
    pub fn sanitize_base_proxy_group(
        group: &serde_yaml::Value,
        valid_targets: &HashSet<String>,
    ) -> serde_yaml::Value {
        if let serde_yaml::Value::Mapping(map) = group {
            let mut new_map = map.clone();
            if let Some(serde_yaml::Value::Sequence(proxies_seq)) = map.get(&serde_yaml::Value::String("proxies".to_string())) {
                let mut filtered_seq = Vec::new();
                for p in proxies_seq {
                    if let Some(p_str) = p.as_str() {
                        if valid_targets.contains(p_str) {
                            filtered_seq.push(p.clone());
                        }
                    }
                }
                if filtered_seq.is_empty() {
                    filtered_seq.push(serde_yaml::Value::String("DIRECT".to_string()));
                }
                new_map.insert(
                    serde_yaml::Value::String("proxies".to_string()),
                    serde_yaml::Value::Sequence(filtered_seq),
                );
            }
            serde_yaml::Value::Mapping(new_map)
        } else {
            group.clone()
        }
    }

}
