//! Checks a post cannot end a session stopped at Claude Code's trust prompt, with a real core.
//! Uses no tokens: Claude never gets past the prompt. `HC_SESSION=x HC_SOCKET=... trust_prompt`

use std::sync::Arc;
use std::time::Duration;

use braydr_core::LaunchConfig;
use braydr_herdr::{Client, default_socket};
use braydr_runtime::{Runtime, SessionState};
use braydr_store::Store;

fn main() {
    let cwd = std::env::temp_dir().join(format!("braydr-untrusted-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let core = Arc::new(Client::new(default_socket("default")));
    let rt = Runtime::new(
        Store::in_memory().unwrap(),
        core,
        Box::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
        }),
    );
    let launch = LaunchConfig {
        cwd: cwd.display().to_string(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    };
    let (channel, failure) = rt.create_channel("確認", &launch).unwrap();
    assert!(failure.is_none(), "{failure:?}");
    let id = channel.channel_id;
    std::thread::sleep(Duration::from_secs(5));
    rt.post_main(&id, "こんにちは").unwrap();
    std::thread::sleep(Duration::from_secs(5));
    let state = rt.session_state(&id).unwrap();
    println!("after the post: {state:?}");
    assert_eq!(state, SessionState::Running);
    let session = rt.store().channel(&id).unwrap().session_id.unwrap();
    println!("session {session}");
    std::fs::remove_dir_all(&cwd).unwrap();
}
