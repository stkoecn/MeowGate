use hmac::{Hmac, Mac};
use sha2::Sha256;
use rand::RngCore;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct AuthManager {
    secret: Vec<u8>,
}

impl AuthManager {
    pub fn new(configured_secret: &str) -> Self {
        let secret = if configured_secret.trim().is_empty() {
            let mut rand_bytes = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut rand_bytes);
            rand_bytes.to_vec()
        } else {
            configured_secret.as_bytes().to_vec()
        };
        Self { secret }
    }

    /// Generates a signed token valid for `ttl_secs` (e.g. 7 days = 604,800s).
    /// Format: `<exp_unix_hex>.<hex_hmac_signature>`
    pub fn create_token(&self, ttl_secs: u64) -> String {
        let exp = chrono::Utc::now().timestamp() as u64 + ttl_secs;
        let payload = format!("{:x}", exp);
        let sig = self.sign(&payload);
        format!("{}.{}", payload, sig)
    }

    /// Verifies if a token is authentic and not expired.
    pub fn verify_token(&self, token: &str) -> bool {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 2 {
            return false;
        }
        let payload = parts[0];
        let sig = parts[1];

        // 1. Verify signature
        let expected_sig = self.sign(payload);
        if expected_sig != sig {
            return false;
        }

        // 2. Check expiration
        if let Ok(exp) = u64::from_str_radix(payload, 16) {
            let now = chrono::Utc::now().timestamp() as u64;
            exp > now
        } else {
            false
        }
    }

    fn sign(&self, data: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .expect("HMAC can take key of any size");
        mac.update(data.as_bytes());
        let result = mac.finalize();
        hex_encode(result.into_bytes().as_slice())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_creation_and_verification() {
        let auth = AuthManager::new("my_test_secret");
        let token = auth.create_token(3600);
        assert!(auth.verify_token(&token));

        // Tampered token
        let tampered = format!("{}x", token);
        assert!(!auth.verify_token(&tampered));

        // Invalid format
        assert!(!auth.verify_token("invalid_token"));

        // Expired token (ttl = 0, exp in past)
        let expired = format!("{:x}.{}", 1000u64, auth.sign("3e8"));
        assert!(!auth.verify_token(&expired));
    }
}
