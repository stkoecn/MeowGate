//! Single-port gateway multiplexer, implemented as an axum `Listener`.
//!
//! Only **one public TCP port** is exposed. On accept we peek (non-destructively)
//! at the first bytes of the connection:
//!
//! * SOCKS5 / HTTP proxy traffic with credentials -> tunneled directly to kernel
//! * SOCKS5 / HTTP proxy traffic WITHOUT credentials -> transparently bridged to
//!   kernel using the internal 'guest:guest' account, routing to the PROXY group
//!   (allowing seamless unauthenticated use just like a standard Clash client)
//! * everything else (origin-form `GET /api/…`, `GET /version`, Web UI) -> handed to axum
//!
//! Internal (loopback-only) ports stay private:
//! `gateway_port` is the only externally reachable port.

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use axum::serve::Listener;
use tokio::io::copy_bidirectional;
use tokio::net::{TcpListener, TcpStream};

/// axum listener that splits proxy traffic from Web UI traffic on one port.
pub struct GatewayMux {
    inner: TcpListener,
    /// Kernel mixed inbound (HTTP + SOCKS5) on loopback.
    proxy_port: u16,
}

impl GatewayMux {
    pub async fn bind(addr: &str, proxy_port: u16) -> io::Result<Self> {
        let inner = TcpListener::bind(addr).await?;
        tracing::info!(
            "gateway listening on {} — Web UI, Clash REST API, and HTTP/SOCKS5 proxy share this port (kernel inbound 127.0.0.1:{})",
            addr,
            proxy_port
        );
        Ok(Self { inner, proxy_port })
    }
}

impl Listener for GatewayMux {
    type Io = TcpStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (TcpStream, SocketAddr) {
        loop {
            let (mut socket, peer) = match self.inner.accept().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!("gateway accept error: {}", e);
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            };
            let _ = socket.set_nodelay(true);

            match classify(&mut socket).await {
                Some(ConnKind::ProxyWithAuth) => {
                    let proxy_port = self.proxy_port;
                    tokio::spawn(async move {
                        if let Err(e) = tunnel_direct(socket, proxy_port).await {
                            tracing::debug!("proxy tunnel with auth from {} closed: {}", peer, e);
                        }
                    });
                }
                Some(ConnKind::ProxyHttpNoAuth) => {
                    let proxy_port = self.proxy_port;
                    tokio::spawn(async move {
                        if let Err(e) = tunnel_http_guest(socket, proxy_port).await {
                            tracing::debug!("guest http tunnel from {} closed: {}", peer, e);
                        }
                    });
                }
                Some(ConnKind::ProxySocks5NoAuth) => {
                    let proxy_port = self.proxy_port;
                    tokio::spawn(async move {
                        if let Err(e) = tunnel_socks5_guest(socket, proxy_port).await {
                            tracing::debug!("guest socks5 tunnel from {} closed: {}", peer, e);
                        }
                    });
                }
                Some(ConnKind::Web) => return (socket, peer),
                None => {
                    // Idle / closed / unreadable connection: drop it.
                    continue;
                }
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }
}

/// Direct tunnel for connections that already carry credentials.
async fn tunnel_direct(mut client: TcpStream, proxy_port: u16) -> io::Result<()> {
    let mut upstream = TcpStream::connect(("127.0.0.1", proxy_port)).await?;
    let _ = upstream.set_nodelay(true);
    copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Unauthenticated HTTP proxy tunnel: injects "guest:guest" Proxy-Authorization
/// so the connection is accepted by Mihomo and routed to the standard PROXY group.
async fn tunnel_http_guest(mut client: TcpStream, proxy_port: u16) -> io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut header_buf = Vec::with_capacity(1024);
    let mut temp = [0u8; 1024];

    loop {
        let n = client.read(&mut temp).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Client disconnected before completing HTTP proxy request",
            ));
        }
        header_buf.extend_from_slice(&temp[..n]);
        if header_buf.windows(4).any(|w| w == b"\r\n\r\n") || header_buf.len() > 16 * 1024 {
            break;
        }
    }

    // Insert Proxy-Authorization header right before \r\n\r\n
    // "guest:guest" in base64 is "Z3Vlc3Q6Z3Vlc3Q="
    let mut modified = Vec::with_capacity(header_buf.len() + 48);
    if let Some(pos) = header_buf.windows(4).position(|w| w == b"\r\n\r\n") {
        modified.extend_from_slice(&header_buf[..pos + 2]); // up to first \r\n
        modified.extend_from_slice(b"Proxy-Authorization: Basic Z3Vlc3Q6Z3Vlc3Q=\r\n\r\n");
        modified.extend_from_slice(&header_buf[pos + 4..]); // any trailing body bytes already read
    } else {
        modified = header_buf;
    }

    let mut upstream = match TcpStream::connect(("127.0.0.1", proxy_port)).await {
        Ok(s) => s,
        Err(e) => {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
            return Err(e);
        }
    };
    let _ = upstream.set_nodelay(true);
    upstream.write_all(&modified).await?;

    copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

/// Unauthenticated SOCKS5 proxy tunnel: accepts the client with NoAuth (0x00),
/// logs in to Mihomo using RFC 1929 "guest:guest", and bridges the connection.
async fn tunnel_socks5_guest(mut client: TcpStream, proxy_port: u16) -> io::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // 1. Read client greeting: [0x05, NMETHODS, ...methods]
    let mut ver_nmethods = [0u8; 2];
    client.read_exact(&mut ver_nmethods).await?;
    let nmethods = ver_nmethods[1] as usize;
    let mut methods = vec![0u8; nmethods];
    client.read_exact(&mut methods).await?;

    // Respond to client: Version 5, Method 0x00 (No authentication required)
    client.write_all(&[0x05, 0x00]).await?;

    // 2. Connect to upstream Mihomo on proxy_port and negotiate SOCKS5 auth with guest:guest
    let mut upstream = match TcpStream::connect(("127.0.0.1", proxy_port)).await {
        Ok(s) => s,
        Err(e) => {
            // SOCKS5 general failure: VER=5, REP=1, RSV=0, ATYP=1, ADDR=0.0.0.0, PORT=0
            let _ = client.write_all(&[0x05, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0, 0]).await;
            return Err(e);
        }
    };
    let _ = upstream.set_nodelay(true);

    // Offer Username/Password (0x02) to Mihomo
    upstream.write_all(&[0x05, 0x01, 0x02]).await?;
    let mut up_choice = [0u8; 2];
    upstream.read_exact(&mut up_choice).await?;
    if up_choice != [0x05, 0x02] {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Upstream kernel did not accept SOCKS5 user/password auth",
        ));
    }

    // Send RFC 1929 subnegotiation: [0x01, ULEN, UNAME, PLEN, PASSWD]
    let u = b"guest";
    let p = b"guest";
    let mut auth_req = Vec::with_capacity(4 + u.len() + p.len());
    auth_req.push(0x01);
    auth_req.push(u.len() as u8);
    auth_req.extend_from_slice(u);
    auth_req.push(p.len() as u8);
    auth_req.extend_from_slice(p);
    upstream.write_all(&auth_req).await?;

    let mut auth_resp = [0u8; 2];
    upstream.read_exact(&mut auth_resp).await?;
    if auth_resp[1] != 0x00 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Upstream kernel rejected guest credentials",
        ));
    }

    // 3. Upstream is authenticated as guest! Bridge client and upstream
    copy_bidirectional(&mut client, &mut upstream).await?;
    Ok(())
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ConnKind {
    Web,
    ProxyWithAuth,
    ProxyHttpNoAuth,
    ProxySocks5NoAuth,
}

/// Peeks at the beginning of the connection and decides where it belongs.
/// Returns `None` when the peer closed or stayed silent.
async fn classify(socket: &mut TcpStream) -> Option<ConnKind> {
    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    loop {
        let n = match tokio::time::timeout(Duration::from_secs(10), socket.peek(&mut buf)).await {
            Ok(Ok(0)) => return None,
            Ok(Ok(n)) => n,
            Ok(Err(_)) => return None,
            Err(_) => return None,
        };

        let head = &buf[..n];
        if let Some(kind) = sniff(head) {
            return Some(kind);
        }
        if head.windows(4).any(|w| w == b"\r\n\r\n") {
            // Full request head seen but no proxy fingerprint -> Web UI.
            return Some(ConnKind::Web);
        }
        if tokio::time::Instant::now() >= deadline {
            return Some(ConnKind::Web);
        }
        // Partial request: let more bytes arrive, then re-peek.
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Pure header sniffer (unit tested).
pub fn sniff(head: &[u8]) -> Option<ConnKind> {
    if head.is_empty() {
        return None;
    }
    // SOCKS5 greeting: 0x05, NMETHODS, ...
    if head[0] == 0x05 {
        if head.len() >= 2 {
            let nmethods = head[1] as usize;
            if head.len() >= 2 + nmethods {
                let methods = &head[2..2 + nmethods];
                // If client offers username/password (0x02), tunnel directly to kernel
                // so the client can authenticate with its credentials.
                if methods.contains(&0x02) {
                    return Some(ConnKind::ProxyWithAuth);
                } else if methods.contains(&0x00) {
                    return Some(ConnKind::ProxySocks5NoAuth);
                } else {
                    return Some(ConnKind::ProxyWithAuth);
                }
            }
        }
        if head.len() == 3 && head[1] == 1 && head[2] == 0 {
            return Some(ConnKind::ProxySocks5NoAuth);
        }
        return Some(ConnKind::ProxyWithAuth);
    }
    // SOCKS4 connect or TLS ClientHello
    if head[0] == 0x04 || head[0] == 0x16 {
        return Some(ConnKind::ProxyWithAuth);
    }

    let text = String::from_utf8_lossy(head);
    let lower = text.to_ascii_lowercase();

    // Check if it has proxy credentials
    let has_auth = lower.contains("\r\nproxy-authorization:") || lower.starts_with("proxy-authorization:");
    let headers_complete = head.windows(4).any(|w| w == b"\r\n\r\n");

    // CONNECT host:port HTTP/1.1
    if lower.starts_with("connect ") {
        if has_auth {
            return Some(ConnKind::ProxyWithAuth);
        } else if headers_complete {
            return Some(ConnKind::ProxyHttpNoAuth);
        } else {
            // Incomplete headers: wait for more bytes to see if Proxy-Authorization arrives
            return None;
        }
    }
    // Absolute-form request target: GET http://example.com/ HTTP/1.1
    if let Some(rest) = lower.split_once(' ').map(|(_, r)| r) {
        if rest.starts_with("http://") || rest.starts_with("https://") {
            if has_auth {
                return Some(ConnKind::ProxyWithAuth);
            } else if headers_complete {
                return Some(ConnKind::ProxyHttpNoAuth);
            } else {
                return None;
            }
        }
    }
    // Explicit proxy credentials without CONNECT/http:// prefix (rare)
    if has_auth {
        return Some(ConnKind::ProxyWithAuth);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_socks5() {
        assert_eq!(sniff(&[0x05, 0x01, 0x00]), Some(ConnKind::ProxySocks5NoAuth));
        assert_eq!(sniff(&[0x05, 0x01, 0x02]), Some(ConnKind::ProxyWithAuth));
        assert_eq!(sniff(&[0x05, 0x02, 0x00, 0x02]), Some(ConnKind::ProxyWithAuth));
        assert_eq!(sniff(&[0x05, 0x02, 0x02, 0x00]), Some(ConnKind::ProxyWithAuth));
    }

    #[test]
    fn sniffs_http_connect() {
        // Without \r\n\r\n and without auth: None (wait for complete headers)
        assert_eq!(
            sniff(b"CONNECT api.ipify.org:443 HTTP/1.1\r\nHost: api.ipify.org\r\n"),
            None
        );
        // With \r\n\r\n and without auth: ProxyHttpNoAuth
        assert_eq!(
            sniff(b"CONNECT api.ipify.org:443 HTTP/1.1\r\nHost: api.ipify.org\r\n\r\n"),
            Some(ConnKind::ProxyHttpNoAuth)
        );
        // With auth even before \r\n\r\n: ProxyWithAuth immediately
        assert_eq!(
            sniff(b"CONNECT api.ipify.org:443 HTTP/1.1\r\nHost: api.ipify.org\r\nProxy-Authorization: Basic test\r\n"),
            Some(ConnKind::ProxyWithAuth)
        );
    }

    #[test]
    fn sniffs_absolute_form_proxy_request() {
        // Without \r\n\r\n and without auth: None (wait)
        assert_eq!(
            sniff(b"GET http://ip-api.com/json HTTP/1.1\r\nHost: ip-api.com\r\n"),
            None
        );
        // With \r\n\r\n and without auth: ProxyHttpNoAuth
        assert_eq!(
            sniff(b"GET http://ip-api.com/json HTTP/1.1\r\nHost: ip-api.com\r\n\r\n"),
            Some(ConnKind::ProxyHttpNoAuth)
        );
        // With auth: ProxyWithAuth
        assert_eq!(
            sniff(b"GET http://ip-api.com/json HTTP/1.1\r\nHost: ip-api.com\r\nProxy-Authorization: Basic test\r\n"),
            Some(ConnKind::ProxyWithAuth)
        );
    }

    #[test]
    fn sniffs_proxy_authorization() {
        assert_eq!(
            sniff(b"POST /something HTTP/1.1\r\nProxy-Authorization: Basic xyz\r\n"),
            Some(ConnKind::ProxyWithAuth)
        );
    }

    #[test]
    fn web_ui_requests_are_web() {
        assert_eq!(sniff(b"GET /api/health HTTP/1.1\r\n"), None);
        assert_eq!(sniff(b"POST /api/config/reload HTTP/1.1\r\n"), None);
        assert_eq!(sniff(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n"), None);
    }
}
