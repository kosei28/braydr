mod theme;
mod workspace;

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use braydr_herdr::{Client, default_socket};
use braydr_runtime::Runtime;
use braydr_store::Store;
use gpui_kit::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, point,
    px, size,
};

const NUDGE_INTERVAL_MS: i64 = 5 * 60 * 1000;

fn data_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let dir = std::env::var_os("BRAYDR_DATA_DIR").map_or_else(
        || home.join("Library/Application Support/Braydr"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir).expect("create the data directory");
    dir
}

fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_millis() as i64
}

/// Apps launched from Finder do not get the shell's PATH.
fn start_core() {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let hc = home.join(".local/bin/hc");
    let program = if hc.exists() { hc } else { PathBuf::from("hc") };
    if let Err(error) = Command::new(program).args(["server", "start"]).output() {
        tracing::warn!(%error, "cannot start herdr-core");
    }
}

fn runtime() -> Arc<Runtime<Client>> {
    let dir = data_dir();
    let store = Store::open(dir.join("braydr.db")).expect("open the database");
    start_core();
    let core = Arc::new(Client::new(default_socket("default")));
    let runtime = Runtime::new(store, core, Box::new(clock));
    if let Err(error) = runtime.reconcile() {
        tracing::warn!(?error, "cannot reconcile sessions");
    }
    match braydr_ipc::server::bind(&dir.join("ipc.sock")) {
        Ok(listener) => {
            let handler = runtime.tool_handler();
            std::thread::spawn(move || braydr_ipc::server::serve_with(listener, handler));
        }
        Err(error) => tracing::error!(%error, "cannot bind the tool socket"),
    }
    let ticking = runtime.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            if let Err(error) = ticking.tick(NUDGE_INTERVAL_MS) {
                tracing::warn!(?error, "nudge tick failed");
            }
        }
    });
    runtime
}

fn main() {
    let runtime = runtime();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            cx.bind_keys([KeyBinding::new("cmd-enter", workspace::Send, Some("Input"))]);
            let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Braydr".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(23.))),
                }),
                app_owns_titlebar_drag: true,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| workspace::Workspace::new(runtime.clone(), window, cx))
            })
            .expect("open the window");
            cx.activate(true);
        });
}
