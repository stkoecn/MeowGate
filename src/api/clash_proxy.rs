use axum::{
    body::Body,
    extract::State,
    http::{Request, Response, StatusCode},
};
use crate::state::SharedState;

/// Transparent reverse-proxy forwarding Clash/Mihomo REST API requests on port :9565
/// directly to the internal kernel controller on :9091.
///
/// This allows external dashboards (ZashBoard, Yacd, Metacubexd, Clash Verge, etc.)
/// to connect to MeowGate's unified port (http://<ip>:9565) as if it were a native Clash/Mihomo backend.
pub async fn clash_proxy_handler(
    State(state): State<SharedState>,
    req: Request<Body>,
) -> Response<Body> {
    let kernel_api = &state.config.meow.api_url;
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("");
    let target_url = format!("{}{}", kernel_api.trim_end_matches('/'), path_and_query);

    let method = req.method().clone();
    let mut client_req = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap_or_default()
        .request(method, &target_url);

    // Forward headers
    for (name, val) in req.headers() {
        if name != axum::http::header::HOST && name != axum::http::header::CONTENT_LENGTH {
            if let Ok(v) = reqwest::header::HeaderValue::from_bytes(val.as_bytes()) {
                if let Ok(hn) = reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()) {
                    client_req = client_req.header(hn, v);
                }
            }
        }
    }

    // Forward body if present
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap_or_default();
    if !body_bytes.is_empty() {
        client_req = client_req.body(body_bytes);
    }

    match client_req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut response_builder = Response::builder().status(status);

            // Forward response headers
            for (k, v) in resp.headers() {
                if k != reqwest::header::CONTENT_LENGTH && k != reqwest::header::TRANSFER_ENCODING {
                    if let Ok(ak) = axum::http::header::HeaderName::from_bytes(k.as_str().as_bytes()) {
                        if let Ok(av) = axum::http::header::HeaderValue::from_bytes(v.as_bytes()) {
                            response_builder = response_builder.header(ak, av);
                        }
                    }
                }
            }

            // Always allow CORS for external web dashboards
            response_builder = response_builder
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "*")
                .header("Access-Control-Allow-Headers", "*");

            let bytes = resp.bytes().await.unwrap_or_default();
            response_builder
                .body(Body::from(bytes))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap()
                })
        }
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::json!({
                "error": format!("Kernel API unreachable: {}", e)
            }).to_string()))
            .unwrap(),
    }
}

/// Streaming reverse-proxy for /traffic and /logs endpoints.
pub async fn clash_proxy_stream_handler(
    State(state): State<SharedState>,
    req: Request<Body>,
) -> Response<Body> {
    let kernel_api = &state.config.meow.api_url;
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("");
    let target_url = format!("{}{}", kernel_api.trim_end_matches('/'), path_and_query);

    let method = req.method().clone();
    let client_req = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap_or_default()
        .request(method, &target_url);

    match client_req.send().await {
        Ok(resp) => {
            let status = StatusCode::from_u16(resp.status().as_u16())
                .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let mut response_builder = Response::builder().status(status);

            for (k, v) in resp.headers() {
                if let Ok(ak) = axum::http::header::HeaderName::from_bytes(k.as_str().as_bytes()) {
                    if let Ok(av) = axum::http::header::HeaderValue::from_bytes(v.as_bytes()) {
                        response_builder = response_builder.header(ak, av);
                    }
                }
            }

            response_builder = response_builder
                .header("Access-Control-Allow-Origin", "*")
                .header("Access-Control-Allow-Methods", "*")
                .header("Access-Control-Allow-Headers", "*");

            use futures_util::TryStreamExt;
            let stream = resp.bytes_stream().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
            response_builder
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::empty())
                        .unwrap()
                })
        }
        Err(e) => Response::builder()
            .status(StatusCode::BAD_GATEWAY)
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::json!({
                "error": format!("Kernel stream unreachable: {}", e)
            }).to_string()))
            .unwrap(),
    }
}
