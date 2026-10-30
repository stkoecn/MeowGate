use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use chrono::Utc;
use meowgate::collector::Node;
use meowgate::config::AppConfig;
use meowgate::state::AppState;
use meowgate::store::db::StoredSource;
use std::time::Instant;
use tower::ServiceExt;

fn generate_realistic_base_config() -> String {
    let mut s = String::new();
    s.push_str("dns:\n  enable: true\n  ipv6: false\n  enhanced-mode: fake-ip\n  fake-ip-range: 198.18.0.1/16\n");
    s.push_str("proxy-groups:\n");
    for i in 0..50 {
        s.push_str(&format!(
            "  - name: \"Custom-Group-{:02}\"\n    type: select\n    proxies:\n      - DIRECT\n      - REJECT\n",
            i
        ));
    }
    s.push_str("rules:\n");
    for i in 0..2000 {
        s.push_str(&format!(
            "  - \"DOMAIN-SUFFIX,service-{:04}.example.com,Custom-Group-{:02}\"\n",
            i,
            i % 50
        ));
    }
    s.push_str("sniffer:\n  enable: true\n  sniff:\n    TLS:\n      ports: [443, 8443]\n");
    s
}

#[tokio::test]
async fn test_benchmark_generation_and_endpoints() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    let state = AppState::new(config).await.expect("AppState creation failed");

    // Populate 1000 nodes
    let mut nodes = Vec::with_capacity(1000);
    for i in 0..1000 {
        nodes.push(Node {
            name: format!("Node-{:04}", i),
            proxy_type: "ss".to_string(),
            server: format!("192.168.1.{}", (i % 250) + 1),
            port: 8000 + (i as u16 % 1000),
            extra: Default::default(),
        });
    }
    *state.nodes.write().await = nodes;

    // Create a base config file
    let tmp_dir = std::env::temp_dir();
    let base_file = tmp_dir.join(format!("meowgate_bench_base_{}.yaml", rand::random::<u64>()));
    let base_content = generate_realistic_base_config();
    std::fs::write(&base_file, &base_content).expect("write base file");

    let source = StoredSource {
        id: "bench-base-src".to_string(),
        name: "Benchmark Base".to_string(),
        source_type: "clash_file".to_string(),
        target: base_file.to_string_lossy().to_string(),
        secret: None,
        enabled: true,
        is_base: true,
        created_at: Utc::now(),
    };
    state.db.save_source(&source).await.expect("save source");

    // Warm-up / verify
    let _ = state.generate_config().await.expect("warmup gen config");

    // Benchmark generate_config
    let mut gen_config_times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _ = state.generate_config().await.expect("gen config");
        gen_config_times.push(t.elapsed());
    }
    let avg_gen_config: std::time::Duration =
        gen_config_times.iter().sum::<std::time::Duration>() / gen_config_times.len() as u32;
    println!(">>> state.generate_config() avg time: {:?}", avg_gen_config);

    // Benchmark generate_clash_subscription
    let mut gen_sub_times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let _ = state.generate_clash_subscription().await.expect("gen sub");
        gen_sub_times.push(t.elapsed());
    }
    let avg_gen_sub: std::time::Duration =
        gen_sub_times.iter().sum::<std::time::Duration>() / gen_sub_times.len() as u32;
    println!(">>> state.generate_clash_subscription() avg time: {:?}", avg_gen_sub);

    // Benchmark GET /sub and GET /api/config/preview via Axum router
    let app = Router::new()
        .route("/sub", get(meowgate::api::subscription::clash_subscription_handler))
        .route("/api/config/preview", get(meowgate::api::config::preview_config))
        .with_state(state.clone());

    let mut get_sub_times = Vec::new();
    for _ in 0..5 {
        let req = Request::builder()
            .uri("/sub")
            .body(Body::empty())
            .unwrap();
        let t = Instant::now();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        get_sub_times.push(t.elapsed());
    }
    let avg_get_sub: std::time::Duration =
        get_sub_times.iter().sum::<std::time::Duration>() / get_sub_times.len() as u32;
    println!(">>> GET /sub avg time: {:?}", avg_get_sub);

    let mut get_preview_times = Vec::new();
    for _ in 0..5 {
        let req = Request::builder()
            .uri("/api/config/preview")
            .body(Body::empty())
            .unwrap();
        let t = Instant::now();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        get_preview_times.push(t.elapsed());
    }
    let avg_get_preview: std::time::Duration =
        get_preview_times.iter().sum::<std::time::Duration>() / get_preview_times.len() as u32;
    println!(">>> GET /api/config/preview avg time: {:?}", avg_get_preview);

    let _ = std::fs::remove_file(&base_file);
}
