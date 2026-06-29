use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputEnvelope {
    pub kind: String,
    pub path: Option<String>,
    pub engine: String,
    pub schema_version: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error_code: String,
    pub exit_code: u8,
    pub message: String,
    pub hint: Option<String>,
}
