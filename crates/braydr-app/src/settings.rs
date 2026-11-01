//! The settings window (`docs/design-system.md` 4).

use std::cell::RefCell;
use std::rc::Rc;

use braydr_core::Settings;
use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, Styled as _,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, point, px, size,
};

use crate::timeline::Rt;
use crate::{theme, ui};

const INTERVALS: [(&str, i64); 7] = [
    ("1 分ごと", 1),
    ("3 分ごと", 3),
    ("5 分ごと", 5),
    ("10 分ごと", 10),
    ("15 分ごと", 15),
    ("30 分ごと", 30),
    ("60 分ごと", 60),
];

pub struct SettingsView {
    rt: Rt,
    command: Entity<TextareaState>,
    prompt: Entity<TextareaState>,
    interval: Entity<SelectState<Vec<&'static str>>>,
}

impl SettingsView {
    pub fn new(rt: Rt, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = rt.store().settings().unwrap_or_default();
        let command = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 4)
                .default_value(settings.default_command.clone())
        });
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .default_value(settings.default_prompt.clone())
        });
        let selected = INTERVALS
            .iter()
            .position(|(_, minutes)| *minutes == settings.nudge_minutes)
            .unwrap_or(2);
        let labels: Vec<&'static str> = INTERVALS.iter().map(|(label, _)| *label).collect();
        let interval = cx.new(|cx| {
            SelectState::new(
                labels,
                Some(gpui_kit::component::IndexPath::new(selected)),
                window,
                cx,
            )
        });
        cx.subscribe(&command, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.save(cx);
            }
        })
        .detach();
        cx.subscribe(&prompt, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.save(cx);
            }
        })
        .detach();
        cx.subscribe(
            &interval,
            |this, _, _: &SelectEvent<Vec<&'static str>>, cx| {
                this.save(cx);
            },
        )
        .detach();
        #[cfg(feature = "screenshot")]
        if let Some(path) = std::env::var_os("BRAYDR_SCREENSHOT") {
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(2))
                    .await;
                let _ = cx.update(|window, cx| {
                    if let Ok(image) = window.render_to_image() {
                        let _ = image.save(&path);
                    }
                    cx.quit();
                });
            })
            .detach();
        }
        Self {
            rt,
            command,
            prompt,
            interval,
        }
    }

    /// Saved as they change. A blank command keeps the last usable one.
    fn save(&mut self, cx: &mut Context<Self>) {
        let mut settings = self.rt.store().settings().unwrap_or_default();
        let command = self.command.read(cx).value().trim().to_owned();
        if !command.is_empty() {
            settings.default_command = command;
        }
        settings.default_prompt = self.prompt.read(cx).value().trim().to_owned();
        let label = self.interval.read(cx).selected_value().copied();
        if let Some((_, minutes)) = INTERVALS.iter().find(|(name, _)| Some(*name) == label) {
            settings.nudge_minutes = *minutes;
        }
        if let Err(error) = self.rt.store().save_settings(&settings) {
            tracing::warn!(%error, "cannot save the settings");
        }
    }
}

fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::ink())
        .child(text)
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = |state: &Entity<TextareaState>| {
            ui::field()
                .px(px(8.))
                .font_family(theme::MONO)
                .child(ui::plain_textarea(state, 12.5))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(24.))
            .pt(px(48.))
            .pb(px(24.))
            .bg(theme::sheet())
            .font_family(theme::SANS)
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(label("起動コマンド"))
            .child(text(&self.command))
            .child(div().h(px(8.)))
            .child(label("プロンプト"))
            .child(text(&self.prompt))
            .child(
                div()
                    .pt(px(8.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(label("返信がないエージェントへの再通知"))
                    .child(div().w(px(120.)).child(Select::new(&self.interval))),
            )
    }
}

/// Opens the settings window, or brings the open one forward.
pub fn open(rt: &Rt, open_window: &Rc<RefCell<Option<AnyWindowHandle>>>, cx: &mut App) {
    if let Some(handle) = *open_window.borrow()
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let bounds = Bounds::centered(None, size(px(560.), px(320.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("設定".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(18.), px(18.))),
        }),
        app_owns_titlebar_drag: true,
        is_resizable: false,
        ..Default::default()
    };
    let rt = rt.clone();
    match gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsView::new(rt, window, cx))
    }) {
        Ok((handle, _)) => *open_window.borrow_mut() = Some(handle),
        Err(error) => tracing::error!(%error, "cannot open the settings"),
    }
}

/// What a new channel starts with.
pub fn defaults(rt: &Rt) -> Settings {
    rt.store().settings().unwrap_or_default()
}
