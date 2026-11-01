//! herdr-core's JSON-RPC API (`hc schema`), for the methods Braydr uses.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::{Value, json};

pub const GENERATION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot reach herdr-core: {0}")]
    Io(#[from] io::Error),
    #[error("{kind}: {message}")]
    Rpc { kind: String, message: String },
    #[error("unexpected response: {0}")]
    Protocol(String),
    #[error("herdr-core speaks generation {0}, Braydr needs {GENERATION}")]
    Generation(u32),
    #[error("herdr-core lacks the method {0}")]
    MissingMethod(String),
}

impl Error {
    pub fn kind(&self) -> Option<&str> {
        match self {
            Error::Rpc { kind, .. } => Some(kind),
            _ => None,
        }
    }

    /// The terminal is gone, so retrying cannot help.
    pub fn means_stopped(&self) -> bool {
        matches!(self.kind(), Some("not_found" | "finished" | "unavailable"))
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Terminal {
    pub address: String,
    pub state: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub status: Option<Status>,
}

impl Terminal {
    pub fn is_live(&self) -> bool {
        self.state == "live"
    }

    pub fn activity(&self) -> Activity {
        match self.status.as_ref().map(Status::word) {
            None => Activity::Unknown,
            Some("working") => Activity::Working,
            Some("blocked") => Activity::Blocked,
            Some(_) => Activity::Stopped,
        }
    }
}

/// The program's own word for what it is doing. `terminal.get` sends a record, and the list form
/// a bare word.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Status {
    Record { state: String },
    Word(String),
}

impl Status {
    fn word(&self) -> &str {
        match self {
            Status::Record { state } | Status::Word(state) => state,
        }
    }
}

/// What a live program is doing, as far as it reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Activity {
    /// It has not said, which may be because it cannot, or has not got that far.
    #[default]
    Unknown,
    Working,
    /// Waiting for someone to answer a permission, question, or sign-in prompt.
    Blocked,
    /// Idle, done, or failed: its turn is over.
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub line: String,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
}

/// What the runtime needs from herdr-core. Tests use a fake.
pub trait Core: Send + Sync {
    fn run(&self, spec: &RunSpec) -> Result<Terminal>;
    fn get(&self, terminal: &str) -> Result<Terminal>;
    fn list(&self, labels: &BTreeMap<String, String>) -> Result<Vec<Terminal>>;
    fn send(&self, terminal: &str, text: &str) -> Result<()>;
    fn kill(&self, terminal: &str) -> Result<Terminal>;
    fn remove(&self, terminal: &str) -> Result<()>;
}

/// Follows hc's order, except for the `socket` setting in its config file.
pub fn default_socket(session: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("HC_SOCKET") {
        return path.into();
    }
    let file = format!("{session}.sock");
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Path::new(&dir).join("herdr-core").join(file);
    }
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state")
        });
    state.join("herdr-core").join(session).join(file)
}

struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Connection {
    fn open(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
        })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        writeln!(self.writer, "{line}")?;
        loop {
            let message = self.read()?;
            if message.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(Error::Rpc {
                    kind: error["data"]["kind"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_owned(),
                    message: error["message"].as_str().unwrap_or_default().to_owned(),
                });
            }
            return Ok(message["result"].clone());
        }
    }

    fn read(&mut self) -> Result<Value> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
        }
        serde_json::from_str(&line).map_err(|error| Error::Protocol(error.to_string()))
    }

    fn hello(&mut self, required: &[&str]) -> Result<()> {
        let hello = self.call("hello", json!({ "generation": GENERATION }))?;
        let generation = hello["generation"].as_u64().unwrap_or_default() as u32;
        if generation != GENERATION {
            return Err(Error::Generation(generation));
        }
        let methods: Vec<&str> = hello["methods"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        for method in required {
            if !methods.contains(method) {
                return Err(Error::MissingMethod((*method).to_owned()));
            }
        }
        Ok(())
    }
}

const REQUIRED: &[&str] = &[
    "terminal.run",
    "terminal.get",
    "terminal.list",
    "terminal.send",
    "terminal.kill",
    "terminal.rm",
    "events.subscribe",
];

/// One connection shared by callers, one request at a time.
pub struct Client {
    socket: PathBuf,
    connection: Mutex<Option<Connection>>,
}

impl Client {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
            connection: Mutex::new(None),
        }
    }

    /// Calls once, reconnecting first if the previous connection broke.
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        let mut slot = self
            .connection
            .lock()
            .expect("the connection lock is not poisoned");
        if slot.is_none() {
            let mut connection = Connection::open(&self.socket)?;
            connection.hello(REQUIRED)?;
            *slot = Some(connection);
        }
        let result = slot.as_mut().expect("just connected").call(method, params);
        if matches!(result, Err(Error::Io(_) | Error::Protocol(_))) {
            *slot = None;
        }
        result
    }

    pub fn check(&self) -> Result<()> {
        self.call("terminal.list", json!({})).map(|_| ())
    }

    /// A dedicated connection, as the protocol requires.
    pub fn subscribe(&self, labels: &BTreeMap<String, String>) -> Result<Events> {
        let mut connection = Connection::open(&self.socket)?;
        connection.hello(REQUIRED)?;
        let snapshot = connection.call("events.subscribe", json!({ "labels": labels }))?;
        let terminals = parse(snapshot["terminals"].clone())?;
        Ok(Events {
            connection,
            snapshot: terminals,
        })
    }
}

fn parse<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| Error::Protocol(error.to_string()))
}

impl Core for Client {
    fn run(&self, spec: &RunSpec) -> Result<Terminal> {
        parse(self.call(
            "terminal.run",
            json!({
                "line": spec.line,
                "cwd": spec.cwd,
                "env": spec.env,
                "labels": spec.labels,
                "wait": false,
            }),
        )?)
    }

    fn get(&self, terminal: &str) -> Result<Terminal> {
        parse(self.call("terminal.get", json!({ "terminal": terminal }))?)
    }

    fn list(&self, labels: &BTreeMap<String, String>) -> Result<Vec<Terminal>> {
        parse(self.call("terminal.list", json!({ "labels": labels }))?["terminals"].clone())
    }

    fn send(&self, terminal: &str, text: &str) -> Result<()> {
        self.call(
            "terminal.send",
            json!({ "terminal": terminal, "text": text, "force": true, "wait": false }),
        )
        .map(|_| ())
    }

    fn kill(&self, terminal: &str) -> Result<Terminal> {
        parse(self.call("terminal.kill", json!({ "terminal": terminal }))?)
    }

    fn remove(&self, terminal: &str) -> Result<()> {
        self.call("terminal.rm", json!({ "terminal": terminal }))
            .map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Changed {
        kind: String,
        terminal: Option<String>,
    },
    Heartbeat,
    /// The subscriber fell behind: replace state with this snapshot.
    Missed(Vec<Terminal>),
}

pub struct Events {
    connection: Connection,
    pub snapshot: Vec<Terminal>,
}

impl Events {
    pub fn next_event(&mut self) -> Result<Event> {
        loop {
            let message = self.connection.read()?;
            let params = &message["params"];
            match message["method"].as_str() {
                Some("event") => {
                    return Ok(Event::Changed {
                        kind: params["type"].as_str().unwrap_or_default().to_owned(),
                        terminal: params["terminal"].as_str().map(str::to_owned),
                    });
                }
                Some("heartbeat") => return Ok(Event::Heartbeat),
                Some("missed") => return Ok(Event::Missed(parse(params["terminals"].clone())?)),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_status_in_both_forms() {
        let parse = |status: &str| -> Terminal {
            serde_json::from_str(&format!(
                r#"{{"address":"t1","state":"live","status":{status}}}"#
            ))
            .unwrap()
        };
        assert_eq!(parse("null").activity(), Activity::Unknown);
        assert_eq!(
            parse(r#"{"app":"claude-code","msg":"x","state":"working"}"#).activity(),
            Activity::Working
        );
        assert_eq!(parse(r#""working""#).activity(), Activity::Working);
        assert_eq!(
            parse(r#"{"state":"blocked"}"#).activity(),
            Activity::Blocked
        );
        for word in ["idle", "done", "error"] {
            assert_eq!(
                parse(&format!(r#"{{"state":"{word}"}}"#)).activity(),
                Activity::Stopped
            );
        }
    }
}
