//! A full loop with the real hc and Claude Code, without the app:
//! `e2e <hc session> <workdir> <mcp config> <ipc socket> <db>`. The caller prepares the workdir.

use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use braydr_core::{Author, LaunchConfig, Scope};
use braydr_herdr::{Client, Core, default_socket};
use braydr_runtime::Runtime;
use braydr_store::Store;

fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn wait_for(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(
            start.elapsed() < Duration::from_secs(secs),
            "timed out: {what}"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("ok ({:>5.1?}): {what}", start.elapsed());
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (session, workdir, mcp, socket, db) = (&a[0], &a[1], &a[2], &a[3], &a[4]);
    let core = Arc::new(Client::new(default_socket(session)));
    core.check().expect("herdr-core is running");
    let rt = Runtime::new(Store::open(db).unwrap(), core.clone(), Box::new(clock));
    let listener = braydr_ipc::server::bind(socket.as_ref()).unwrap();
    let handler = rt.tool_handler();
    std::thread::spawn(move || braydr_ipc::server::serve_with(listener, handler));

    let launch = LaunchConfig {
        cwd: workdir.clone(),
        command: format!(
            "claude --mcp-config {mcp} --strict-mcp-config --allowedTools 'mcp__braydr__*' --model haiku"
        ),
        prompt: "/braydr".into(),
    };
    let (channel, failure) = rt.create_channel("検証", &launch).unwrap();
    assert!(failure.is_none(), "{failure:?}");
    let ch = channel.channel_id.clone();
    let address = channel.session_id.clone().unwrap();
    println!("session {address}");

    // A new folder stops at Claude Code's trust prompt. This one is a scratch folder.
    std::thread::sleep(Duration::from_secs(6));
    let screen = |address: &str| {
        let out = Command::new("hc")
            .args(["read", address, "--lines", "30"])
            .env("HC_SESSION", session)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    if screen(&address).contains("trust this folder") {
        Command::new("hc")
            .args(["keys", &address, "down", "enter"])
            .env("HC_SESSION", session)
            .status()
            .unwrap();
        println!("accepted the trust prompt");
    }

    wait_for("the session calls a tool", 90, || {
        !rt.is_silent(&ch) && rt.store().channel_summary(&ch).is_ok() && {
            // `is_silent` needs 60 s to turn on, so look for the call in the agent's screen as well.
            screen(&address).contains("braydr")
        }
    });
    let agent_main = |rt: &Runtime<Client>| {
        rt.store()
            .list_posts(Scope::Main { channel_id: &ch }, None, None)
            .unwrap()
            .items
            .iter()
            .filter(|m| m.author == Author::Agent)
            .count()
    };
    assert_eq!(agent_main(&rt), 0, "an empty channel gets no post");

    let parent = rt.post_main(&ch, "テストです。「了解」とだけ、このメッセージを親にしたスレッドを作らずメインに返信してください。").unwrap();
    wait_for("an agent reply in main", 120, || agent_main(&rt) >= 1);

    let reply = rt
        .reply(
            &ch,
            &parent.message_id,
            "続けて、このスレッドに「OK」とだけ返信し、スレッドは解決してください。",
        )
        .unwrap();
    assert!(reply.created);
    let thread_id = reply.thread.thread_id.clone();
    wait_for("the thread is answered and resolved", 120, || {
        rt.store().thread(&ch, &thread_id).unwrap().resolved_by == Some(Author::Agent)
    });

    let again = rt
        .reply(
            &ch,
            &parent.message_id,
            "もう一度「OK」とだけ返信してください。解決はしないでください。",
        )
        .unwrap();
    assert!(again.reopened);
    wait_for("a reopened thread is answered", 120, || {
        let thread = rt.store().thread(&ch, &thread_id).unwrap();
        thread.last_author == Author::Agent && thread.resolved_at.is_none()
    });

    rt.post_main(
        &ch,
        "新しいセッションで読み直してほしい依頼です。「再開」とだけメインに返信してください。",
    )
    .unwrap();
    let count = agent_main(&rt);
    let next = rt.new_session(&ch, &launch).unwrap();
    println!("new session {}", next.address);
    assert!(!core.get(&address).unwrap().is_live());
    wait_for("the new session catches up from history", 180, || {
        agent_main(&rt) > count
    });

    let store = rt.store();
    for message in store
        .list_posts(Scope::Main { channel_id: &ch }, None, None)
        .unwrap()
        .items
    {
        println!(
            "main {:?}: {}",
            message.author,
            message.text.chars().take(40).collect::<String>()
        );
    }
    for event in store.state_events(&thread_id).unwrap() {
        println!("state {:?} by {:?}", event.action, event.actor);
    }
    println!("E2E PASSED");
    let _ = core.kill(&next.address);
}
