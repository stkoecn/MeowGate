use meowgate::collector::Node;
use meowgate::config::AppConfig;
use meowgate::state::AppState;
use meowgate::store::db::{Database, StoredSource};
use chrono::Utc;

#[tokio::test]
async fn test_source_lifecycle() {
    let db = Database::init("sqlite::memory:").await.expect("DB creation failed");

    let source = StoredSource {
        id: "src-test-1".to_string(),
        name: "Test Clash File".to_string(),
        source_type: "clash_file".to_string(),
        target: "C:/path/to/profile.yaml".to_string(),
        secret: None,
        enabled: true,
        is_base: false,
        created_at: Utc::now(),
    };

    // 1. Save source
    db.save_source(&source).await.expect("save_source failed");
    let fetched = db.get_source("src-test-1").await.expect("get_source failed").expect("should exist");
    assert_eq!(fetched.name, "Test Clash File");
    assert!(!fetched.is_base);

    // 1b. Toggle base inheritance
    let new_is_base = db.toggle_source_base("src-test-1").await.expect("toggle_base failed");
    assert!(new_is_base);
    let after_toggle = db.get_source("src-test-1").await.expect("get_source failed").unwrap();
    assert!(after_toggle.is_base);

    // 2. Delete source
    db.delete_source("src-test-1").await.expect("delete_source failed");
    let after_del = db.get_source("src-test-1").await.expect("get_source failed");
    assert!(after_del.is_none());

    // 3. User re-adds the source
    db.save_source(&source).await.expect("re-save source failed");
    let re_fetched = db.get_source("src-test-1").await.expect("get_source failed");
    assert!(re_fetched.is_some());
}

#[tokio::test]
async fn test_subscription_healthy_filtering() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    let state = AppState::new(config).await.expect("AppState creation failed");

    let n1 = Node {
        name: "Node-Healthy".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.1.1.1".to_string(),
        port: 1001,
        extra: Default::default(),
    };
    let n2 = Node {
        name: "Node-Failing".to_string(),
        proxy_type: "ss".to_string(),
        server: "2.2.2.2".to_string(),
        port: 1002,
        extra: Default::default(),
    };

    *state.nodes.write().await = vec![n1.clone(), n2.clone()];
    // Mark Node-Failing as disabled / unavailable
    state.health.set_manual_disabled("Node-Failing", true).await;

    // All nodes subscription
    let yaml_all = state.generate_clash_subscription_with_filter(false).await.expect("all sub");
    assert!(yaml_all.contains("Node-Healthy"));
    assert!(yaml_all.contains("Node-Failing"));

    // Healthy only subscription
    let yaml_healthy = state.generate_clash_subscription_with_filter(true).await.expect("healthy sub");
    assert!(yaml_healthy.contains("Node-Healthy"));
    assert!(!yaml_healthy.contains("Node-Failing"));
}

#[tokio::test]
async fn test_base_config_inheritance_toggle() {
    let mut config = AppConfig::default();
    config.database.url = "sqlite::memory:".to_string();
    let state = AppState::new(config).await.expect("AppState creation failed");

    // Create a temporary base config file
    let tmp_dir = std::env::temp_dir();
    let base_file = tmp_dir.join("test_meowgate_base.yaml");
    let base_content = r#"
proxy-groups:
  - name: "CUSTOM-BASE-GROUP"
    type: select
    proxies:
      - DIRECT
rules:
  - "DOMAIN-SUFFIX,custom-base-rule.com,DIRECT"
dns:
  enable: true
  fake-ip-range: 198.18.0.1/16
"#;
    std::fs::write(&base_file, base_content).expect("write base file");

    let source = StoredSource {
        id: "src-base-test".to_string(),
        name: "Custom Base Profile".to_string(),
        source_type: "clash_file".to_string(),
        target: base_file.to_string_lossy().to_string(),
        secret: None,
        enabled: true,
        is_base: false,
        created_at: Utc::now(),
    };
    state.db.save_source(&source).await.expect("save source");

    // Initially is_base = false
    let base_opt = state.get_effective_base_config().await;
    assert!(base_opt.is_none());

    // Generate config without base
    let yaml_no_base = state.generate_config().await.expect("gen config");
    assert!(!yaml_no_base.contains("CUSTOM-BASE-GROUP"));
    assert!(!yaml_no_base.contains("custom-base-rule.com"));

    // Generate subscription without base
    let sub_no_base = state.generate_clash_subscription_with_filter(false).await.expect("gen sub");
    assert!(!sub_no_base.contains("CUSTOM-BASE-GROUP"));
    assert!(!sub_no_base.contains("custom-base-rule.com"));

    // Toggle is_base = true
    state.db.toggle_source_base("src-base-test").await.expect("toggle base");

    let base_opt_active = state.get_effective_base_config().await;
    assert!(base_opt_active.is_some());
    assert!(base_opt_active.unwrap().contains("CUSTOM-BASE-GROUP"));

    // Now generate gateway config with base inheritance
    let yaml_with_base = state.generate_config().await.expect("gen config with base");
    assert!(yaml_with_base.contains("CUSTOM-BASE-GROUP"));
    assert!(yaml_with_base.contains("custom-base-rule.com"));
    assert!(yaml_with_base.contains("fake-ip-range: 198.18.0.1/16"));

    // And generate subscription with base inheritance
    let sub_with_base = state.generate_clash_subscription_with_filter(false).await.expect("gen sub with base");
    assert!(sub_with_base.contains("CUSTOM-BASE-GROUP"));
    assert!(sub_with_base.contains("custom-base-rule.com"));
    assert!(sub_with_base.contains("fake-ip-range: 198.18.0.1/16"));

    // Cleanup
    let _ = std::fs::remove_file(&base_file);
}

#[tokio::test]
async fn test_permanent_subscription_token_across_restarts() {
    let temp_dir = std::env::temp_dir();
    let db_path = temp_dir.join(format!("meowgate_test_sub_tok_{}.db", rand::random::<u64>()));
    let db_url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

    let mut config = AppConfig::default();
    config.database.url = db_url.clone();
    config.auth.admin_password = "my_admin_password_123".to_string();

    // 1. First boot: generates and saves permanent_sub_token in SQLite
    let state1 = AppState::new(config.clone()).await.expect("AppState creation 1");
    let token1 = state1.permanent_sub_token.clone();
    assert!(!token1.is_empty());
    assert_eq!(token1.len(), 32); // 16 random bytes in hex

    // Verify token can authorize subscription
    assert!(state1.verify_token_or_secret(&token1));
    assert!(state1.verify_token_or_secret("my_admin_password_123"));
    assert!(!state1.verify_token_or_secret("wrong_token"));

    // 2. Second boot (simulating service restart with same database): loads same token from SQLite
    let state2 = AppState::new(config.clone()).await.expect("AppState creation 2");
    let token2 = state2.permanent_sub_token.clone();
    assert_eq!(token1, token2, "Subscription token must remain identical across restarts!");

    // 3. User can also specify a custom sub_token in config.toml
    let mut config3 = config.clone();
    config3.auth.sub_token = "my_custom_fixed_token_abc".to_string();
    let state3 = AppState::new(config3).await.expect("AppState creation 3");
    assert_eq!(state3.permanent_sub_token, "my_custom_fixed_token_abc");
    assert!(state3.verify_token_or_secret("my_custom_fixed_token_abc"));

    // Cleanup
    drop(state1);
    drop(state2);
    drop(state3);
    let _ = std::fs::remove_file(&db_path);
}
