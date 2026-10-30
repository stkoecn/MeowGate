use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use chrono::Utc;
use meowgate::{
    collector::Node,
    config::AppConfig,
    state::AppState,
    store::db::StoredSource,
    web::build_app,
};
use std::sync::Arc;
use tower::ServiceExt; // for oneshot

async fn setup_test_app() -> (Arc<AppState>, axum::Router) {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    config.meow.gateway_port = 9565;
    config.meow.mixed_port = 7890;

    let state = AppState::new(config).await.expect("AppState creation failed");
    let router = build_app(state.clone());
    (state, router)
}

#[tokio::test]
async fn test_empty_lists_safety() {
    let (_state, router) = setup_test_app().await;

    // 1. /api/nodes
    let req = Request::builder().uri("/api/nodes").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. /api/regions
    let req = Request::builder().uri("/api/regions").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. /api/ip-groups
    let req = Request::builder().uri("/api/ip-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. /api/level-groups
    let req = Request::builder().uri("/api/level-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. /api/speed-groups
    let req = Request::builder().uri("/api/speed-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 6. /api/inherited-groups
    let req = Request::builder().uri("/api/inherited-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 7. /api/sources
    let req = Request::builder().uri("/api/sources").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 8. Probe non-existent region group
    let req = Request::builder()
        .method("POST")
        .uri("/api/regions/nonexistent/probe")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 9. Probe non-existent level group
    let req = Request::builder()
        .method("POST")
        .uri("/api/level-groups/nonexistent/probe")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 10. Probe non-existent speed group
    let req = Request::builder()
        .method("POST")
        .uri("/api/speed-groups/nonexistent/probe")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 11. Toggle non-existent source
    let req = Request::builder()
        .method("PATCH")
        .uri("/api/sources/src-nonexistent/toggle")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 12. Delete non-existent source
    let req = Request::builder()
        .method("DELETE")
        .uri("/api/sources/src-nonexistent")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_subscription_modes_and_mihomo_validation() {
    let (state, router) = setup_test_app().await;

    // Populate valid nodes for clash/mihomo
    let mut ss_extra = serde_json::Map::new();
    ss_extra.insert("cipher".to_string(), serde_json::Value::String("aes-256-gcm".to_string()));
    ss_extra.insert("password".to_string(), serde_json::Value::String("test-secret".to_string()));

    let n1 = Node {
        name: "🇭🇰 香港01-HK".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.1.1.1".to_string(),
        port: 8388,
        extra: ss_extra,
    };

    let mut vmess_extra = serde_json::Map::new();
    vmess_extra.insert("uuid".to_string(), serde_json::Value::String("b831381d-6324-4d53-ad4f-8cda48b30811".to_string()));
    vmess_extra.insert("alterId".to_string(), serde_json::Value::Number(0.into()));
    vmess_extra.insert("cipher".to_string(), serde_json::Value::String("auto".to_string()));

    let n2 = Node {
        name: "🇺🇸 美国01-US".to_string(),
        proxy_type: "vmess".to_string(),
        server: "2.2.2.2".to_string(),
        port: 443,
        extra: vmess_extra,
    };

    let mut trojan_extra = serde_json::Map::new();
    trojan_extra.insert("password".to_string(), serde_json::Value::String("trojan-pass".to_string()));

    let n_fail = Node {
        name: "🇯🇵 日本01-FAIL".to_string(),
        proxy_type: "trojan".to_string(),
        server: "3.3.3.3".to_string(),
        port: 443,
        extra: trojan_extra,
    };

    *state.nodes.write().await = vec![n1.clone(), n2.clone(), n_fail.clone()];
    // Mark Japan node failing
    state.health.set_manual_disabled("🇯🇵 日本01-FAIL", true).await;

    // Add base config source for inherited groups (us1..us5)
    let tmp_dir = std::env::temp_dir();
    let base_file = tmp_dir.join(format!("meowgate_test_base_{}.yaml", rand::random::<u64>()));
    let base_content = r#"
proxy-groups:
  - name: "us1"
    type: select
    proxies:
      - "🇺🇸 美国01-US"
      - DIRECT
  - name: "us2"
    type: fallback
    url: https://cp.cloudflare.com/generate_204
    interval: 300
    proxies:
      - "🇺🇸 美国01-US"
"#;
    std::fs::write(&base_file, base_content).unwrap();

    let source = StoredSource {
        id: "src-base-sub".to_string(),
        name: "US Base".to_string(),
        source_type: "clash_file".to_string(),
        target: base_file.to_string_lossy().to_string(),
        secret: None,
        enabled: true,
        is_base: true,
        created_at: Utc::now(),
    };
    state.db.save_source(&source).await.unwrap();

    // Test 1: /sub (default: mode=clash / mode=direct, all nodes)
    let req = Request::builder().uri("/sub").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let yaml_direct_all = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(yaml_direct_all.contains("🇭🇰 香港01-HK"));
    assert!(yaml_direct_all.contains("🇯🇵 日本01-FAIL")); // included when filter_healthy is not set
    assert!(yaml_direct_all.contains("port: 7890"));
    assert!(yaml_direct_all.contains("mixed-port: 7890"));

    // Test 2: /sub?filter_healthy=true (healthy only)
    let req = Request::builder().uri("/sub?filter_healthy=true").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let yaml_direct_healthy = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(yaml_direct_healthy.contains("🇭🇰 香港01-HK"));
    assert!(!yaml_direct_healthy.contains("🇯🇵 日本01-FAIL")); // excluded!

    // Test 3: /sub?mode=gateway (routes all traffic through MeowGate port 9565)
    let req = Request::builder()
        .uri("/sub?mode=gateway")
        .header(header::HOST, "127.0.0.1:9565")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let yaml_gateway = String::from_utf8(bytes.to_vec()).unwrap();

    // Verify gateway YAML structure:
    // Proxies have type: http, server: 127.0.0.1, port: 9565
    assert!(yaml_gateway.contains("type: http"));
    assert!(yaml_gateway.contains("port: 9565"));
    assert!(yaml_gateway.contains("server: \"127.0.0.1\"") || yaml_gateway.contains("server: 127.0.0.1"));
    assert!(yaml_gateway.contains("username:"));
    assert!(yaml_gateway.contains("password:"));
    assert!(yaml_gateway.contains("proxy-groups:"));
    assert!(yaml_gateway.contains("rules:"));

    // Test 4: Validation with verge-mihomo.exe if available
    let mihomo_path = "D:\\Clash Verge\\verge-mihomo.exe";
    if std::path::Path::new(mihomo_path).exists() {
        let test_file_direct = tmp_dir.join(format!("test_direct_sub_{}.yaml", rand::random::<u64>()));
        let test_file_gateway = tmp_dir.join(format!("test_gateway_sub_{}.yaml", rand::random::<u64>()));

        std::fs::write(&test_file_direct, &yaml_direct_healthy).unwrap();
        std::fs::write(&test_file_gateway, &yaml_gateway).unwrap();

        let output_direct = std::process::Command::new(mihomo_path)
            .args(["-t", "-f", test_file_direct.to_str().unwrap()])
            .output()
            .expect("run mihomo on direct sub");
        let direct_stdout = String::from_utf8_lossy(&output_direct.stdout);
        let direct_stderr = String::from_utf8_lossy(&output_direct.stderr);
        assert!(
            output_direct.status.success(),
            "mihomo -t -f failed on direct sub:\nstdout:\n{}\nstderr:\n{}",
            direct_stdout,
            direct_stderr
        );

        let output_gateway = std::process::Command::new(mihomo_path)
            .args(["-t", "-f", test_file_gateway.to_str().unwrap()])
            .output()
            .expect("run mihomo on gateway sub");
        let gateway_stdout = String::from_utf8_lossy(&output_gateway.stdout);
        let gateway_stderr = String::from_utf8_lossy(&output_gateway.stderr);
        assert!(
            output_gateway.status.success(),
            "mihomo -t -f failed on gateway sub:\nstdout:\n{}\nstderr:\n{}",
            gateway_stdout,
            gateway_stderr
        );
    }
}

#[tokio::test]
async fn test_system_proxy_toggle() {
    let (_state, router) = setup_test_app().await;

    // 1. GET /api/system-proxy
    let req = Request::builder().uri("/api/system-proxy").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. POST /api/system-proxy/toggle with JSON body
    let req = Request::builder()
        .method("POST")
        .uri("/api/system-proxy/toggle")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"enabled": false}"#))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. POST /api/system-proxy/toggle with empty body (Option<Json<...>>)
    let req = Request::builder()
        .method("POST")
        .uri("/api/system-proxy/toggle")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_source_crud_and_inheritance() {
    let (state, router) = setup_test_app().await;

    // 1. Add source (POST /api/sources)
    let tmp_dir = std::env::temp_dir();
    let source_file = tmp_dir.join(format!("test_src_{}.yaml", rand::random::<u64>()));
    std::fs::write(&source_file, "proxies: []\n").unwrap();

    let add_payload = serde_json::json!({
        "name": "Integration Test Source",
        "source_type": "clash_file",
        "target": source_file.to_string_lossy(),
        "enabled": true,
        "is_base": false
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/sources")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&add_payload).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let src_id = val["data"]["id"].as_str().unwrap().to_string();

    // 2. Toggle enabled (PATCH /api/sources/:id/toggle)
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/sources/{}/toggle", src_id))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify it is now disabled
    let s = state.db.get_source(&src_id).await.unwrap().unwrap();
    assert!(!s.enabled);

    // 3. Toggle base (PATCH /api/sources/:id/toggle-base)
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/api/sources/{}/toggle-base", src_id))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify is_base is now true
    let s = state.db.get_source(&src_id).await.unwrap().unwrap();
    assert!(s.is_base);

    // 4. Edit source (PUT /api/sources/:id)
    let edit_payload = serde_json::json!({
        "name": "Updated Source Name",
        "source_type": "clash_file",
        "target": source_file.to_string_lossy(),
        "enabled": true,
        "is_base": true
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/sources/{}", src_id))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&edit_payload).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let s = state.db.get_source(&src_id).await.unwrap().unwrap();
    assert_eq!(s.name, "Updated Source Name");

    // 5. Delete source (DELETE /api/sources/:id)
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/sources/{}", src_id))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let s = state.db.get_source(&src_id).await.unwrap();
    assert!(s.is_none());
}

#[tokio::test]
async fn test_all_6_views_data_and_probes() {
    let (state, router) = setup_test_app().await;

    // Create realistic nodes
    let mut ss_extra = serde_json::Map::new();
    ss_extra.insert("cipher".to_string(), serde_json::Value::String("aes-256-gcm".to_string()));
    ss_extra.insert("password".to_string(), serde_json::Value::String("test-secret".to_string()));

    let n1 = Node {
        name: "🇭🇰 香港01-HK".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.1.1.1".to_string(),
        port: 8388,
        extra: ss_extra.clone(),
    };

    let n2 = Node {
        name: "🇺🇸 美国01-US".to_string(),
        proxy_type: "ss".to_string(),
        server: "2.2.2.2".to_string(),
        port: 8388,
        extra: ss_extra.clone(),
    };

    *state.nodes.write().await = vec![n1.clone(), n2.clone()];
    state.sync_nodes_and_users(false).await.unwrap();

    // 1. View 1: 节点列表 (GET /api/nodes)
    let req = Request::builder().uri("/api/nodes").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["count"].as_u64().unwrap() >= 2);
    assert!(val["data"][0]["username"].as_str().is_some());
    assert!(val["data"][0]["password"].as_str().is_some());

    // 2. View 2: 地区分组 (GET /api/regions)
    let req = Request::builder().uri("/api/regions").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["data"].is_array());
    let regions = val["data"].as_array().unwrap();
    assert!(regions.iter().any(|r| r["code"] == "HK" || r["code"] == "hk"));
    assert!(regions.iter().any(|r| r["code"] == "US" || r["code"] == "us"));

    // 3. View 3: IP 分组 (GET /api/ip-groups)
    let req = Request::builder().uri("/api/ip-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["data"].is_array());

    // 4. View 4: 分级分组 (GET /api/level-groups)
    let req = Request::builder().uri("/api/level-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["data"].is_array());
    let levels = val["data"].as_array().unwrap();
    assert!(levels.iter().any(|l| l["code"] == "lv1"));

    // 5. View 5: 速度分组 (GET /api/speed-groups)
    let req = Request::builder().uri("/api/speed-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["data"].is_array());
    let speeds = val["data"].as_array().unwrap();
    assert!(speeds.iter().any(|s| s["code"] == "fast"));
    assert!(speeds.iter().any(|s| s["code"] == "untested"));

    // 6. View 6: 继承分组 (GET /api/inherited-groups)
    // Add base config
    let tmp_dir = std::env::temp_dir();
    let base_file = tmp_dir.join(format!("test_base_views_{}.yaml", rand::random::<u64>()));
    let base_content = r#"
proxy-groups:
  - name: "us1"
    type: select
    proxies:
      - "🇺🇸 美国01-US"
"#;
    std::fs::write(&base_file, base_content).unwrap();

    let source = StoredSource {
        id: "src-view6-base".to_string(),
        name: "Base Inherited".to_string(),
        source_type: "clash_file".to_string(),
        target: base_file.to_string_lossy().to_string(),
        secret: None,
        enabled: true,
        is_base: true,
        created_at: Utc::now(),
    };
    state.db.save_source(&source).await.unwrap();
    state.invalidate_source_caches().await;

    let req = Request::builder().uri("/api/inherited-groups").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["data"].is_array());
    let igs = val["data"].as_array().unwrap();
    assert!(igs.iter().any(|ig| ig["name"] == "us1"));
}

#[tokio::test]
async fn test_subscription_auth() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    config.auth.admin_password = "secret-admin-pass".to_string();

    let state = AppState::new(config).await.expect("AppState creation failed");
    let router = build_app(state.clone());

    // 1. Without credentials: /sub must return 401 Unauthorized
    let req = Request::builder().uri("/sub").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 2. With wrong token: /sub?token=wrong must return 401 Unauthorized
    let req = Request::builder().uri("/sub?token=wrong").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. With permanent sub token: /sub?token=<permanent_sub_token> must return 200 OK
    let valid_token = &state.permanent_sub_token;
    let req = Request::builder()
        .uri(format!("/sub?token={}", valid_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. With admin password as token: /sub?token=secret-admin-pass must return 200 OK
    let req = Request::builder()
        .uri("/sub?token=secret-admin-pass")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. With Bearer header: Authorization: Bearer <token> must return 200 OK
    let req = Request::builder()
        .uri("/sub")
        .header(header::AUTHORIZATION, "Bearer secret-admin-pass")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_config_preview_and_reload() {
    let (_state, router) = setup_test_app().await;

    // 1. GET /api/config/preview
    let req = Request::builder().uri("/api/config/preview").body(Body::empty()).unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 10 * 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["success"].as_bool().unwrap());
    assert!(val["config"].as_str().is_some());
    let cfg = val["config"].as_str().unwrap();
    assert!(cfg.contains("mixed-port:"));
    assert!(cfg.contains("authentication:"));
    assert!(cfg.contains("guest:guest"));

    // 2. POST /api/config/reload
    let req = Request::builder()
        .method("POST")
        .uri("/api/config/reload")
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val["success"].as_bool().unwrap());
    assert!(val["path"].as_str().is_some());
}

#[tokio::test]
async fn test_credential_routing_logic() {
    let (state, _router) = setup_test_app().await;

    // Set up nodes
    let mut ss_extra = serde_json::Map::new();
    ss_extra.insert("cipher".to_string(), serde_json::Value::String("aes-256-gcm".to_string()));
    ss_extra.insert("password".to_string(), serde_json::Value::String("test-secret".to_string()));

    let n1 = Node {
        name: "🇭🇰 香港01-HK".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.1.1.1".to_string(),
        port: 8388,
        extra: ss_extra.clone(),
    };
    *state.nodes.write().await = vec![n1.clone()];
    state.sync_nodes_and_users(false).await.unwrap();

    // Base config with us1
    let tmp_dir = std::env::temp_dir();
    let base_file = tmp_dir.join(format!("test_base_routing_{}.yaml", rand::random::<u64>()));
    let base_content = r#"
proxy-groups:
  - name: "us1"
    type: select
    proxies:
      - "🇭🇰 香港01-HK"
"#;
    std::fs::write(&base_file, base_content).unwrap();

    let source = StoredSource {
        id: "src-route-base".to_string(),
        name: "Route Base".to_string(),
        source_type: "clash_file".to_string(),
        target: base_file.to_string_lossy().to_string(),
        secret: None,
        enabled: true,
        is_base: true,
        created_at: Utc::now(),
    };
    state.db.save_source(&source).await.unwrap();
    state.invalidate_source_caches().await;

    // Generate config
    let config_yaml = state.generate_config().await.unwrap();

    // Verify all credentials and rules:
    // 1. Unauthenticated guest:guest -> 🚀 节点选择
    assert!(config_yaml.contains("- \"guest:guest\""));
    assert!(config_yaml.contains("IN-USER,guest,🚀 节点选择"));

    // 2. Region credentials: hk:hk, hk-fb:hk-fb
    assert!(config_yaml.contains("- \"hk:hk\""));
    assert!(config_yaml.contains("- \"hk-fb:hk-fb\""));

    // 3. Level credentials: lv1:lv1, lv1-fb:lv1-fb
    assert!(config_yaml.contains("- \"lv1:lv1\""));
    assert!(config_yaml.contains("- \"lv1-fb:lv1-fb\""));

    // 4. Speed credentials: fast:fast, mid:mid, slow:slow
    assert!(config_yaml.contains("- \"fast:fast\""));
    assert!(config_yaml.contains("- \"mid:mid\""));
    assert!(config_yaml.contains("- \"slow:slow\""));

    // 5. Inherited credentials: us1:us1 and g_us1:g_us1
    assert!(config_yaml.contains("- \"us1:us1\"") || config_yaml.contains("- \"g_us1:g_us1\""));
    assert!(config_yaml.contains("IN-USER,us1,us1") || config_yaml.contains("IN-USER,g_us1,us1"));

    // 6. Single node credentials: <user>:<user>
    let users = state.users.read().await;
    let node_user = &users[0];
    assert!(config_yaml.contains(&format!("- \"{}:{}\"", node_user.username, node_user.password)));
    assert!(config_yaml.contains(&format!("IN-USER,{},node-{}", node_user.username, node_user.node_name)));
}
