//! MCP stdio server for one agent session. It holds no logic: every call goes to the app.

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::Arc;

use braydr_ipc::{Request, Response, tools};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock as Content, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData as McpError, ServiceExt, transport::stdio};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

struct Bridge {
    channel: Option<String>,
    socket: PathBuf,
    tools: Vec<Tool>,
}

impl Bridge {
    async fn forward(&self, tool: String, args: Value) -> std::io::Result<Response> {
        let mut stream = UnixStream::connect(&self.socket).await?;
        let mut line = serde_json::to_vec(&Request {
            channel: self.channel.clone(),
            tool,
            args,
        })?;
        line.push(b'\n');
        stream.write_all(&line).await?;
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).await?;
        Ok(serde_json::from_str(&reply)?)
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, McpError> {
        let args = request
            .arguments
            .map(Value::Object)
            .unwrap_or_else(|| json!({}));
        let result = match self.forward(request.name.into_owned(), args).await {
            Ok(Response::Result(value)) => {
                CallToolResult::success(vec![Content::text(value.to_string())])
            }
            Ok(Response::Error(error)) => {
                CallToolResult::error(vec![Content::text(json!({ "error": error }).to_string())])
            }
            Err(error) => {
                let body = json!({ "error": {
                    "code": "internal",
                    "message": format!("cannot reach the Braydr app: {error}"),
                }});
                CallToolResult::error(vec![Content::text(body.to_string())])
            }
        };
        Ok(result.into())
    }
}

fn default_socket() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Library/Application Support/Braydr/ipc.sock")
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let tools = tools::specs()
        .into_iter()
        .map(|spec| {
            let schema = spec
                .schema
                .as_object()
                .expect("schemas are objects")
                .clone();
            Tool::new(
                Cow::Borrowed(spec.name),
                Cow::Borrowed(spec.description),
                Arc::new(schema),
            )
        })
        .collect();
    let bridge = Bridge {
        channel: std::env::var("BRAYDR_CHANNEL_ID").ok(),
        socket: std::env::var_os("BRAYDR_IPC_SOCKET").map_or_else(default_socket, PathBuf::from),
        tools,
    };
    bridge.serve(stdio()).await?.waiting().await?;
    Ok(())
}
