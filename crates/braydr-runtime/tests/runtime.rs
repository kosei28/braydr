use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use braydr_core::launch::in_user_shell;
use braydr_core::*;
use braydr_herdr::{Core, Error, Result, RunSpec, Terminal};
use braydr_ipc::{Request, Response};
use braydr_runtime::{Runtime, SessionError, SessionState};
use braydr_store::Store;
use serde_json::json;

#[derive(Default)]
struct Fake {
    runs: Mutex<Vec<RunSpec>>,
    live: Mutex<BTreeMap<String, Terminal>>,
    sent: Mutex<Vec<(String, String)>>,
    killed: Mutex<Vec<String>>,
    fail_run: Mutex<Option<String>>,
    send_failures: Mutex<Vec<&'static str>>,
    refuse_kill: Mutex<bool>,
}

fn rpc(kind: &str) -> Error {
    Error::Rpc {
        kind: kind.into(),
        message: kind.into(),
    }
}

fn terminal(
    address: &str,
    state: &str,
    labels: BTreeMap<String, String>,
    created_ms: i64,
) -> Terminal {
    Terminal {
        address: address.into(),
        state: state.into(),
        reason: None,
        exit_code: None,
        labels,
        created_ms,
    }
}

impl Core for Fake {
    fn run(&self, spec: &RunSpec) -> Result<Terminal> {
        if let Some(kind) = self.fail_run.lock().unwrap().clone() {
            return Err(rpc(&kind));
        }
        let mut runs = self.runs.lock().unwrap();
        runs.push(spec.clone());
        let t = terminal(
            &format!("t{}", runs.len()),
            "live",
            spec.labels.clone(),
            runs.len() as i64,
        );
        self.live
            .lock()
            .unwrap()
            .insert(t.address.clone(), t.clone());
        Ok(t)
    }
    fn get(&self, address: &str) -> Result<Terminal> {
        self.live
            .lock()
            .unwrap()
            .get(address)
            .cloned()
            .ok_or_else(|| rpc("not_found"))
    }
    fn list(&self, labels: &BTreeMap<String, String>) -> Result<Vec<Terminal>> {
        assert_eq!(labels.get("braydr").map(String::as_str), Some("1"));
        Ok(self.live.lock().unwrap().values().cloned().collect())
    }
    fn send(&self, address: &str, text: &str) -> Result<()> {
        let failure = {
            let mut failures = self.send_failures.lock().unwrap();
            if failures.is_empty() {
                None
            } else {
                Some(failures.remove(0))
            }
        };
        if let Some(kind) = failure {
            return Err(rpc(kind));
        }
        self.sent
            .lock()
            .unwrap()
            .push((address.into(), text.into()));
        Ok(())
    }
    fn kill(&self, address: &str) -> Result<Terminal> {
        self.killed.lock().unwrap().push(address.into());
        if *self.refuse_kill.lock().unwrap() {
            return Ok(terminal(address, "live", BTreeMap::new(), 0));
        }
        match self.live.lock().unwrap().remove(address) {
            Some(t) => Ok(Terminal {
                state: "finished".into(),
                ..t
            }),
            None => Err(rpc("not_found")),
        }
    }
    fn remove(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

fn launch() -> LaunchConfig {
    LaunchConfig {
        cwd: "/repo".into(),
        command: "claude --model opus".into(),
        prompt: "/braydr".into(),
    }
}

struct Env {
    rt: Arc<Runtime<Fake>>,
    core: Arc<Fake>,
    clock: Arc<AtomicI64>,
}

fn env() -> Env {
    let core = Arc::new(Fake::default());
    let clock = Arc::new(AtomicI64::new(1_000_000));
    let time = clock.clone();
    let rt = Runtime::new(
        Store::in_memory().unwrap(),
        core.clone(),
        Box::new(move || time.load(Ordering::SeqCst)),
    );
    Env { rt, core, clock }
}

/// What a session does first when it is up: calls a tool.
fn session_ready(e: &Env, channel_id: &str) {
    let response = e.rt.handle_tool(&Request {
        channel: Some(channel_id.to_owned()),
        tool: "braydr_channel_get".into(),
        args: json!({}),
    });
    assert!(matches!(response, Response::Result(_)));
}

fn wait_sent(core: &Fake, count: usize) -> Vec<(String, String)> {
    let start = Instant::now();
    loop {
        let sent = core.sent.lock().unwrap().clone();
        if sent.len() >= count || start.elapsed() > Duration::from_secs(5) {
            return sent;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn create_starts_a_session_with_the_channel_settings() {
    let e = env();
    let (channel, failure) = e.rt.create_channel("開発", &launch()).unwrap();
    assert!(failure.is_none());
    assert_eq!(channel.session_id.as_deref(), Some("t1"));
    let runs = e.core.runs.lock().unwrap();
    assert_eq!(runs[0].line, in_user_shell("claude --model opus '/braydr'"));
    assert_eq!(runs[0].cwd, "/repo");
    assert_eq!(runs[0].env["BRAYDR_CHANNEL_ID"], channel.channel_id);
    assert_eq!(runs[0].labels["channel"], channel.channel_id);
}

#[test]
fn a_failed_start_keeps_the_channel() {
    let e = env();
    *e.core.fail_run.lock().unwrap() = Some("spawn_failed".into());
    let (channel, failure) = e.rt.create_channel("開発", &launch()).unwrap();
    assert_eq!(failure.unwrap().kind(), Some("spawn_failed"));
    assert_eq!(channel.session_id, None);
    assert!(matches!(
        e.rt.session_state(&channel.channel_id).unwrap(),
        SessionState::Stopped { .. }
    ));
    e.rt.post_main(&channel.channel_id, "保存はされる").unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(e.core.sent.lock().unwrap().is_empty());
}

#[test]
fn notifications_keep_posting_order_and_format() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    session_ready(&e, &ch);
    let parent = e.rt.post_main(&ch, "親").unwrap();
    let first = e.rt.reply(&ch, &parent.message_id, "最初の返信").unwrap();
    assert!(first.created);
    e.rt.store()
        .user_resolve(&ch, &first.thread.thread_id)
        .unwrap();
    let again = e.rt.reply(&ch, &parent.message_id, "追加").unwrap();
    assert!(again.reopened);
    let sent = wait_sent(&e.core, 3);
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|(address, _)| address == "t1"));
    assert!(
        sent[0]
            .1
            .starts_with("[braydr] new post\nchannel_name: 開発\nscope: main\n")
    );
    assert!(sent[0].1.ends_with("---\n親"));
    assert!(sent[1].1.starts_with("[braydr] new thread\n"));
    assert!(sent[2].1.contains("scope: thread") && sent[2].1.contains("reopened: true"));
    assert!(
        !sent[0].1.contains("channel:"),
        "tools do not need the channel ID"
    );
}

#[test]
fn delivery_retries_transient_errors_and_stops_on_a_dead_session() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    session_ready(&e, &channel.channel_id);
    e.core.send_failures.lock().unwrap().push("internal");
    e.rt.post_main(&channel.channel_id, "再送").unwrap();
    assert_eq!(wait_sent(&e.core, 1).len(), 1);
    e.core.send_failures.lock().unwrap().push("finished");
    e.rt.post_main(&channel.channel_id, "届かない").unwrap();
    e.rt.post_main(&channel.channel_id, "次").unwrap();
    let sent = wait_sent(&e.core, 2);
    assert_eq!(sent.len(), 2);
    assert!(
        sent[1].1.ends_with("次"),
        "a stopped session does not get a retry"
    );
}

#[test]
fn new_session_stops_the_old_one_and_saves_settings() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    let next = LaunchConfig {
        cwd: "/other".into(),
        command: "claude".into(),
        prompt: "/braydr 続き".into(),
    };
    let started = e.rt.new_session(&ch, &next).unwrap();
    assert_eq!(started.address, "t2");
    assert_eq!(e.core.killed.lock().unwrap().as_slice(), ["t1"]);
    let saved = e.rt.store().channel(&ch).unwrap();
    assert_eq!(
        (saved.cwd.as_str(), saved.session_id.as_deref()),
        ("/other", Some("t2"))
    );
    assert_eq!(
        e.core.runs.lock().unwrap()[1].line,
        in_user_shell("claude '/braydr 続き'")
    );

    e.core.live.lock().unwrap().clear();
    assert_eq!(
        e.rt.new_session(&ch, &next).unwrap().address,
        "t3",
        "not_found counts as stopped"
    );

    *e.core.refuse_kill.lock().unwrap() = true;
    assert!(matches!(
        e.rt.new_session(&ch, &launch()),
        Err(SessionError::StillLive)
    ));
    assert_eq!(
        e.rt.store().channel(&ch).unwrap().session_id.as_deref(),
        Some("t3"),
        "aborted"
    );
}

#[test]
fn reconcile_keeps_the_newest_live_terminal() {
    let e = env();
    let (a, _) = e.rt.create_channel("a", &launch()).unwrap();
    let (b, _) = e.rt.create_channel("b", &launch()).unwrap();
    let labels = BTreeMap::from([
        ("braydr".to_owned(), "1".to_owned()),
        ("channel".to_owned(), a.channel_id.clone()),
    ]);
    {
        let mut live = e.core.live.lock().unwrap();
        live.insert("t9".into(), terminal("t9", "live", labels, 99));
        live.remove("t2");
    }
    e.rt.reconcile().unwrap();
    assert_eq!(
        e.rt.store()
            .channel(&a.channel_id)
            .unwrap()
            .session_id
            .as_deref(),
        Some("t9")
    );
    assert_eq!(
        e.rt.store().channel(&b.channel_id).unwrap().session_id,
        None
    );
    assert_eq!(e.core.killed.lock().unwrap().as_slice(), ["t1"]);
}

#[test]
fn nudges_wait_then_repeat_until_resolved() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    session_ready(&e, &ch);
    let parent = e.rt.post_main(&ch, "依頼").unwrap();
    wait_sent(&e.core, 1);
    let nudges = |core: &Fake| {
        core.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t)| t.starts_with("[braydr] nudge"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let interval = 300_000;
    e.rt.tick(interval).unwrap();
    assert!(nudges(&e.core).is_empty(), "no nudge before the interval");
    e.clock.fetch_add(interval, Ordering::SeqCst);
    e.rt.tick(interval).unwrap();
    let sent = nudges(&e.core);
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0]
            .1
            .contains(&format!("main:\n  last_message_id: {}", parent.message_id))
    );
    assert!(sent[0].1.ends_with("more: false"));
    e.clock.fetch_add(interval, Ordering::SeqCst);
    e.rt.tick(interval).unwrap();
    assert_eq!(nudges(&e.core).len(), 2);

    e.rt.store()
        .agent_create_thread(&ch, &parent.message_id, "対応")
        .unwrap();
    e.clock.fetch_add(interval, Ordering::SeqCst);
    e.rt.tick(interval).unwrap();
    assert_eq!(nudges(&e.core).len(), 2, "a child thread counts as handled");
}

#[test]
fn no_nudge_to_a_stopped_session() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    session_ready(&e, &channel.channel_id);
    e.rt.post_main(&channel.channel_id, "依頼").unwrap();
    wait_sent(&e.core, 1);
    e.core.live.lock().unwrap().clear();
    e.rt.tick(1).unwrap();
    e.clock.fetch_add(10, Ordering::SeqCst);
    e.rt.tick(1).unwrap();
    assert_eq!(e.core.sent.lock().unwrap().len(), 1);
}

#[test]
fn silence_ends_with_any_tool_call() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    assert!(!e.rt.is_silent(&ch));
    e.clock.fetch_add(60_000, Ordering::SeqCst);
    assert!(e.rt.is_silent(&ch));
    let response = e.rt.handle_tool(&Request {
        channel: Some(ch.clone()),
        tool: "braydr_channel_get".into(),
        args: json!({}),
    });
    assert!(matches!(response, Response::Result(_)));
    assert!(!e.rt.is_silent(&ch));
    e.rt.post_main(&ch, "次").unwrap();
    e.clock.fetch_add(59_999, Ordering::SeqCst);
    assert!(!e.rt.is_silent(&ch));
    e.clock.fetch_add(1, Ordering::SeqCst);
    assert!(e.rt.is_silent(&ch));
}

/// Claude Code's trust prompt defaults to "No, exit", so one Enter ends the session.
#[test]
fn nothing_reaches_a_session_before_its_first_tool_call() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    e.rt.post_main(&ch, "早すぎる投稿").unwrap();
    e.clock.fetch_add(60 * 60 * 1000, Ordering::SeqCst);
    e.rt.tick(1).unwrap();
    e.clock.fetch_add(60 * 60 * 1000, Ordering::SeqCst);
    e.rt.tick(1).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.core.sent.lock().unwrap().is_empty());

    session_ready(&e, &ch);
    e.rt.post_main(&ch, "次の投稿").unwrap();
    let sent = wait_sent(&e.core, 1);
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("次の投稿"));
}

#[test]
fn a_new_session_has_to_respond_again_but_a_restart_does_not() {
    let e = env();
    let (channel, _) = e.rt.create_channel("開発", &launch()).unwrap();
    let ch = channel.channel_id;
    session_ready(&e, &ch);

    // An app restart finds the same terminal, which is still ready.
    e.rt.reconcile().unwrap();
    assert!(e.rt.store().session_responded(&ch).unwrap());

    e.rt.new_session(&ch, &launch()).unwrap();
    assert!(!e.rt.store().session_responded(&ch).unwrap());
    e.rt.post_main(&ch, "新しいセッションへ").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(e.core.sent.lock().unwrap().is_empty());
}
