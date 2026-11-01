//! Agent tools: their schemas, their JSON (`docs/product-spec.md` 6), and the JSON-line
//! protocol between `braydr-mcp` and the app.

pub mod server;
pub mod tools;
pub mod views;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One line from `braydr-mcp` to the app.
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub channel: Option<String>,
    pub tool: String,
    pub args: Value,
}

/// One line back: `{"result": ...}` or `{"error": ...}`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Result(Value),
    Error(Value),
}
