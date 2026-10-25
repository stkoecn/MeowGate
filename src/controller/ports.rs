//! Port helpers: pick a free loopback port for the kernel's internal inbound
//! so a busy port can never stop the gateway from starting.

use std::net::{SocketAddr, TcpListener};

/// Returns `true` when `127.0.0.1:port` can be bound right now.
pub fn is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Returns `preferred` when it is free, otherwise the next free port at or
/// after it (scanning up to 200 candidates).
pub fn resolve(preferred: u16) -> u16 {
    if preferred != 0 && is_free(preferred) {
        return preferred;
    }
    for offset in 1..=200u16 {
        let candidate = preferred.saturating_add(offset);
        if candidate > 1024 && is_free(candidate) {
            return candidate;
        }
    }
    // Last resort: let the OS choose.
    0
}

/// Binds a listener on `127.0.0.1` starting at `preferred`, returning the
/// listener and the port it actually bound.
#[allow(dead_code)]
pub fn bind_loopback(preferred: u16) -> std::io::Result<(TcpListener, u16)> {
    let mut candidate = resolve(preferred);
    loop {
        match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], candidate))) {
            Ok(l) => {
                let port = l.local_addr()?.port();
                return Ok((l, port));
            }
            Err(_) if candidate != 0 && candidate < preferred.saturating_add(220) => {
                candidate += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_returns_preferred_when_free() {
        let mut port = 45100;
        while !is_free(port) {
            port += 1;
        }
        assert_eq!(resolve(port), port);
    }

    #[test]
    fn resolve_avoids_busy_port() {
        let held = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = held.local_addr().unwrap().port();
        let picked = resolve(busy);
        assert_ne!(picked, busy, "busy port must not be reused");
        assert!(picked == 0 || is_free(picked));
    }

    #[test]
    fn bind_loopback_reports_real_port() {
        let (_listener, port) = bind_loopback(0).unwrap();
        assert!(port > 0);
    }
}
