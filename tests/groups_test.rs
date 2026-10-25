use meowgate::collector::Node;
use meowgate::config::AppConfig;
use meowgate::controller::prober::{NodeProbeResult, ProbeHistoryEntry};
use meowgate::api::level_groups::level_strategy_key;
use meowgate::api::regions::region_strategy_key;
use meowgate::api::ip_groups::ip_strategy_key;
use meowgate::generator::level_groups::build_level_groups_filtered;
use meowgate::generator::groups::build_region_groups_filtered;
use meowgate::generator::ip_groups::build_ip_groups_filtered;
use meowgate::state::AppState;
use std::collections::{HashMap, HashSet};
use chrono::Utc;

#[test]
fn test_level_strategy_keys() {
    assert_eq!(level_strategy_key("LV1", "auto"), "[level]lv1");
    assert_eq!(level_strategy_key("lv1", "auto"), "[level]lv1");
    assert_eq!(level_strategy_key("lv1", "fallback"), "[level]lv1-fb");
    assert_eq!(level_strategy_key("lv2", "fallback"), "[level]lv2-fb");
}

#[test]
fn test_region_strategy_keys() {
    assert_eq!(region_strategy_key("HK", "auto"), "[region]HK");
    assert_eq!(region_strategy_key("hk", "auto"), "[region]HK");
    assert_eq!(region_strategy_key("US", "fallback"), "[region]US-fb");
    assert_eq!(region_strategy_key("us", "fallback"), "[region]US-fb");
}

#[test]
fn test_ip_strategy_keys() {
    assert_eq!(ip_strategy_key("1_2_3_4", "auto"), "[ip]1_2_3_4");
    assert_eq!(ip_strategy_key("1_2_3_4", "fallback"), "[ip]1_2_3_4-fb");
}

#[tokio::test]
async fn test_state_probe_results_and_history() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    let state = AppState::new(config).await.expect("AppState creation failed");

    // Test inserting initial probe result
    let initial_res = NodeProbeResult {
        latency_ms: Some(120),
        speed_bps: None,
        speed_formatted: None,
        outbound_ip: Some("1.2.3.4".to_string()),
        location: Some("Hong Kong".to_string()),
        country_code: Some("HK".to_string()),
        checked_at: Some(Utc::now()),
        error: None,
        history: vec![ProbeHistoryEntry {
            ok: true,
            latency_ms: Some(120),
            speed_bps: None,
            checked_at: Utc::now(),
        }],
    };

    let key = level_strategy_key("lv1", "auto");
    state.update_probe_result(key.clone(), initial_res.clone()).await;

    // Verify stored
    {
        let p = state.probe_results.read().await;
        let entry = p.get(&key).expect("Should find [level]lv1");
        assert_eq!(entry.latency_ms, Some(120));
        assert_eq!(entry.history.len(), 1);
    }

    // Now update with single incremental probe entry
    let next_entry = ProbeHistoryEntry {
        ok: true,
        latency_ms: Some(110),
        speed_bps: None,
        checked_at: Utc::now() + chrono::Duration::seconds(5),
    };
    let second_res = NodeProbeResult {
        latency_ms: Some(110),
        speed_bps: None,
        speed_formatted: None,
        outbound_ip: None,
        location: None,
        country_code: None,
        checked_at: Some(next_entry.checked_at),
        error: None,
        history: vec![next_entry],
    };

    state.update_probe_result(key.clone(), second_res).await;

    // Verify history grew to 2 without duplicates and metadata was preserved
    {
        let p = state.probe_results.read().await;
        let entry = p.get(&key).unwrap();
        assert_eq!(entry.latency_ms, Some(110));
        assert_eq!(entry.history.len(), 2);
        assert_eq!(entry.outbound_ip.as_deref(), Some("1.2.3.4"));
        assert_eq!(entry.country_code.as_deref(), Some("HK"));
    }
}

#[tokio::test]
async fn test_groups_aggregation() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    let state = AppState::new(config).await.expect("AppState creation failed");

    let nodes = vec![
        Node {
            name: "HK-Fast".to_string(),
            proxy_type: "ss".to_string(),
            server: "1.2.3.4".to_string(),
            port: 8388,
            extra: Default::default(),
        },
        Node {
            name: "US-Mid".to_string(),
            proxy_type: "vmess".to_string(),
            server: "5.6.7.8".to_string(),
            port: 443,
            extra: Default::default(),
        },
    ];

    *state.nodes.write().await = nodes.clone();

    // Add probe results
    let mut pr_hk = NodeProbeResult::default();
    pr_hk.latency_ms = Some(80);
    pr_hk.outbound_ip = Some("103.1.2.3".to_string());
    pr_hk.country_code = Some("HK".to_string());
    pr_hk.location = Some("Hong Kong".to_string());
    state.update_probe_result("HK-Fast".to_string(), pr_hk.clone()).await;

    let mut pr_us = NodeProbeResult::default();
    pr_us.latency_ms = Some(250);
    pr_us.outbound_ip = Some("104.5.6.7".to_string());
    pr_us.country_code = Some("US".to_string());
    pr_us.location = Some("Los Angeles".to_string());
    state.update_probe_result("US-Mid".to_string(), pr_us.clone()).await;

    // Check level groups generator
    let mut probes = HashMap::new();
    probes.insert("HK-Fast".to_string(), pr_hk.clone());
    probes.insert("US-Mid".to_string(), pr_us.clone());

    let level_groups = build_level_groups_filtered(&nodes, &probes, &HashSet::new());
    assert_eq!(level_groups.len(), 5);
    let lv1 = level_groups.iter().find(|g| g.code == "lv1").unwrap();
    assert_eq!(lv1.node_count, 1);
    assert_eq!(lv1.nodes, vec!["HK-Fast"]);

    let lv2 = level_groups.iter().find(|g| g.code == "lv2").unwrap();
    assert_eq!(lv2.node_count, 1);
    assert_eq!(lv2.nodes, vec!["US-Mid"]);

    // Check regions generator
    let regions = build_region_groups_filtered(&nodes, &probes, &HashSet::new());
    assert_eq!(regions.len(), 2);
    let hk_region = regions.iter().find(|r| r.code == "HK").unwrap();
    assert_eq!(hk_region.node_count, 1);
    let us_region = regions.iter().find(|r| r.code == "US").unwrap();
    assert_eq!(us_region.node_count, 1);

    // Check IP groups generator
    let ip_groups = build_ip_groups_filtered(&nodes, &probes, &HashSet::new());
    assert_eq!(ip_groups.len(), 2);
    let hk_ip = ip_groups.iter().find(|g| g.ip == "103.1.2.3").unwrap();
    assert_eq!(hk_ip.sanitized_ip, "103_1_2_3");
    assert_eq!(hk_ip.node_count, 1);
}
