use chrono::Utc;
use crate::collector::User;

pub struct AuthGenerator;

impl AuthGenerator {
    /// Generates a compact ASCII-safe username: lowercase region code + short 8-char hex hash.
    /// E.g. "us3e5d24be" (US node) or "jp39c0ea95" (Japan node) or "hkd6bbc82f" (HK node).
    /// If region cannot be inferred from the node name, defaults to "xx".
    /// Short, clean, human-readable region, deterministic, and collision-free across thousands of nodes.
    pub fn sanitize_username(node_name: &str) -> String {
        let region = crate::collector::country::CountryInfer::from_name(node_name)
            .unwrap_or_else(|| "xx".to_string())
            .to_lowercase();
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        node_name.hash(&mut hasher);
        let h = (hasher.finish() & 0xffff_ffff) as u32;
        format!("{}{:08x}", region, h)
    }

    /// Password policy: password == username. This project's credentials are
    /// only used to route IN-USER rules, so no secret strength is required.
    pub fn create_user_for_node(node_name: &str) -> User {
        let username = Self::sanitize_username(node_name);
        User {
            node_name: node_name.to_string(),
            username: username.clone(),
            password: username,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_username() {
        let u1 = AuthGenerator::sanitize_username("HK:01@Fast");
        assert!(u1.starts_with("hk"));
        assert!(u1.is_ascii());
        assert!(!u1.contains(':'));
        assert!(!u1.contains('@'));

        let u2 = AuthGenerator::sanitize_username("JP 免费-日本1-Ver.7");
        assert!(u2.starts_with("jp"));
        assert!(u2.is_ascii());
        assert!(!u2.contains(' '));

        let u3 = AuthGenerator::sanitize_username("HK:01@Fast");
        assert_eq!(u1, u3, "Hashing must be deterministic");
    }

    #[test]
    fn test_password_equals_username() {
        let u = AuthGenerator::create_user_for_node("HK:01@Fast");
        assert_eq!(u.username, u.password);
    }
}
