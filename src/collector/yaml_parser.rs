use crate::collector::Node;
use crate::error::{AppError, Result};
use std::path::Path;

pub struct YamlParser;

impl YamlParser {
    pub fn parse_file<P: AsRef<Path>>(path: P) -> Result<Vec<Node>> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| AppError::Config(format!("Failed to read YAML file: {}", e)))?;
        Self::parse_str(&content)
    }

    pub fn parse_str(content: &str) -> Result<Vec<Node>> {
        let root: serde_yaml::Value = serde_yaml::from_str(content)
            .map_err(|e| AppError::Config(format!("Failed to parse YAML: {}", e)))?;

        let proxies_seq = root
            .get("proxies")
            .and_then(serde_yaml::Value::as_sequence)
            .ok_or_else(|| AppError::Config("YAML configuration does not contain a 'proxies' list".to_string()))?;

        let mut nodes = Vec::new();
        for proxy_val in proxies_seq {
            let json_val: serde_json::Value = serde_json::to_value(proxy_val)
                .map_err(|e| AppError::Config(format!("Failed to convert YAML proxy node: {}", e)))?;

            if let Ok(node) = serde_json::from_value::<Node>(json_val) {
                if !node.name.is_empty() {
                    nodes.push(node);
                }
            }
        }

        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(nodes)
    }
}
