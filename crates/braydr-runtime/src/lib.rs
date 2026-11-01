//! Sessions, delivery, and nudges, without any UI (`docs/product-spec.md` 7, 8, and 10).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use braydr_core::launch::{ENV_CHANNEL_ID, shell_line};
use braydr_core::notification::{self, NudgeMain, NudgeThread};
use braydr_core::schedule::{NudgeTimer, ResponseWatch};
use braydr_core::*;
use braydr_herdr::{Core, RunSpec, Terminal};
use braydr_ipc::{Request, Response, server, views};
use braydr_store::{Store, UserReply};

const NUDGE_THREAD_LIMIT: u32 = 20;
const SEND_ATTEMPTS: u32 = 3;

#[derive(Debug)]
pub enum SessionError {
    Store(Error),
    Core(braydr_herdr::Error),
    StillLive,
}

impl From<Error> for SessionError {
    fn from(error: Error) -> Self {
        SessionError::Store(error)
    }
}

impl From<braydr_herdr::Error> for SessionError {
    fn from(error: braydr_herdr::Error) -> Self {
        SessionError::Core(error)
    }
}

pub type SessionResult<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Stopped {
        reason: Option<String>,
        exit_code: Option<i64>,
    },
}

#[derive(Default)]
struct ChannelRuntime {
    outbox: Option<Sender<String>>,
    nudge: NudgeTimer,
    watch: ResponseWatch,
}

const STOPPED: SessionState = SessionState::Stopped {
    reason: None,
    exit_code: None,
};

/// The labels on every Braydr terminal, for subscribing to their events.
pub fn terminal_labels() -> BTreeMap<String, String> {
    labels_for(None)
}

pub struct Runtime<C: Core + 'static> {
    store: Arc<Mutex<Store>>,
    core: Arc<C>,
    channels: Mutex<HashMap<String, ChannelRuntime>>,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
    retry_delay: Duration,
    revision: AtomicU64,
    states: Mutex<HashMap<String, SessionState>>,
    start_failed: Mutex<HashSet<String>>,
}

fn labels_for(channel_id: Option<&str>) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::from([("braydr".to_owned(), "1".to_owned())]);
    if let Some(channel_id) = channel_id {
        labels.insert("channel".to_owned(), channel_id.to_owned());
    }
    labels
}

impl<C: Core + 'static> Runtime<C> {
    pub fn new(store: Store, core: Arc<C>, clock: Box<dyn Fn() -> i64 + Send + Sync>) -> Arc<Self> {
        Arc::new(Self {
            store: Arc::new(Mutex::new(store)),
            core,
            channels: Mutex::new(HashMap::new()),
            clock,
            retry_delay: Duration::from_millis(500),
            revision: AtomicU64::new(1),
            states: Mutex::new(HashMap::new()),
            start_failed: Mutex::new(HashSet::new()),
        })
    }

    /// Changes whenever data may have changed. The UI polls it.
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Relaxed)
    }

    /// Tells the UI to read the data again.
    pub fn bump(&self) {
        self.revision.fetch_add(1, Ordering::Relaxed);
    }

    pub fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().expect("the store lock is not poisoned")
    }

    fn with_channel<T>(&self, channel_id: &str, body: impl FnOnce(&mut ChannelRuntime) -> T) -> T {
        let mut channels = self
            .channels
            .lock()
            .expect("the channels lock is not poisoned");
        body(channels.entry(channel_id.to_owned()).or_default())
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    // Sessions

    /// Creates the channel even if its session fails to start.
    pub fn create_channel(
        &self,
        name: &str,
        launch: &LaunchConfig,
    ) -> Result<(Channel, Option<braydr_herdr::Error>)> {
        let channel = self.store().create_channel(name, launch)?;
        let failure = self
            .start(&channel.channel_id)
            .err()
            .and_then(|error| match error {
                SessionError::Core(error) => Some(error),
                _ => None,
            });
        self.bump();
        Ok((self.store().channel(&channel.channel_id)?, failure))
    }

    fn start(&self, channel_id: &str) -> SessionResult<Terminal> {
        let channel = self.store().channel(channel_id)?;
        let spec = RunSpec {
            line: shell_line(&channel.command, &channel.prompt),
            cwd: channel.cwd,
            env: BTreeMap::from([(ENV_CHANNEL_ID.to_owned(), channel_id.to_owned())]),
            labels: labels_for(Some(channel_id)),
        };
        let terminal = match self.core.run(&spec) {
            Ok(terminal) => terminal,
            Err(error) => {
                self.set_cached_state(channel_id, STOPPED);
                self.start_failed
                    .lock()
                    .expect("the start_failed lock is not poisoned")
                    .insert(channel_id.to_owned());
                return Err(error.into());
            }
        };
        self.store()
            .set_session(channel_id, Some(&terminal.address))?;
        self.start_failed
            .lock()
            .expect("the start_failed lock is not poisoned")
            .remove(channel_id);
        self.set_cached_state(channel_id, SessionState::Running);
        let now = self.now();
        self.with_channel(channel_id, |state| state.watch.expect(now));
        Ok(terminal)
    }

    fn set_cached_state(&self, channel_id: &str, state: SessionState) {
        let previous = self
            .states
            .lock()
            .expect("the states lock is not poisoned")
            .insert(channel_id.to_owned(), state.clone());
        if previous.as_ref() != Some(&state) {
            self.bump();
        }
    }

    /// The last known state, without asking herdr-core. Safe to call while rendering.
    pub fn cached_state(&self, channel_id: &str) -> SessionState {
        self.states
            .lock()
            .expect("the states lock is not poisoned")
            .get(channel_id)
            .cloned()
            .unwrap_or(STOPPED)
    }

    /// Whether the last attempt to start this channel's session failed.
    pub fn start_failed(&self, channel_id: &str) -> bool {
        self.start_failed
            .lock()
            .expect("the start_failed lock is not poisoned")
            .contains(channel_id)
    }

    /// Asks herdr-core for every channel's state. An unreachable core counts as stopped.
    pub fn refresh_sessions(&self) -> SessionResult<()> {
        let channels = self.store().channels()?;
        for channel in channels {
            let state = self
                .session_state(&channel.channel_id)
                .unwrap_or_else(|error| {
                    tracing::warn!(
                        channel_id = channel.channel_id,
                        ?error,
                        "cannot read the session"
                    );
                    STOPPED
                });
            self.set_cached_state(&channel.channel_id, state);
        }
        Ok(())
    }

    /// `docs/product-spec.md` 8.2.
    pub fn new_session(&self, channel_id: &str, launch: &LaunchConfig) -> SessionResult<Terminal> {
        let current = self.store().channel(channel_id)?.session_id;
        if let Some(address) = current {
            match self.core.kill(&address) {
                Ok(terminal) if terminal.is_live() => return Err(SessionError::StillLive),
                Ok(_) => {}
                Err(error) if error.kind() == Some("not_found") => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.store().prepare_new_session(channel_id, launch)?;
        self.with_channel(channel_id, |state| state.watch.tool_called());
        self.start(channel_id)
    }

    pub fn session_state(&self, channel_id: &str) -> SessionResult<SessionState> {
        let Some(address) = self.store().channel(channel_id)?.session_id else {
            return Ok(STOPPED);
        };
        match self.core.get(&address) {
            Ok(terminal) if terminal.is_live() => Ok(SessionState::Running),
            Ok(terminal) => Ok(SessionState::Stopped {
                reason: terminal.reason,
                exit_code: terminal.exit_code,
            }),
            Err(error) if error.kind() == Some("not_found") => Ok(STOPPED),
            Err(error) => Err(error.into()),
        }
    }

    /// Matches live terminals to channels on startup. Keeps the newest live one per channel.
    pub fn reconcile(&self) -> SessionResult<()> {
        let terminals = self.core.list(&labels_for(None))?;
        let mut newest: HashMap<String, Terminal> = HashMap::new();
        for terminal in terminals.into_iter().filter(Terminal::is_live) {
            let Some(channel_id) = terminal.labels.get("channel").cloned() else {
                continue;
            };
            match newest.get(&channel_id) {
                Some(kept) if kept.created_ms >= terminal.created_ms => {
                    let _ = self.core.kill(&terminal.address);
                }
                Some(kept) => {
                    let _ = self.core.kill(&kept.address);
                    newest.insert(channel_id, terminal);
                }
                None => {
                    newest.insert(channel_id, terminal);
                }
            }
        }
        let channels = self.store().channels()?;
        for channel in channels {
            let address = newest
                .get(&channel.channel_id)
                .map(|terminal| terminal.address.as_str());
            self.store().set_session(&channel.channel_id, address)?;
        }
        self.refresh_sessions()
    }

    // Posts

    pub fn post_main(self: &Arc<Self>, channel_id: &str, text: &str) -> Result<Message> {
        let (message, name) = {
            let mut store = self.store();
            let message = store.user_post_main(channel_id, text)?;
            (message, store.channel(channel_id)?.name)
        };
        self.deliver(channel_id, notification::new_post_main(&name, &message));
        self.bump();
        Ok(message)
    }

    pub fn reply(
        self: &Arc<Self>,
        channel_id: &str,
        parent_message_id: &str,
        text: &str,
    ) -> Result<UserReply> {
        let (reply, name) = {
            let mut store = self.store();
            let reply = store.user_reply(channel_id, parent_message_id, text)?;
            (reply, store.channel(channel_id)?.name)
        };
        let text = if reply.created {
            notification::new_thread(
                &name,
                &reply.thread.thread_id,
                parent_message_id,
                &reply.message,
            )
        } else {
            notification::new_post_thread(
                &name,
                &reply.thread.thread_id,
                parent_message_id,
                &reply.message,
                reply.reopened,
            )
        };
        self.deliver(channel_id, text);
        self.bump();
        Ok(reply)
    }

    /// Per-channel queue, so notifications keep posting order. Sends skip a stopped session; the
    /// next session reads the history when it starts.
    fn deliver(self: &Arc<Self>, channel_id: &str, text: String) {
        let now = self.now();
        let this = self.clone();
        self.with_channel(channel_id, |state| {
            state.watch.expect(now);
            let outbox = state
                .outbox
                .get_or_insert_with(|| this.spawn_outbox(channel_id.to_owned()));
            if outbox.send(text).is_err() {
                state.outbox = None;
            }
        });
    }

    fn spawn_outbox(self: &Arc<Self>, channel_id: String) -> Sender<String> {
        let (sender, receiver) = channel::<String>();
        let this = self.clone();
        std::thread::spawn(move || {
            for text in receiver {
                this.send_with_retry(&channel_id, &text);
            }
        });
        sender
    }

    fn send_with_retry(&self, channel_id: &str, text: &str) {
        let Ok(Some(address)) = self
            .store()
            .channel(channel_id)
            .map(|channel| channel.session_id)
        else {
            return;
        };
        for attempt in 1..=SEND_ATTEMPTS {
            match self.core.send(&address, text) {
                Ok(()) => return,
                Err(error) if error.means_stopped() => {
                    tracing::info!(channel_id, %error, "session stopped; not delivered");
                    return;
                }
                Err(error) => {
                    tracing::warn!(channel_id, attempt, %error, "delivery failed");
                    if attempt < SEND_ATTEMPTS {
                        std::thread::sleep(self.retry_delay * attempt);
                    }
                }
            }
        }
    }

    // Periodic work

    /// Sends nudges that are due. Call this periodically.
    pub fn tick(&self, interval_ms: i64) -> SessionResult<()> {
        let now = self.now();
        let channels = self.store().channels()?;
        for channel in channels {
            let (threads, more, main) = {
                let store = self.store();
                let page = store.list_threads(
                    &channel.channel_id,
                    ThreadFilter::AwaitingAgent,
                    None,
                    Some(NUDGE_THREAD_LIMIT),
                )?;
                (
                    page.items,
                    page.has_more,
                    store.main_awaiting_response(&channel.channel_id)?,
                )
            };
            let condition = !threads.is_empty() || main.is_some();
            let due = self.with_channel(&channel.channel_id, |state| {
                state.nudge.tick(condition, now, interval_ms)
            });
            if !due || self.session_state(&channel.channel_id)? != SessionState::Running {
                continue;
            }
            let threads: Vec<NudgeThread> = threads
                .iter()
                .map(|thread| NudgeThread {
                    thread_id: thread.thread_id.clone(),
                    last_message_id: thread.last_message_id.clone(),
                    last_message_at: views::iso(thread.last_message_at),
                })
                .collect();
            let main = main.map(|(last_message_id, at)| NudgeMain {
                last_message_id,
                last_message_at: views::iso(at),
            });
            let text = notification::nudge(&channel.name, &threads, main.as_ref(), more);
            let address = channel
                .session_id
                .as_deref()
                .expect("a running session has an address");
            if let Err(error) = self.core.send(address, &text) {
                tracing::warn!(channel_id = channel.channel_id, %error, "nudge failed");
            }
        }
        Ok(())
    }

    /// Whether to show "no response for a while" (`docs/product-spec.md` 4.6).
    pub fn is_silent(&self, channel_id: &str) -> bool {
        let now = self.now();
        self.with_channel(channel_id, |state| state.watch.silent(now))
    }

    // Tools

    /// Serves agent tools, noting that the session responded.
    pub fn handle_tool(&self, request: &Request) -> Response {
        if let Some(channel_id) = request.channel.as_deref() {
            self.with_channel(channel_id, |state| state.watch.tool_called());
        }
        let response = server::handle(&mut self.store(), request);
        self.bump();
        response
    }

    pub fn tool_handler(self: &Arc<Self>) -> server::Handler {
        let this = self.clone();
        Arc::new(move |request: &Request| this.handle_tool(request))
    }
}
