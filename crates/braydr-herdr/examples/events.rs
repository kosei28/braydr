//! Checks `events.subscribe` against a real core: `HC_SOCKET=... events`.
//! Starts a terminal, subscribes, kills it, and prints what arrives.

use std::collections::BTreeMap;

use braydr_herdr::{Client, Core, Event, RunSpec, default_socket};

fn main() {
    let client = Client::new(default_socket("default"));
    let labels = BTreeMap::from([("braydr-check".to_owned(), "1".to_owned())]);
    let terminal = client
        .run(&RunSpec {
            line: "sleep 60".into(),
            cwd: "/tmp".into(),
            env: BTreeMap::new(),
            labels: labels.clone(),
        })
        .expect("run");
    println!("started {}", terminal.address);
    let mut events = client.subscribe(&labels).expect("subscribe");
    println!("snapshot: {} terminal(s)", events.snapshot.len());
    client.kill(&terminal.address).expect("kill");
    loop {
        match events.next_event().expect("event") {
            Event::Changed { kind, terminal: t } => {
                println!("changed: {kind} {t:?}");
                if kind == "finished" || kind == "lost" {
                    break;
                }
            }
            Event::Heartbeat => println!("heartbeat"),
            Event::Missed(_) => println!("missed"),
        }
    }
    let state = client.get(&terminal.address).expect("get");
    println!("live after kill: {}", state.is_live());
    client.remove(&terminal.address).expect("remove");
}
