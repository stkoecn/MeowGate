use meowgate::collector::{Node, User};
use meowgate::generator::template::TemplateRenderer;
use chrono::Utc;

#[test]
fn test_template_generator() {
    let nodes = vec![
        Node {
            name: "HK-01".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.2.3.4".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "US-01".to_string(),
            proxy_type: "vmess".to_string(),
            server: "5.6.7.8".to_string(),
            port: 443,
            extra: Default::default(),
        },
    ];

    let users = vec![
        User {
            node_name: "HK-01".to_string(),
            username: "HK-01".to_string(),
            password: "pass1".to_string(),
            created_at: Utc::now(),
        },
        User {
            node_name: "US-01".to_string(),
            username: "US-01".to_string(),
            password: "pass2".to_string(),
            created_at: Utc::now(),
        },
    ];

    let yaml = TemplateRenderer::render(&nodes, &users, None, 9565, "DIRECT")
        .expect("Render failed");

    assert!(yaml.contains("mixed-port: 9565"));
    assert!(yaml.contains("HK-01:pass1"));
    assert!(yaml.contains("US-01:pass2"));
    assert!(yaml.contains("IN-USER,HK-01,node-HK-01"));
    assert!(yaml.contains("IN-USER,US-01,node-US-01"));
    assert!(yaml.contains("name: 🚀 节点选择") || yaml.contains("🚀 节点选择"));
    assert!(yaml.contains("MATCH,🚀 节点选择"));
}

#[test]
fn test_ip_group_generation() {
    use meowgate::controller::prober::NodeProbeResult;
    use meowgate::generator::ip_groups::build_ip_groups_filtered;
    use std::collections::{HashMap, HashSet};

    let nodes = vec![
        Node {
            name: "HK-01".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.2.3.4".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "HK-02".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.2.3.5".to_string(),
            port: 8388,
            extra: Default::default(),
        },
    ];

    let users = vec![];

    let mut probes = HashMap::new();
    probes.insert(
        "HK-01".to_string(),
        NodeProbeResult {
            outbound_ip: Some("47.242.19.215".to_string()),
            location: Some("Hong Kong".to_string()),
            country_code: Some("HK".to_string()),
            latency_ms: Some(150),
            speed_bps: None,
            speed_formatted: None,
            error: None,
            checked_at: Some(Utc::now()),
            history: vec![],
        },
    );
    probes.insert(
        "HK-02".to_string(),
        NodeProbeResult {
            outbound_ip: Some("47.242.19.215".to_string()),
            location: Some("Hong Kong".to_string()),
            country_code: Some("HK".to_string()),
            latency_ms: Some(120),
            speed_bps: None,
            speed_formatted: None,
            error: None,
            checked_at: Some(Utc::now()),
            history: vec![],
        },
    );

    let ip_groups = build_ip_groups_filtered(&nodes, &probes, &HashSet::new());
    assert_eq!(ip_groups.len(), 1);
    assert_eq!(ip_groups[0].ip, "47.242.19.215");
    assert_eq!(ip_groups[0].sanitized_ip, "47_242_19_215");
    assert_eq!(ip_groups[0].node_count, 2);
    assert_eq!(ip_groups[0].users[0].username, "IP_47_242_19_215");
    assert_eq!(ip_groups[0].users[1].username, "IP_47_242_19_215_fb");

    let yaml = TemplateRenderer::render_with_regions(
        &nodes,
        &users,
        &[],
        &ip_groups,
        &HashSet::new(),
        None,
        9565,
        "DIRECT",
    ).expect("Render failed");

    // Check auth
    assert!(yaml.contains("IP_47_242_19_215:IP_47_242_19_215"));
    assert!(yaml.contains("ip_47_242_19_215:ip_47_242_19_215"));
    // Check group
    assert!(yaml.contains("name: ip-47-242-19-215"));
    assert!(yaml.contains("name: ip-47-242-19-215-fb"));
    // Check rules
    assert!(yaml.contains("IN-USER,IP_47_242_19_215,ip-47-242-19-215"));
    assert!(yaml.contains("IN-USER,ip_47_242_19_215,ip-47-242-19-215"));
    assert!(yaml.contains("IN-USER,IP_47_242_19_215_fb,ip-47-242-19-215-fb"));
}

#[test]
fn test_level_group_generation() {
    use meowgate::controller::prober::NodeProbeResult;
    use meowgate::generator::level_groups::build_level_groups_filtered;
    use std::collections::{HashMap, HashSet};

    let nodes = vec![
        Node {
            name: "Node-Fast".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.1.1.1".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "Node-Good".to_string(),
            proxy_type: "ss".to_string(),
            server: "2.2.2.2".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "Node-Normal".to_string(),
            proxy_type: "ss".to_string(),
            server: "3.3.3.3".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "Node-Slow".to_string(),
            proxy_type: "ss".to_string(),
            server: "4.4.4.4".to_string(),
            port: 8388,
            extra: Default::default(),
        },
    ];

    let mut probes = HashMap::new();
    probes.insert(
        "Node-Fast".to_string(),
        NodeProbeResult {
            latency_ms: Some(95),
            error: None,
            ..Default::default()
        },
    );
    probes.insert(
        "Node-Good".to_string(),
        NodeProbeResult {
            latency_ms: Some(280),
            error: None,
            ..Default::default()
        },
    );
    probes.insert(
        "Node-Normal".to_string(),
        NodeProbeResult {
            latency_ms: Some(650),
            error: None,
            ..Default::default()
        },
    );
    probes.insert(
        "Node-Slow".to_string(),
        NodeProbeResult {
            latency_ms: Some(1350),
            error: None,
            ..Default::default()
        },
    );

    let level_groups = build_level_groups_filtered(&nodes, &probes, &HashSet::new());
    assert_eq!(level_groups.len(), 5);

    let lv1 = level_groups.iter().find(|g| g.code == "lv1").unwrap();
    assert_eq!(lv1.node_count, 1);
    assert_eq!(lv1.nodes[0], "Node-Fast");

    let lv2 = level_groups.iter().find(|g| g.code == "lv2").unwrap();
    assert_eq!(lv2.node_count, 1);
    assert_eq!(lv2.nodes[0], "Node-Good");

    let lv3 = level_groups.iter().find(|g| g.code == "lv3").unwrap();
    assert_eq!(lv3.node_count, 1);
    assert_eq!(lv3.nodes[0], "Node-Normal");

    let lv4 = level_groups.iter().find(|g| g.code == "lv4").unwrap();
    assert_eq!(lv4.node_count, 1);
    assert_eq!(lv4.nodes[0], "Node-Slow");

    let yaml = TemplateRenderer::render_with_all_groups_cached(
        &nodes,
        None,
        &[],
        &[],
        &[],
        &level_groups,
        &[],
        &HashSet::new(),
        None,
        9565,
        "DIRECT",
    ).expect("Render failed");

    // Check auth
    assert!(yaml.contains("lv1:lv1"));
    assert!(yaml.contains("lv1-fb:lv1-fb"));
    assert!(yaml.contains("lv2:lv2"));
    assert!(yaml.contains("lv2-fb:lv2-fb"));

    // Check proxy groups
    assert!(yaml.contains("🟢 LV1 · 极速响应 (<200ms)"));
    assert!(yaml.contains("🛡️ LV1 · 极速转移 (<200ms)"));
    assert!(yaml.contains("🟡 LV2 · 稳定良好 (200~500ms)"));
    assert!(yaml.contains("🛡️ LV2 · 稳定转移 (200~500ms)"));

    // Check rules
    assert!(yaml.contains("IN-USER,lv1,🟢 LV1 · 极速响应 (<200ms)"));
    assert!(yaml.contains("IN-USER,lv1-fb,🛡️ LV1 · 极速转移 (<200ms)"));
    assert!(yaml.contains("IN-USER,lv2,🟡 LV2 · 稳定良好 (200~500ms)"));
    assert!(yaml.contains("IN-USER,lv2-fb,🛡️ LV2 · 稳定转移 (200~500ms)"));
}

#[test]
fn test_large_scale_performance() {
    use std::time::Instant;
    use std::collections::HashSet;

    let mut nodes = Vec::with_capacity(1500);
    let mut users = Vec::with_capacity(1500);
    for i in 0..1500 {
        let name = format!("🇭🇰 香港 节点 {:04} [高速]", i);
        nodes.push(Node {
            name: name.clone(),
            proxy_type: "ss".to_string(),
            server: format!("192.168.1.{}", (i % 250) + 1),
            port: 8000 + (i as u16 % 100),
            extra: Default::default(),
        });
        users.push(User {
            node_name: name.clone(),
            username: format!("user_{:04}", i),
            password: format!("pass_{:04}", i),
            created_at: Utc::now(),
        });
    }

    // Step 1: Pre-serialize proxies (one-time cost)
    let t0 = Instant::now();
    let cached_proxies = TemplateRenderer::serialize_proxies(&nodes).expect("Serialize proxies");
    let serialize_dur = t0.elapsed();
    println!("serialize_proxies 1500 nodes took: {:?}", serialize_dur);

    // Step 2: Render full config with cached proxies (as done on health flaps)
    let t1 = Instant::now();
    let yaml = TemplateRenderer::render_with_regions_cached(
        &nodes,
        Some(&cached_proxies),
        &users,
        &[],
        &[],
        &HashSet::new(),
        None,
        9565,
        "DIRECT",
    ).expect("Render failed");
    let render_dur = t1.elapsed();
    println!("render_with_regions_cached with 1500 nodes took: {:?}", render_dur);
    println!("Generated YAML size: {} bytes", yaml.len());

    assert!(render_dur.as_millis() < 50, "Rendering must be faster than 50ms, took {:?}", render_dur);
    assert!(yaml.contains("mixed-port: 9565"));
    assert!(yaml.contains("user_1499:pass_1499"));
}

#[test]
fn test_clash_subscription_generation() {
    use std::collections::HashSet;

    let nodes = vec![
        Node {
            name: "HK-01".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.2.3.4".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "US-01".to_string(),
            proxy_type: "vmess".to_string(),
            server: "5.6.7.8".to_string(),
            port: 443,
            extra: Default::default(),
        },
    ];

    let sub_yaml = TemplateRenderer::render_clash_subscription(
        &nodes,
        None,
        &[],
        &[],
        &[],
        &[],
        &HashSet::new(),
        None,
    ).expect("Render subscription failed");

    // Client ports
    assert!(sub_yaml.contains("mixed-port: 7890"));
    assert!(sub_yaml.contains("dns:"));
    assert!(sub_yaml.contains("fake-ip"));
    // Proxies
    assert!(sub_yaml.contains("HK-01"));
    assert!(sub_yaml.contains("US-01"));
    // Strategy groups
    assert!(sub_yaml.contains("🚀 节点选择"));
    assert!(sub_yaml.contains("⚡ 自动选择"));
    assert!(sub_yaml.contains("MATCH,🚀 节点选择"));
    // Must NOT contain IN-USER or authentication
    assert!(!sub_yaml.contains("IN-USER"));
    assert!(!sub_yaml.contains("authentication:"));
}

#[test]
fn test_speed_group_generation() {
    use meowgate::controller::prober::NodeProbeResult;
    use meowgate::controller::traffic_tracker::NodeTrafficStats;
    use meowgate::generator::speed_groups::build_speed_groups_filtered;
    use std::collections::{HashMap, HashSet};

    let nodes = vec![
        Node {
            name: "Node-Fast".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.1.1.1".to_string(),
            port: 1080,
            extra: Default::default(),
        },
        Node {
            name: "Node-Med".to_string(),
            proxy_type: "ss".to_string(),
            server: "2.2.2.2".to_string(),
            port: 1080,
            extra: Default::default(),
        },
        Node {
            name: "Node-Slow".to_string(),
            proxy_type: "ss".to_string(),
            server: "3.3.3.3".to_string(),
            port: 1080,
            extra: Default::default(),
        },
    ];

    let mut probes = HashMap::new();
    probes.insert(
        "Node-Fast".to_string(),
        NodeProbeResult {
            speed_bps: Some(5_000_000), // 5 MB/s -> fast
            ..Default::default()
        },
    );
    probes.insert(
        "Node-Med".to_string(),
        NodeProbeResult {
            speed_bps: Some(800_000), // 800 KB/s -> med
            ..Default::default()
        },
    );
    probes.insert(
        "Node-Slow".to_string(),
        NodeProbeResult {
            speed_bps: Some(100_000), // 100 KB/s -> slow
            ..Default::default()
        },
    );

    let traffic = HashMap::<String, NodeTrafficStats>::new();
    let speed_groups = build_speed_groups_filtered(&nodes, &probes, &traffic, &HashSet::new());

    assert_eq!(speed_groups.len(), 4);
    let fast = speed_groups.iter().find(|sg| sg.code == "fast").unwrap();
    assert_eq!(fast.nodes[0], "Node-Fast");

    let mid = speed_groups.iter().find(|sg| sg.code == "mid").unwrap();
    assert_eq!(mid.nodes[0], "Node-Med");

    let slow = speed_groups.iter().find(|sg| sg.code == "slow").unwrap();
    assert_eq!(slow.nodes[0], "Node-Slow");

    let yaml = TemplateRenderer::render_with_all_groups_cached(
        &nodes,
        None,
        &[],
        &[],
        &[],
        &[],
        &speed_groups,
        &HashSet::new(),
        None,
        9565,
        "DIRECT",
    ).expect("Render failed");

    // Check speed users & groups in YAML
    assert!(yaml.contains("fast:fast"));
    assert!(yaml.contains("fast-fb:fast-fb"));
    assert!(yaml.contains("🚀 高速节点 (自动选优)"));
    assert!(yaml.contains("🛡️ 高速节点 (故障转移)"));
    assert!(yaml.contains("IN-USER,fast,"));
}
