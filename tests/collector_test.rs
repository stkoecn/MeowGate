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
