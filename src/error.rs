use thiserror::Error;
use axum::{
    response::{IntoResponse, Response},
    http::StatusCode,
    Json,
};
use serde_json::json;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Clash collector error: {0}")]
    Clash(String),

    #[error("Meow-rs controller error: {0}")]
    Meow(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Node not found: {0}")]
    #[allow(dead_code)]
    NodeNotFound(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML serialization error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Internal error: {0}")]
    Anyhow(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error_message) = match &self {
            AppError::NodeNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Clash(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            AppError::Meow(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            AppError::Config(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::Yaml(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        let body = Json(json!({
            "success": false,
            "error": error_message
        }));

        (status, body).into_response()
    }
}

pub type Result<T> = std::result::Result<T, AppError>;
