//! Serves the tools without the app: `devserver <db> <socket> [user message]`.
//! Prints a channel ID on stdout.

use std::sync::{Arc, Mutex};

use braydr_core::LaunchConfig;
use braydr_store::Store;

fn main() {
    let mut args = std::env::args().skip(1);
    let (db, socket) = (
        args.next().expect("db path"),
        args.next().expect("socket path"),
    );
    let mut store = Store::open(db).expect("open the store");
    let launch = LaunchConfig {
        cwd: ".".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    };
    let channel = store
        .create_channel("開発", &launch)
        .expect("create a channel");
    if let Some(text) = args.next() {
        store
            .user_post_main(&channel.channel_id, &text)
            .expect("post");
    }
    println!("{}", channel.channel_id);
    let listener = braydr_ipc::server::bind(socket.as_ref()).expect("bind");
    braydr_ipc::server::serve(listener, Arc::new(Mutex::new(store)));
}
