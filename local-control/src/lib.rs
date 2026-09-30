pub mod gateway;
pub mod store;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlError {
    pub code: String,
    pub message: String,
}
impl ControlError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ControlError {}
impl From<String> for ControlError {
    fn from(e: String) -> Self {
        Self::new("internal", e)
    }
}
impl From<sqlx::Error> for ControlError {
    fn from(e: sqlx::Error) -> Self {
        Self::new("internal", e.to_string())
    }
}

pub mod import;
