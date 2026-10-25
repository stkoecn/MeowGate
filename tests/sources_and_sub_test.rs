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
        created_at: Utc::now(),
    };

    // 1. Save source
    db.save_source(&source).await.expect("save_source failed");
    let fetched = db.get_source("src-test-1").await.expect("get_source failed");
    assert!(fetched.is_some());
    assert_eq!(fetched.unwrap().name, "Test Clash File");

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
