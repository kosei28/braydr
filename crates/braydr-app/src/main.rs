mod overlay;
mod pane_widths;
mod settings;
mod theme;
mod timeline;
mod ui;
mod window_state;
mod workspace;

#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use braydr_core::Settings;
use braydr_herdr::{Client, Event, default_socket};
use braydr_runtime::{Runtime, terminal_labels};
use braydr_store::Store;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    App, AppContext as _, AssetSource, KeyBinding, Menu, MenuItem, SharedString, TitlebarOptions,
    WindowOptions, point, px, size,
};

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

/// Keeps session states current from herdr-core's events, reconnecting when it goes away.
fn watch_events(core: &Client, runtime: &Runtime<Client>) {
    loop {
        match core.subscribe(&terminal_labels()) {
            Ok(mut events) => {
                let _ = runtime.refresh_sessions();
                loop {
                    match events.next_event() {
                        Ok(Event::Heartbeat) => {}
                        Ok(Event::Changed { .. } | Event::Missed(_)) => {
                            let _ = runtime.refresh_sessions();
                        }
                        Err(error) => {
                            tracing::info!(%error, "herdr-core events ended");
                            break;
                        }
                    }
                }
            }
            Err(error) => tracing::debug!(%error, "cannot subscribe to herdr-core"),
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn runtime() -> Arc<Runtime<Client>> {
    let dir = data_dir();
    let store = Store::open(dir.join("braydr.db")).expect("open the database");
    start_core();
    let core = Arc::new(Client::new(default_socket("default")));
    let runtime = Runtime::new(store, core.clone(), Box::new(clock));
    if let Err(error) = runtime.reconcile() {
        tracing::warn!(?error, "cannot reconcile sessions");
    }
    let socket = dir.join("ipc.sock");
    match braydr_ipc::server::bind(&socket) {
        Ok(listener) => {
            runtime.set_ipc_socket(socket);
            let handler = runtime.tool_handler();
            std::thread::spawn(move || braydr_ipc::server::serve_with(listener, handler));
        }
        Err(error) => tracing::error!(%error, "cannot bind the tool socket"),
    }
    let ticking = runtime.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(5));
            // Events can be missed while the core restarts, so also check periodically.
            if let Err(error) = ticking.refresh_sessions() {
                tracing::warn!(?error, "cannot refresh sessions");
            }
            let minutes = ticking
                .store()
                .settings()
                .map_or(Settings::default().nudge_minutes, |s| s.nudge_minutes);
            if let Err(error) = ticking.tick(minutes * 60 * 1000) {
                tracing::warn!(?error, "nudge tick failed");
            }
        }
    });
    let watching = runtime.clone();
    std::thread::spawn(move || watch_events(&core, &watching));
    runtime
}

/// Lucide icons the kit does not bundle by default, on top of its assets.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        let extra: Option<&'static [u8]> = match path {
            "icons/corner-up-left.svg" => {
                Some(include_bytes!("../assets/icons/corner-up-left.svg"))
            }
            "icons/inbox.svg" => Some(include_bytes!("../assets/icons/inbox.svg")),
            "icons/loader-circle.svg" => Some(include_bytes!("../assets/icons/loader-circle.svg")),
            "icons/pencil.svg" => Some(include_bytes!("../assets/icons/pencil.svg")),
            "icons/rotate-ccw.svg" => Some(include_bytes!("../assets/icons/rotate-ccw.svg")),
            "icons/trash-2.svg" => Some(include_bytes!("../assets/icons/trash-2.svg")),
            "icons/x.svg" => Some(include_bytes!("../assets/icons/x.svg")),
            _ => None,
        };
        match extra {
            Some(bytes) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

const FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/IBMPlexSansJP-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSansJP-Medium.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSansJP-SemiBold.ttf"),
    include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
];

gpui_kit::actions!(braydr, [Quit]);

/// Light only, with the design's fonts and quiet placeholders.
fn apply_theme(cx: &mut App) {
    if let Err(error) = cx
        .text_system()
        .add_fonts(FONTS.iter().map(|font| Cow::Borrowed(*font)).collect())
    {
        tracing::error!(%error, "cannot load the fonts");
    }
    Theme::change(ThemeMode::Light, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = theme::SANS.into();
    theme.mono_font_family = theme::MONO.into();
    theme.mono_font_size = px(12.5);
    theme.foreground = theme::ink().into();
    theme.muted_foreground = theme::ink4().into();
    theme.caret = theme::ink().into();
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-enter", workspace::Send, Some("Input")),
        KeyBinding::new("escape", workspace::Dismiss, None),
        KeyBinding::new("cmd-f", workspace::FindInChannel, None),
        KeyBinding::new("cmd-,", workspace::OpenSettings, None),
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

fn main() {
    let runtime = runtime();
    gpui_kit::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            apply_theme(cx);
            bind_keys(cx);
            let settings_window = Rc::new(RefCell::new(None));
            let settings_runtime = runtime.clone();
            cx.on_action(move |_: &workspace::OpenSettings, cx| {
                settings::open(&settings_runtime, &settings_window, cx);
            });
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([Menu::new("Braydr").items([
                MenuItem::action("設定…", workspace::OpenSettings),
                MenuItem::separator(),
                MenuItem::action("Braydr を終了", Quit),
            ])]);
            #[cfg(feature = "screenshot")]
            if std::env::var("BRAYDR_SCREENSHOT_STATE").as_deref() == Ok("settings") {
                settings::open(&runtime, &Rc::new(RefCell::new(None)), cx);
                return;
            }
            let (bounds, display_id) = window_state::restore(&runtime, cx);
            let options = WindowOptions {
                window_bounds: Some(bounds),
                window_min_size: Some(size(
                    px(window_state::MIN_SIZE.0),
                    px(window_state::MIN_SIZE.1),
                )),
                display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("Braydr".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(23.))),
                }),
                app_owns_titlebar_drag: true,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                // The app has one main window; notifications need it, so closing it quits.
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                cx.new(|cx| workspace::Workspace::new(runtime.clone(), window, cx))
            })
            .expect("open the window");
            cx.activate(true);
        });
}
