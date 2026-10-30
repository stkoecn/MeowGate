use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthQueryParams {
    pub token: Option<String>,
}

/// Checks whether auth is enabled and whether the current request is authenticated.
pub async fn auth_status(
    State(state): State<SharedState>,
    headers: HeaderMap,
    Query(query): Query<AuthQueryParams>,
) -> Json<Value> {
    let auth_enabled = state.is_auth_enabled();
    let authenticated = if !auth_enabled {
        true
    } else {
        extract_and_verify(&state, &headers, query.token.as_deref())
    };

    // If authenticated, automatically provide the permanent subscription token
    // so any subscription link generated for this authorized user can be directly imported into Clash/clients!
    let sub_token = if auth_enabled && authenticated {
        Some(state.permanent_sub_token.clone())
    } else {
        None
    };

    Json(json!({
        "success": true,
        "auth_enabled": auth_enabled,
        "authenticated": authenticated,
        "sub_token": sub_token
    }))
}

/// Logs in with admin password and returns an auth session token + HttpOnly cookie.
pub async fn login(
    State(state): State<SharedState>,
    Json(payload): Json<LoginRequest>,
) -> Response {
    if !state.is_auth_enabled() {
        return (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": "无需认证，当前为免密模式",
                "token": ""
            })),
        )
            .into_response();
    }

    if state.verify_password(&payload.password) {
        // 7 days TTL (604,800 seconds)
        let ttl_secs = 7 * 24 * 3600;
        let token = state.auth_mgr.create_token(ttl_secs);

        let cookie = format!(
            "meow_token={}; Path=/; Max-Age={}; SameSite=Lax; HttpOnly",
            token, ttl_secs
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::SET_COOKIE, cookie.parse().unwrap());

        (
            StatusCode::OK,
            headers,
            Json(json!({
                "success": true,
                "token": token,
                "expires_in": ttl_secs,
                "message": "登录验证成功"
            })),
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "success": false,
                "message": "访问密码错误，请重新输入"
            })),
        )
            .into_response()
    }
}

/// Logs out by clearing the auth session cookie.
pub async fn logout() -> Response {
    let cookie = "meow_token=; Path=/; Max-Age=0; SameSite=Lax; HttpOnly";
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie.parse().unwrap());

    (
        StatusCode::OK,
        headers,
        Json(json!({
            "success": true,
            "message": "已成功退出登录"
        })),
    )
        .into_response()
}

/// Extracts token from Authorization header, Cookie, or query param, and verifies it.
pub fn extract_and_verify(state: &SharedState, headers: &HeaderMap, query_token: Option<&str>) -> bool {
    if !state.is_auth_enabled() {
        return true;
    }

    // 1. Check Authorization: Bearer <token/password>
    if let Some(auth_header) = headers.get(header::AUTHORIZATION).and_then(|h| h.to_str().ok()) {
        if let Some(token) = auth_header.strip_prefix("Bearer ").or_else(|| auth_header.strip_prefix("bearer ")) {
            if state.verify_token_or_secret(token.trim()) {
                return true;
            }
        }
    }

    // 2. Check Cookie: meow_token=<token>
    if let Some(cookie_header) = headers.get(header::COOKIE).and_then(|h| h.to_str().ok()) {
        for cookie in cookie_header.split(';') {
            let cookie = cookie.trim();
            if let Some(token) = cookie.strip_prefix("meow_token=") {
                if state.verify_token_or_secret(token.trim()) {
                    return true;
                }
            }
        }
    }

    // 3. Check query param: ?token=<token>
    if let Some(token) = query_token {
        if state.verify_token_or_secret(token.trim()) {
            return true;
        }
    }

    false
}

/// Axum middleware protecting `/api/*` routes.
pub async fn auth_middleware(
    State(state): State<SharedState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let path = req.uri().path();

    // Exempt auth endpoints and health check from authentication
    if path.ends_with("/auth/login") || path.ends_with("/auth/status") || path.ends_with("/health") {
        return next.run(req).await;
    }

    if state.is_auth_enabled() {
        let query_token = req.uri().query().and_then(|q| {
            q.split('&').find_map(|pair| pair.strip_prefix("token="))
        });
        if !extract_and_verify(&state, req.headers(), query_token) {
            return (
                StatusCode::UNAUTHORIZED,
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
                ],
                axum::Json(serde_json::json!({
                    "success": false,
                    "message": "Unauthorized: valid authentication token required",
                    "code": 401
                })),
            )
                .into_response();
        }
    }

    next.run(req).await
}
