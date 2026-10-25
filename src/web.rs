use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode},
    response::Response,
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "static/"]
pub struct Asset;

pub async fn static_handler(uri: axum::http::Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() {
        path = "index.html".to_string();
    }

    match Asset::get(&path) {
        Some(content) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, HeaderValue::from_str(mime.as_ref()).unwrap())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => {
            // Embedded flags (static/flags/*.png) live under /flags/<code>.png.
            // Never fall back to index.html for asset paths: a missing image
            // must 404 so the browser can hide it instead of rendering HTML.
            if path.contains('.') && !path.ends_with(".html") {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))
                    .body(Body::from(""))
                    .unwrap();
            }
            // Fallback to index.html for SPA if not found
            if let Some(content) = Asset::get("index.html") {
                Response::builder()
                    .header(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"))
                    .body(Body::from(content.data))
                    .unwrap()
            } else {
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::from("404 Not Found"))
                    .unwrap()
            }
        }
    }
}
