//! Runs the real sidecar over stdio against a real socket.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use braydr_core::LaunchConfig;
use braydr_store::Store;
use serde_json::{Value, json};

struct Sidecar {
    child: std::process::Child,
    input: std::process::ChildStdin,
    output: BufReader<std::process::ChildStdout>,
}

impl Sidecar {
    fn start(channel: Option<&str>, socket: &std::path::Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_braydr-mcp"));
        command
            .env("BRAYDR_IPC_SOCKET", socket)
            .env_remove("BRAYDR_CHANNEL_ID");
        if let Some(channel) = channel {
            command.env("BRAYDR_CHANNEL_ID", channel);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut sidecar = Self {
            child,
            input,
            output,
        };
        let init = sidecar.request(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" },
            }),
        );
        assert!(init["capabilities"]["tools"].is_object());
        sidecar.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        sidecar
    }

    fn send(&mut self, message: Value) {
        writeln!(self.input, "{message}").unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        reply["result"].clone()
    }

    fn tool(&mut self, id: i64, name: &str, args: Value) -> (Value, bool) {
        let result = self.request(id, "tools/call", json!({ "name": name, "arguments": args }));
        let body = result["content"][0]["text"].as_str().unwrap();
        (
            serde_json::from_str(body).unwrap(),
            result["isError"] == true,
        )
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn forwards_tools_and_scopes_to_the_session_channel() {
    let dir = std::env::temp_dir().join(format!("braydr-mcp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("ipc.sock");
    let mut store = Store::in_memory().unwrap();
    let launch = LaunchConfig {
        cwd: ".".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    };
    let channel = store.create_channel("開発", &launch).unwrap().channel_id;
    let listener = braydr_ipc::server::bind(&socket).unwrap();
    let store = Arc::new(Mutex::new(store));
    let shared = store.clone();
    std::thread::spawn(move || braydr_ipc::server::serve(listener, shared));

    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let mut sidecar = Sidecar::start(Some(&channel), &socket);
    let tools = sidecar.request(2, "tools/list", json!({}));
    assert_eq!(tools["tools"].as_array().unwrap().len(), 12);

    let (posted, is_error) = sidecar.tool(
        3,
        "braydr_channel_create_post",
        json!({ "text": "こんにちは" }),
    );
    assert!(!is_error, "{posted}");
    assert_eq!(posted["message"]["channel_id"], channel);
    let posts = store
        .lock()
        .unwrap()
        .list_posts(
            braydr_core::Scope::Main {
                channel_id: &channel,
            },
            None,
            None,
        )
        .unwrap();
    assert_eq!(posts.items[0].text, "こんにちは");

    let (error, is_error) = sidecar.tool(4, "braydr_channel_create_post", json!({ "text": "次" }));
    assert!(is_error);
    assert_eq!(error["error"]["code"], "after_required");

    let mut unbound = Sidecar::start(None, &socket);
    let (error, is_error) = unbound.tool(2, "braydr_channel_get", json!({}));
    assert!(is_error);
    assert_eq!(error["error"]["code"], "channel_not_found");

    let mut offline = Sidecar::start(Some(&channel), &dir.join("missing.sock"));
    let (error, is_error) = offline.tool(2, "braydr_channel_get", json!({}));
    assert!(is_error);
    assert_eq!(error["error"]["code"], "internal");
    std::fs::remove_dir_all(&dir).unwrap();
}
