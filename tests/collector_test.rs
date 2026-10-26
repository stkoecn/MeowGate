use meowgate::collector::yaml_parser::YamlParser;

#[test]
fn test_yaml_parser() {
    let yaml = r#"
proxies:
  - name: "HK-01"
    type: ss
    server: 1.2.3.4
    port: 8388
    cipher: aes-128-gcm
    password: test
  - name: "US-01"
    type: vmess
    server: 5.6.7.8
    port: 443
    uuid: 12345678-1234-1234-1234-123456789abc
    alterId: 0
"#;

    let nodes = YamlParser::parse_str(yaml).expect("Failed to parse YAML");
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0].name, "HK-01");
    assert_eq!(nodes[0].proxy_type, "ss");
    assert_eq!(nodes[0].port, 8388);

    assert_eq!(nodes[1].name, "US-01");
    assert_eq!(nodes[1].proxy_type, "vmess");
    assert_eq!(nodes[1].port, 443);
}

#[test]
fn test_multi_user_dedup_and_credentials() {
    use meowgate::collector::Node;

    // 1. Two nodes with same server, port, type, and SAME uuid -> identical dedup_key
    let mut extra1 = serde_json::Map::new();
    extra1.insert("uuid".to_string(), serde_json::json!("user-a-uuid-1234"));
    let node1 = Node {
        name: "HK-01".to_string(),
        proxy_type: "vmess".to_string(),
        server: "hk.example.com".to_string(),
        port: 443,
        extra: extra1,
    };

    let mut extra2 = serde_json::Map::new();
    extra2.insert("uuid".to_string(), serde_json::json!("user-a-uuid-1234"));
    let node2 = Node {
        name: "HK-01-Duplicate".to_string(),
        proxy_type: "vmess".to_string(),
        server: "HK.example.COM".to_string(),
        port: 443,
        extra: extra2,
    };

    assert_eq!(node1.dedup_key(), node2.dedup_key());

    // 2. User B has node on the exact same server & port, but DIFFERENT uuid (multi-user scenario)
    let mut extra3 = serde_json::Map::new();
    extra3.insert("uuid".to_string(), serde_json::json!("user-b-uuid-5678"));
    let node3 = Node {
        name: "HK-01".to_string(),
        proxy_type: "vmess".to_string(),
        server: "hk.example.com".to_string(),
        port: 443,
        extra: extra3,
    };

    assert_ne!(node1.dedup_key(), node3.dedup_key());

    // 3. Shadowsocks with different passwords on same server
    let mut extra_ss1 = serde_json::Map::new();
    extra_ss1.insert("password".to_string(), serde_json::json!("pass_alice"));
    let ss1 = Node {
        name: "SS-01".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.2.3.4".to_string(),
        port: 8388,
        extra: extra_ss1,
    };

    let mut extra_ss2 = serde_json::Map::new();
    extra_ss2.insert("password".to_string(), serde_json::json!("pass_bob"));
    let ss2 = Node {
        name: "SS-01".to_string(),
        proxy_type: "ss".to_string(),
        server: "1.2.3.4".to_string(),
        port: 8388,
        extra: extra_ss2,
    };

    assert_ne!(ss1.dedup_key(), ss2.dedup_key());

    // 4. SSH / SOCKS5 with different users on same server
    let mut extra_ssh1 = serde_json::Map::new();
    extra_ssh1.insert("username".to_string(), serde_json::json!("admin"));
    let ssh1 = Node {
        name: "SSH-01".to_string(),
        proxy_type: "ssh".to_string(),
        server: "ssh.instabox.sh".to_string(),
        port: 2222,
        extra: extra_ssh1,
    };

    let mut extra_ssh2 = serde_json::Map::new();
    extra_ssh2.insert("username".to_string(), serde_json::json!("guest"));
    let ssh2 = Node {
        name: "SSH-01".to_string(),
        proxy_type: "ssh".to_string(),
        server: "ssh.instabox.sh".to_string(),
        port: 2222,
        extra: extra_ssh2,
    };

    assert_ne!(ssh1.dedup_key(), ssh2.dedup_key());
}
