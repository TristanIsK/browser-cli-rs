use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("authentication failed: {0}")]
    Authentication(String),
    #[error("resource not found: {0}")]
    NotFound(String),
    #[error("resource conflict: {0}")]
    Conflict(String),
    #[error("request timed out: {0}")]
    Timeout(String),
    #[error("API request failed with HTTP {status}: {message}")]
    Api {
        status: u16,
        message: String,
        body: Option<Value>,
    },
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket error: {0}")]
    WebSocket(Box<tungstenite::Error>),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("local tunnel error: {0}")]
    Tunnel(String),
    #[error("CDP command failed: {0}")]
    Cdp(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<tungstenite::Error> for Error {
    fn from(value: tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(value))
    }
}
