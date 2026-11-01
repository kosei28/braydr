//! The window: sidebar, main sheet, and right sheet (`docs/design-system.md` 3 and 4).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use braydr_core::notify::{self, View};
use braydr_core::text::head;
use braydr_core::{Author, LaunchConfig, Message, StateAction, Thread, validate};
use braydr_herdr::Activity;
use braydr_runtime::SessionState;
use gpui_kit::component::input::{self, Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, AppContext as _, ClipboardItem, Context, CursorStyle,
    DispatchPhase, Div, Entity, FocusHandle, Focusable as _, FontWeight, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    SystemNotification, Transformation, Window, canvas, div, list, percentage, px,
};

use crate::overlay::{LaunchFields, Overlay};
use crate::pane_widths::{Edge, Frame, PaneWidths};
use crate::timeline::{OwnedScope, Row, Rt, ThreadLink, Timeline};
use crate::{settings, theme, ui, window_state};

gpui_kit::actions!(braydr, [Send, Dismiss, OpenSettings, FindInChannel]);

const BODY_LINE_HEIGHT: f32 = 23.;
const NOTIFICATION_CHARS: usize = 120;
/// A message's room inside its hover and highlight area. Lists take it off their own padding, so
/// the text stays on the same axis as the rest of the sheet.
const MESSAGE_INSET_X: f32 = 10.;
const MESSAGE_INSET_Y: f32 = 7.;
const COPIED_FOR: Duration = Duration::from_millis(1500);

/// The right sheet: a thread, or a draft reply before the thread exists.
pub struct ThreadPane {
    channel_id: String,
    parent: Message,
    thread: Option<Thread>,
    pub timeline: Option<Timeline>,
    draft_scroll: ScrollHandle,
    input: Entity<TextareaState>,
    just_reopened: bool,
    send_failed: bool,
}

pub struct ChannelRow {
    pub channel: braydr_core::Channel,
    pub state: SessionState,
    pub stopped: bool,
    blocked: bool,
    pub start_failed: bool,
    unresolved: i64,
    unread: bool,
    unresolved_unread: bool,
}

pub struct CreateForm {
    pub name: Entity<InputState>,
    pub fields: LaunchFields,
    error: Option<String>,
}

/// A drag on a pane's edge, measured from where it started.
#[derive(Clone, Copy)]
struct Resize {
    edge: Edge,
    start_x: f32,
    start_width: f32,
}

const EDGE_WIDTH: f32 = 8.;

pub struct Workspace {
    pub rt: Rt,
    revision: u64,
    channels: Vec<ChannelRow>,
    pub selected: Option<String>,
    pub timeline: Option<Timeline>,
    drafts: HashMap<String, Entity<TextareaState>>,
    send_failed: bool,
    pub pane: Option<ThreadPane>,
    pub create: Option<CreateForm>,
    pub overlay: Option<Overlay>,
    pub sidebar_open: bool,
    pub widths: PaneWidths,
    /// `widths` fitted to the window, as last drawn.
    pub shown: PaneWidths,
    resize: Option<Resize>,
    pub pointer_down: bool,
    bounds_changed: bool,
    highlight: Option<String>,
    pub copied: Option<String>,
    copy_generation: u64,
    announced_through: Option<String>,
    pub notified: HashSet<String>,
    focus: FocusHandle,
}

impl Workspace {
    pub fn new(rt: Rt, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let announced_through = rt.store().newest_message_id().ok().flatten();
        let widths = PaneWidths::load(&rt);
        let mut this = Self {
            rt,
            revision: 0,
            channels: Vec::new(),
            selected: None,
            timeline: None,
            drafts: HashMap::new(),
            send_failed: false,
            pane: None,
            create: None,
            overlay: None,
            sidebar_open: true,
            widths,
            shown: widths,
            resize: None,
            pointer_down: false,
            bounds_changed: false,
            highlight: None,
            copied: None,
            copy_generation: 0,
            announced_through,
            notified: HashSet::new(),
            focus: cx.focus_handle(),
        };
        this.reload_channels();
        if let Some(first) = this
            .channels
            .first()
            .map(|row| row.channel.channel_id.clone())
        {
            this.select(first, window, cx);
        } else {
            // With no channels, creating one is the only thing to do.
            this.start_create(window, cx);
        }
        #[cfg(feature = "screenshot")]
        this.schedule_screenshot(window, cx);
        // Agents write through the tool socket on another thread.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        // Reaching a timeline's bottom counts only while the window has focus.
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        // A drag reports every step, so the poll saves the latest rather than each one.
        cx.observe_window_bounds(window, |this, _, _| this.bounds_changed = true)
            .detach();
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        cx.on_system_notification_response(move |response, cx| {
            let tag = response.tag.to_string();
            let weak = weak.clone();
            let _ = handle.update(cx, |_, window, cx| {
                window.activate_window();
                let _ = weak.update(cx, |this, cx| this.open_tag(&tag, window, cx));
            });
        });
        this
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.bounds_changed) {
            window_state::save(&self.rt, window, cx);
        }
        let revision = self.rt.revision();
        if revision != self.revision {
            self.revision = revision;
            self.announce(window, cx);
            self.reload(cx);
            // With no unresolved threads left, the button that opened the list is gone too.
            if self
                .overlay
                .as_ref()
                .is_some_and(Overlay::is_empty_unresolved)
            {
                self.close_overlay(window, cx);
            }
        }
    }

    pub fn selected_row(&self) -> Option<&ChannelRow> {
        self.channels
            .iter()
            .find(|row| Some(&row.channel.channel_id) == self.selected.as_ref())
    }

    fn reload_channels(&mut self) {
        let Ok(channels) = self.rt.store().channels() else {
            return;
        };
        self.channels = channels
            .into_iter()
            .map(|channel| {
                let id = channel.channel_id.clone();
                let store = self.rt.store();
                let unresolved = store.unresolved_count(&id).unwrap_or(0);
                let unread = store
                    .has_unread(braydr_core::Scope::Main { channel_id: &id })
                    .unwrap_or(false)
                    || store.has_unread_thread(&id, false).unwrap_or(false);
                let unresolved_unread = store.has_unread_thread(&id, true).unwrap_or(false);
                drop(store);
                let state = self.rt.cached_state(&id);
                ChannelRow {
                    stopped: state != SessionState::Running,
                    blocked: self.rt.activity(&id) == Activity::Blocked,
                    start_failed: self.rt.start_failed(&id),
                    state,
                    channel,
                    unresolved,
                    unread,
                    unresolved_unread,
                }
            })
            .collect();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_channels();
        let rt = self.rt.clone();
        if let Some(timeline) = &mut self.timeline {
            timeline.reload(&rt);
        }
        if let Some(pane) = &mut self.pane {
            pane.thread = match &pane.thread {
                Some(thread) => rt.store().thread(&pane.channel_id, &thread.thread_id).ok(),
                None => rt
                    .store()
                    .thread_for_parent(&pane.channel_id, &pane.parent.message_id)
                    .ok()
                    .flatten(),
            };
            if let (Some(thread), None) = (&pane.thread, &pane.timeline) {
                pane.timeline = Some(Timeline::open(
                    &rt,
                    OwnedScope::Thread(thread.thread_id.clone()),
                    Some(pane.parent.clone()),
                ));
            } else if let Some(timeline) = &mut pane.timeline {
                timeline.reload(&rt);
            }
        }
        self.reload_overlay(cx);
        cx.notify();
    }

    fn select(&mut self, channel_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.create = None;
        self.pane = None;
        self.overlay = None;
        self.send_failed = false;
        self.timeline = Some(Timeline::open(
            &self.rt,
            OwnedScope::Main(channel_id.clone()),
            None,
        ));
        let opened = OwnedScope::Main(channel_id.clone());
        self.selected = Some(channel_id);
        let draft = self.current_draft(window, cx);
        draft.update(cx, |state, cx| state.focus(window, cx));
        self.dismiss_opened(&opened, window, cx);
        cx.notify();
    }

    /// Each channel keeps its own unsent text.
    fn current_draft(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let channel_id = self.selected.clone().unwrap_or_default();
        if let Some(draft) = self.drafts.get(&channel_id) {
            return draft.clone();
        }
        let draft = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("メッセージ")
        });
        cx.subscribe(&draft, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        self.drafts.insert(channel_id, draft.clone());
        draft
    }

    pub fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("チャンネル名"));
        cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        let defaults = settings::defaults(&self.rt);
        let fields = LaunchFields::new(
            LaunchConfig {
                cwd: std::env::var("HOME").unwrap_or_default(),
                command: defaults.default_command,
                prompt: defaults.default_prompt,
            },
            window,
            cx,
        );
        cx.subscribe(&fields.command, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        name.update(cx, |state, cx| state.focus(window, cx));
        self.selected = None;
        self.timeline = None;
        self.pane = None;
        self.overlay = None;
        self.create = Some(CreateForm {
            name,
            fields,
            error: None,
        });
        cx.notify();
    }

    fn submit_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &self.create else { return };
        let name = form.name.read(cx).value().to_string();
        let launch = form.fields.read(cx);
        match self.rt.create_channel(&name, &launch) {
            Ok((channel, failure)) => {
                if let Some(failure) = failure {
                    tracing::warn!(%failure, "session did not start");
                }
                self.reload_channels();
                self.select(channel.channel_id, window, cx);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot create the channel");
                if let Some(form) = &mut self.create {
                    form.error = Some(error.to_string());
                }
                cx.notify();
            }
        }
    }

    fn send_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected.clone() else {
            return;
        };
        let input = self.current_draft(window, cx);
        let text = input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        match self.rt.post_main(&channel_id, &text) {
            Ok(_) => {
                self.send_failed = false;
                input.update(cx, |state, cx| state.set_value("", window, cx));
                if let Some(timeline) = &self.timeline {
                    timeline.list.scroll_to_end();
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot post");
                self.send_failed = true;
            }
        }
        cx.notify();
    }

    fn send_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = &mut self.pane else { return };
        let text = pane.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let input = pane.input.clone();
        match self
            .rt
            .reply(&pane.channel_id, &pane.parent.message_id, &text)
        {
            Ok(_) => {
                pane.send_failed = false;
                pane.just_reopened = false;
                input.update(cx, |state, cx| state.set_value("", window, cx));
                if let Some(timeline) = &pane.timeline {
                    timeline.list.scroll_to_end();
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot reply");
                pane.send_failed = true;
            }
        }
        cx.notify();
    }

    pub fn open_thread(&mut self, parent: Message, window: &mut Window, cx: &mut Context<Self>) {
        let channel_id = parent.channel_id.clone();
        let thread = self
            .rt
            .store()
            .thread_for_parent(&channel_id, &parent.message_id)
            .ok()
            .flatten();
        let timeline = thread.as_ref().map(|t| {
            Timeline::open(
                &self.rt,
                OwnedScope::Thread(t.thread_id.clone()),
                Some(parent.clone()),
            )
        });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder("返信")
        });
        cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        input.update(cx, |state, cx| state.focus(window, cx));
        let opened = thread
            .as_ref()
            .map(|t| OwnedScope::Thread(t.thread_id.clone()));
        self.pane = Some(ThreadPane {
            channel_id,
            parent,
            thread,
            timeline,
            draft_scroll: ScrollHandle::new(),
            input,
            just_reopened: false,
            send_failed: false,
        });
        if let Some(opened) = opened {
            self.dismiss_opened(&opened, window, cx);
        }
        cx.notify();
    }

    fn close_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pane = None;
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn toggle_resolved(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = &mut self.pane else { return };
        let Some(thread) = &pane.thread else { return };
        let reopening = thread.resolved_at.is_some();
        let mut store = self.rt.store();
        let result = if reopening {
            store.user_reopen(&pane.channel_id, &thread.thread_id)
        } else {
            store.user_resolve(&pane.channel_id, &thread.thread_id)
        };
        drop(store);
        match result {
            Ok(_) => pane.just_reopened = reopening,
            Err(error) => tracing::warn!(%error, "cannot change the thread state"),
        }
        self.rt.bump();
        self.reload(cx);
    }

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = None;
        self.focus_composer(window, cx);
        cx.notify();
    }

    /// Keeps typing out of the composer behind an open overlay. Keys still reach the workspace.
    pub fn release_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = &self.pane {
            let input = pane.input.clone();
            input.update(cx, |state, cx| state.focus(window, cx));
        } else if self.selected.is_some() {
            let draft = self.current_draft(window, cx);
            draft.update(cx, |state, cx| state.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
    }

    /// Esc closes what floats on top, then the creation form. Returns whether it closed anything.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.overlay.is_some() {
            self.close_overlay(window, cx);
            return true;
        }
        // The creation form has nothing to go back to without a channel.
        if self.create.is_some()
            && let Some(first) = self.channels.first().map(|r| r.channel.channel_id.clone())
        {
            self.select(first, window, cx);
            return true;
        }
        false
    }

    /// Copies `text` and shows a check on the button named `key` for a moment.
    pub fn copy(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.copied = Some(key);
        self.copy_generation += 1;
        let generation = self.copy_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.copy_generation == generation {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn copy_icon(&self, key: &str, color: gpui_kit::Rgba) -> impl IntoElement + use<> {
        let copied = self.copied.as_deref() == Some(key);
        ui::icon(if copied { "check" } else { "copy" }, 14., color)
    }

    /// Highlights a message for a moment, after search.
    pub fn flash(&mut self, message_id: String, cx: &mut Context<Self>) {
        self.highlight = Some(message_id.clone());
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this.highlight.as_deref() == Some(message_id.as_str()) {
                    this.highlight = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    // Native notifications (`docs/product-spec.md` 4.8)

    fn displayed(&self, scope: &OwnedScope) -> Option<&Timeline> {
        [
            self.timeline.as_ref(),
            self.pane.as_ref().and_then(|p| p.timeline.as_ref()),
        ]
        .into_iter()
        .flatten()
        .find(|timeline| &timeline.scope == scope)
    }

    fn view_of(&self, scope: &OwnedScope, window: &Window) -> View {
        let timeline = self.displayed(scope);
        View {
            window_focused: window.is_window_active(),
            timeline_displayed: timeline.is_some(),
            at_bottom: timeline.is_some_and(Timeline::is_at_bottom),
        }
    }

    /// Announces new agent messages. Runs before timelines reload, so "at the bottom" means the
    /// user was following along when the message arrived.
    fn announce(&mut self, window: &Window, cx: &mut Context<Self>) {
        let store = self.rt.store();
        let Ok(messages) = store.agent_messages_after(self.announced_through.as_deref()) else {
            return;
        };
        drop(store);
        for message in messages {
            self.announced_through = Some(message.message_id.clone());
            let scope = OwnedScope::of(&message);
            if !notify::should_notify(self.view_of(&scope, window)) {
                continue;
            }
            let Ok(channel) = self.rt.store().channel(&message.channel_id) else {
                continue;
            };
            let tag = notify::tag(scope.as_scope());
            cx.show_system_notification(SystemNotification {
                tag: tag.clone().into(),
                title: channel.name.into(),
                body: head(&message.text, NOTIFICATION_CHARS).into(),
                actions: Vec::new(),
            });
            self.notified.insert(tag);
        }
    }

    fn dismiss_opened(&mut self, scope: &OwnedScope, window: &Window, cx: &mut Context<Self>) {
        if notify::should_dismiss(self.view_of(scope, window), true) {
            let tag = notify::tag(scope.as_scope());
            cx.dismiss_system_notification(&tag);
            self.notified.remove(&tag);
        }
    }

    fn dismiss_reached(&mut self, window: &Window, cx: &mut Context<Self>) {
        let scopes: Vec<OwnedScope> = [
            self.timeline.as_ref(),
            self.pane.as_ref().and_then(|p| p.timeline.as_ref()),
        ]
        .into_iter()
        .flatten()
        .map(|timeline| timeline.scope.clone())
        .collect();
        for scope in scopes {
            let tag = notify::tag(scope.as_scope());
            if self.notified.contains(&tag)
                && notify::should_dismiss(self.view_of(&scope, window), false)
            {
                cx.dismiss_system_notification(&tag);
                self.notified.remove(&tag);
            }
        }
    }

    /// Opens the timeline a notification points at.
    pub fn open_tag(&mut self, tag: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(channel_id) = tag.strip_prefix("main:") {
            self.select(channel_id.to_owned(), window, cx);
        } else if let Some(thread_id) = tag.strip_prefix("thread:") {
            let store = self.rt.store();
            let thread = store
                .channel_of_thread(thread_id)
                .ok()
                .flatten()
                .and_then(|channel_id| store.thread(&channel_id, thread_id).ok());
            drop(store);
            if let Some(thread) = thread {
                if self.selected.as_ref() != Some(&thread.channel_id) {
                    self.select(thread.channel_id.clone(), window, cx);
                }
                self.open_thread(thread.parent_message, window, cx);
            }
        }
        // The window may not be active yet, but the user just asked for this timeline.
        cx.dismiss_system_notification(tag);
        self.notified.remove(tag);
    }

    // Rendering

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut rows = div().flex().flex_col();
        for row in &self.channels {
            let id = row.channel.channel_id.clone();
            let selected = self.selected.as_deref() == Some(id.as_str());
            let glyph = div().size(px(16.)).flex().items_center().justify_center();
            let glyph = if row.stopped || row.blocked {
                glyph.child(div().size(px(7.)).rounded_full().bg(theme::warn()))
            } else if row.unread {
                glyph.child(div().size(px(7.)).rounded_full().bg(theme::unread()))
            } else {
                glyph.child(div().size(px(5.)).rounded_full().bg(theme::ink4()))
            };
            let count = (row.unresolved > 0).then(|| {
                div()
                    .text_size(px(12.))
                    .text_color(theme::ink3())
                    .child(row.unresolved.to_string())
            });
            let mut line = div()
                .id(SharedString::from(id.clone()))
                .h(px(30.))
                .px(px(8.))
                .gap(px(8.))
                .flex()
                .items_center()
                .rounded(px(7.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select(id.clone(), window, cx);
                }))
                .child(glyph)
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_size(px(13.))
                        .text_color(theme::ink())
                        .child(row.channel.name.clone()),
                )
                .children(count);
            if selected {
                line = line.bg(theme::selected());
            }
            rows = rows.child(line);
        }
        div()
            .w(px(self.shown.sidebar))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .px(px(10.))
            .child(
                div()
                    .h(px(60.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down)
                    .child(self.sidebar_toggle(cx)),
            )
            .child(
                div()
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(theme::ink3())
                    .child("チャンネル")
                    .child(
                        div()
                            .id("new-channel")
                            .cursor_pointer()
                            .child(ui::icon("plus", 16., theme::ink2()))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_create(window, cx)),
                            ),
                    ),
            )
            .child(rows)
    }

    fn sidebar_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        ui::icon_button("sidebar-toggle", "panel-left", theme::ink2())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                this.sidebar_open = !this.sidebar_open;
                cx.notify();
            }))
    }

    fn message_row(
        &self,
        row: &Row,
        ix: usize,
        is_main: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match row {
            Row::Parent(parent) => div()
                .pt(px(1.))
                .child(self.message(parent, None, usize::MAX / 2, false, cx))
                .into_any_element(),
            // More below than above: the footer before it already leaves room, and the label's
            // line box and the next line's leading are smaller than that.
            Row::Unread => div()
                .px(px(MESSAGE_INSET_X))
                .pt(px(MESSAGE_INSET_Y))
                .pb(px(9.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().h(px(1.)).bg(theme::unread_line()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::unread_text())
                        .child("ここから未読"),
                )
                .child(div().flex_1().h(px(1.)).bg(theme::unread_line()))
                .into_any_element(),
            Row::Event(event) => {
                let who = match event.actor {
                    Author::User => "あなた",
                    Author::Agent => "エージェント",
                };
                let (mark, what) = match event.action {
                    StateAction::Resolved => ("check", "が解決"),
                    StateAction::Reopened => ("rotate-ccw", "が再開"),
                };
                div()
                    .px(px(MESSAGE_INSET_X))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(theme::ink2())
                    .child(ui::icon(mark, 14., theme::ink2()))
                    .child(format!("{who}{what}"))
                    .child(
                        div()
                            .text_color(theme::ink3())
                            .child(ui::when(event.created_at)),
                    )
                    .into_any_element()
            }
            Row::Message(message, link) => self
                .message(message, link.as_deref(), ix, is_main, cx)
                .into_any_element(),
        }
    }

    fn message(
        &self,
        message: &Message,
        link: Option<&ThreadLink>,
        ix: usize,
        is_main: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let group = SharedString::from(format!("message-{}", message.message_id));
        let markdown_id = SharedString::from(format!("md-{is_main}-{}", message.message_id));
        let body: AnyElement = match message.author {
            Author::User => div()
                .max_w(px(520.))
                .bg(theme::ink())
                .rounded(px(14.))
                .px(px(14.))
                .py(px(9.))
                .text_size(px(14.))
                .line_height(px(BODY_LINE_HEIGHT))
                .text_color(theme::white())
                .child(message.text.clone())
                .into_any_element(),
            Author::Agent => div()
                .w_full()
                .text_size(px(14.))
                .line_height(px(BODY_LINE_HEIGHT))
                .text_color(theme::ink())
                .child(TextView::markdown(markdown_id, message.text.clone()))
                .into_any_element(),
        };
        let mut footer = div().h(px(26.)).flex().items_center().gap(px(6.)).child(
            div()
                .text_size(px(12.))
                .text_color(theme::ink3())
                .child(ui::when(message.created_at)),
        );
        if is_main {
            let parent = message.clone();
            let open = cx
                .listener(move |this, _, window, cx| this.open_thread(parent.clone(), window, cx));
            footer = match link {
                Some(ThreadLink { thread, unread }) => {
                    let done = thread.resolved_at.is_some();
                    footer.child(
                        div()
                            .id(("thread-link", ix))
                            .h(px(26.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded(px(8.))
                            .cursor_pointer()
                            .bg(if done {
                                theme::tint()
                            } else {
                                theme::accent_tint()
                            })
                            .on_click(open)
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(if done { theme::ink() } else { theme::accent() })
                                    .child(format!("{}件の返信", thread.reply_count)),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(if done {
                                        theme::ink3()
                                    } else {
                                        theme::accent_soft()
                                    })
                                    .child(ui::when(thread.last_message_at)),
                            )
                            .children(
                                unread
                                    .then(|| div().size(px(7.)).rounded_full().bg(theme::unread())),
                            ),
                    )
                }
                None => footer.child(
                    div()
                        .id(("start-thread", ix))
                        .invisible()
                        .group_hover(group.clone(), |style| style.visible())
                        .h(px(26.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .rounded(px(8.))
                        .bg(theme::tint())
                        .cursor_pointer()
                        .on_click(open)
                        .text_size(px(13.))
                        .text_color(theme::ink())
                        .child(ui::icon("corner-up-left", 14., theme::ink2()))
                        .child("スレッドで返信"),
                ),
            };
        }
        let text = message.text.clone();
        let copy_key = format!("message-{is_main}-{}", message.message_id);
        let copy_icon = self.copy_icon(&copy_key, theme::ink2());
        footer = footer.child(
            div()
                .id(("copy", ix))
                .invisible()
                .group_hover(group.clone(), |style| style.visible())
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(theme::tint())
                .cursor_pointer()
                .child(copy_icon)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy(copy_key.clone(), text.clone(), cx);
                })),
        );
        let highlighted = self.highlight.as_deref() == Some(message.message_id.as_str());
        let mut row = div()
            .group(group)
            .w_full()
            .px(px(MESSAGE_INSET_X))
            .py(px(MESSAGE_INSET_Y))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(5.))
            .child(body)
            .child(footer);
        if highlighted {
            row = row.bg(theme::match_mark()).rounded(px(10.));
        }
        row
    }

    fn composer(
        &self,
        state: &Entity<TextareaState>,
        failed: bool,
        on_send: fn(&mut Self, &mut Window, &mut Context<Self>),
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let focused = state.read(cx).focus_handle(cx).contains_focused(window, cx);
        let ready = !state.read(cx).value().trim().is_empty();
        let (button, glyph, color) = if failed {
            (theme::warn(), "rotate-ccw", theme::white().into())
        } else if ready {
            (theme::ink(), "arrow-up", theme::white().into())
        } else {
            (theme::tint(), "arrow-up", theme::ink2())
        };
        div()
            .key_context("Composer")
            .on_action(cx.listener(move |this, _: &Send, window, cx| on_send(this, window, cx)))
            .w_full()
            .flex()
            .items_end()
            .gap(px(10.))
            .pl(px(10.))
            .pr(px(8.))
            .py(px(8.))
            .min_h(px(44.))
            .bg(theme::sheet())
            .rounded(px(12.))
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::field_border()
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pb(px(2.))
                    .child(ui::plain_textarea(state, 14.)),
            )
            .child(
                div()
                    .id("send")
                    .size(px(26.))
                    .flex_none()
                    .rounded_full()
                    .bg(button)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .child(ui::icon(glyph, 14., color))
                    .on_click(cx.listener(move |this, _, window, cx| on_send(this, window, cx))),
            )
    }

    fn render_header(&self, row: &ChannelRow, cx: &mut Context<Self>) -> Div {
        let stop_drag = |_: &gpui_kit::MouseDownEvent, _: &mut Window, cx: &mut gpui_kit::App| {
            cx.stop_propagation();
        };
        div()
            .h(px(44.))
            .flex_none()
            .pl(px(if self.sidebar_open { 20. } else { 72. }))
            .pr(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children((!self.sidebar_open).then(|| self.sidebar_toggle(cx)))
                    .child(
                        div()
                            .id("channel-menu")
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, stop_drag)
                            .on_click(cx.listener(|this, _, window, cx| this.open_menu(window, cx)))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::ink())
                                    .child(row.channel.name.clone()),
                            )
                            .child(ui::icon("chevron-down", 14., theme::ink3())),
                    )
                    .children((row.stopped || row.blocked).then(|| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(theme::warn())
                            .child(div().size(px(7.)).rounded_full().bg(theme::warn()))
                            .child(if row.stopped {
                                "停止中"
                            } else {
                                "確認待ち"
                            })
                    }))
                    .children(
                        (self.rt.activity(&row.channel.channel_id) == Activity::Working).then(
                            || {
                                div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(12.))
                                    .text_color(theme::ink3())
                                    .child(
                                        ui::icon("loader-circle", 12., theme::ink3())
                                            .with_animation(
                                                "running-spinner",
                                                Animation::new(Duration::from_secs(1)).repeat(),
                                                |icon, delta| {
                                                    icon.with_transformation(
                                                        Transformation::rotate(percentage(delta)),
                                                    )
                                                },
                                            ),
                                    )
                                    .child("実行中")
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .children((row.unresolved > 0).then(|| {
                        div()
                            .id("open-unresolved")
                            .h(px(28.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .rounded(px(7.))
                            .cursor_pointer()
                            .hover(|style| style.bg(theme::tint()))
                            .text_size(px(13.))
                            .text_color(theme::ink2())
                            .on_mouse_down(MouseButton::Left, stop_drag)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_unresolved(window, cx)),
                            )
                            .children(
                                row.unresolved_unread
                                    .then(|| div().size(px(7.)).rounded_full().bg(theme::unread())),
                            )
                            .child(format!("未解決 {}", row.unresolved))
                    }))
                    .child(
                        ui::icon_button("open-search", "search", theme::ink2())
                            .on_mouse_down(MouseButton::Left, stop_drag)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_search(window, cx)),
                            ),
                    ),
            )
    }

    fn render_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let sheet = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::sheet())
            .rounded(px(10.))
            .border_1()
            .border_color(theme::hairline())
            .overflow_hidden();
        if let Some(form) = &self.create {
            return sheet.child(self.render_create(form, cx));
        }
        if self.selected.is_none() {
            return sheet.on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down);
        }
        let draft = self.current_draft(window, cx);
        let (Some(timeline), Some(channel)) = (&self.timeline, self.selected_row()) else {
            return sheet;
        };
        let header = self.render_header(channel, cx);
        let banner = self.render_banner(channel, cx);
        let composer = div()
            .w_full()
            .max_w(px(720.))
            .px(px(24.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(banner)
            .child(self.composer(&draft, self.send_failed, Self::send_main, window, cx));
        if timeline.rows.is_empty() {
            // An empty channel is just its composer, centered until the first post.
            return sheet.child(header).child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(composer),
            );
        }
        let rows = timeline.rows.clone();
        let list_view = list(
            timeline.list.clone(),
            cx.processor(move |this, ix: usize, _, cx| {
                let row = rows[ix].clone();
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(720.))
                            .px(px(24. - MESSAGE_INSET_X))
                            .child(this.message_row(&row, ix, true, cx)),
                    )
                    .into_any_element()
            }),
        )
        .size_full();
        sheet
            .child(header)
            .child(
                div().flex_1().min_h_0().relative().child(list_view).child(
                    gpui_kit::base::Scrollbar::vertical(&timeline.list)
                        .mode(gpui_kit::base::ScrollbarMode::Always),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w_full()
                    .flex()
                    .justify_center()
                    .pb(px(16.))
                    .child(composer),
            )
    }

    fn render_banner(&self, row: &ChannelRow, cx: &mut Context<Self>) -> Option<AnyElement> {
        let base = |text: &'static str| {
            div()
                .min_h(px(40.))
                .pl(px(14.))
                .pr(px(8.))
                .py(px(5.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .rounded(px(10.))
                .bg(theme::warn_tint())
                .text_size(px(13.))
                .text_color(theme::ink())
                .child(text)
        };
        let action = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .h(px(30.))
                .px(px(12.))
                .flex()
                .flex_none()
                .items_center()
                .rounded(px(7.))
                .bg(theme::ink())
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(theme::white())
                .child(label)
        };
        if row.stopped {
            let button = if row.start_failed {
                let channel = row.channel.clone();
                action("retry-session", "再試行").on_click(cx.listener(move |this, _, _, cx| {
                    this.start_session(
                        channel.channel_id.clone(),
                        LaunchConfig {
                            cwd: channel.cwd.clone(),
                            command: channel.command.clone(),
                            prompt: channel.prompt.clone(),
                        },
                        cx,
                    );
                }))
            } else {
                action("new-session", "新しいセッションを開始").on_click(
                    cx.listener(|this, _, window, cx| this.open_session_dialog(window, cx)),
                )
            };
            return Some(
                base("セッションが停止しています")
                    .child(button)
                    .into_any_element(),
            );
        }
        if row.blocked {
            let attach = row.channel.session_id.as_ref().map(|session| {
                let command = format!("hc attach {session}");
                let copied = command.clone();
                let copy_icon = self.copy_icon("banner-attach", theme::ink3());
                div()
                    .id("attach-command")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .cursor_pointer()
                    .font_family(theme::MONO)
                    .text_size(px(12.))
                    .text_color(theme::ink2())
                    .child(command)
                    .child(copy_icon)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.copy("banner-attach".to_owned(), copied.clone(), cx);
                    }))
            });
            return Some(
                base("エージェントが確認を待っています")
                    .children(attach)
                    .into_any_element(),
            );
        }
        None
    }

    fn render_pane(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let pane = self.pane.as_ref()?;
        let resolved = pane
            .thread
            .as_ref()
            .is_some_and(|t| t.resolved_at.is_some());
        let header = div()
            .h(px(44.))
            .flex_none()
            .pl(px(20.))
            .pr(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down)
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::ink2())
                    .children(resolved.then_some("解決済み")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .children(pane.thread.as_ref().map(|_| {
                        div()
                            .id("resolve")
                            .h(px(28.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .rounded(px(7.))
                            .border_1()
                            .border_color(theme::field_border())
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .children((!resolved).then(|| ui::icon("check", 14., theme::ink2())))
                            .child(if resolved { "再開" } else { "解決" })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_resolved(cx)))
                    }))
                    .child(
                        ui::icon_button("close-thread", "x", theme::ink3())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_thread(window, cx)),
                            ),
                    ),
            );
        let mut replies = div().flex_1().min_h_0().relative();
        if pane.timeline.is_none() {
            replies = replies
                .child(
                    div()
                        .id("thread-draft")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&pane.draft_scroll)
                        .px(px(20. - MESSAGE_INSET_X))
                        .pt(px(1.))
                        .child(self.message(&pane.parent, None, usize::MAX / 2, false, cx)),
                )
                .child(
                    gpui_kit::base::Scrollbar::vertical(&pane.draft_scroll)
                        .mode(gpui_kit::base::ScrollbarMode::Always),
                );
        }
        if let Some(timeline) = &pane.timeline {
            let rows = timeline.rows.clone();
            replies = replies
                .child(
                    list(
                        timeline.list.clone(),
                        cx.processor(move |this, ix: usize, _, cx| {
                            let row = rows[ix].clone();
                            div()
                                .w_full()
                                .px(px(20. - MESSAGE_INSET_X))
                                .child(this.message_row(&row, ix + 1_000_000, false, cx))
                                .into_any_element()
                        }),
                    )
                    .size_full(),
                )
                .child(
                    gpui_kit::base::Scrollbar::vertical(&timeline.list)
                        .mode(gpui_kit::base::ScrollbarMode::Always),
                );
        }
        let notice = if resolved {
            Some(("rotate-ccw", "投稿すると再開します"))
        } else if pane.just_reopened {
            Some(("rotate-ccw", "エージェントへの依頼は投稿で伝わります"))
        } else {
            None
        };
        Some(
            div()
                .w(px(self.shown.thread))
                .flex_none()
                .h_full()
                .flex()
                .flex_col()
                .bg(theme::sheet())
                .rounded(px(10.))
                .border_1()
                .border_color(theme::hairline())
                .overflow_hidden()
                .child(header)
                .child(replies)
                .child(
                    div()
                        .flex_none()
                        .px(px(16.))
                        .pb(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(notice.map(|(mark, text)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(12.))
                                .text_color(theme::ink2())
                                .child(ui::icon(mark, 14., theme::ink2()))
                                .child(text)
                        }))
                        .child(self.composer(
                            &pane.input,
                            pane.send_failed,
                            Self::send_thread,
                            window,
                            cx,
                        )),
                ),
        )
    }

    fn render_create(&self, form: &CreateForm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = form.name.read(cx).value();
        let ready = validate::channel_name(&name).is_ok()
            && !form.fields.command.read(cx).value().trim().is_empty();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(480.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .pb(px(8.))
                            .text_size(px(24.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(
                                Input::new(&form.name)
                                    .appearance(false)
                                    .bordered(false)
                                    .p_0()
                                    .h(px(36.))
                                    .line_height(px(36.))
                                    .text_size(px(24.)),
                            ),
                    )
                    .child(form.fields.render(cx))
                    .children(form.error.clone().map(|error| {
                        div()
                            .text_size(px(12.))
                            .text_color(theme::warn())
                            .child(error)
                    }))
                    .child(div().pt(px(6.)).flex().justify_end().child(
                        ui::primary_button("create", "作成", ready).on_click(cx.listener(
                            move |this, _, window, cx| {
                                if ready {
                                    this.submit_create(window, cx);
                                }
                            },
                        )),
                    )),
            )
    }

    /// Development aid: writes one window state to a PNG, then quits.
    #[cfg(feature = "screenshot")]
    fn schedule_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = std::env::var_os("BRAYDR_SCREENSHOT") else {
            return;
        };
        let state = std::env::var("BRAYDR_SCREENSHOT_STATE").unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let threaded = this.timeline.as_ref().and_then(|timeline| {
                    timeline.rows.iter().find_map(|row| match row {
                        Row::Message(m, Some(_)) => Some(m.clone()),
                        _ => None,
                    })
                });
                match state.as_str() {
                    "thread" => {
                        if let Some(parent) = threaded {
                            this.open_thread(parent, window, cx);
                        }
                    }
                    "copied" => {
                        let last = this.timeline.as_ref().and_then(|timeline| {
                            timeline.rows.iter().rev().find_map(|row| match row {
                                Row::Message(m, _) => Some(m.clone()),
                                _ => None,
                            })
                        });
                        if let Some(m) = last {
                            // Without the timer, which would end before the capture.
                            this.copied = Some(format!("message-true-{}", m.message_id));
                        }
                    }
                    "draft" => {
                        let unthreaded = this.timeline.as_ref().and_then(|timeline| {
                            timeline.rows.iter().find_map(|row| match row {
                                Row::Message(m, None) => Some(m.clone()),
                                _ => None,
                            })
                        });
                        if let Some(parent) = unthreaded {
                            this.open_thread(parent, window, cx);
                        }
                    }
                    "flash" => {
                        // Without the timer, which would end before the capture.
                        this.highlight = this.timeline.as_ref().and_then(|timeline| {
                            timeline.rows.iter().find_map(|row| match row {
                                Row::Message(m, _) if m.child_thread_id.is_some() => {
                                    Some(m.message_id.clone())
                                }
                                _ => None,
                            })
                        });
                    }
                    "resize" => window.resize(gpui_kit::size(px(1100.), px(750.))),
                    "unresolved" => this.open_unresolved(window, cx),
                    "menu" => this.open_menu(window, cx),
                    "rename" => this.open_rename(window, cx),
                    "session" => this.open_session_dialog(window, cx),
                    "create" => {
                        this.start_create(window, cx);
                        if let Some(form) = &this.create {
                            form.name.update(cx, |state, cx| {
                                state.set_value("gjpqy 決済API", window, cx);
                            });
                        }
                    }
                    "collapsed" => this.sidebar_open = false,
                    "search" => {
                        this.open_search(window, cx);
                        if let Some(Overlay::Search(panel)) = &this.overlay {
                            panel.set_query("バリデーション", window, cx);
                        }
                        this.run_search(cx);
                    }
                    _ => {}
                }
            });
            // Longer when something outside the app, like `screencapture`, takes the picture.
            let seconds = std::env::var("BRAYDR_SCREENSHOT_WAIT")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(2);
            cx.background_executor()
                .timer(Duration::from_secs(seconds))
                .await;
            let _ = cx.update(|window, cx| {
                match window.render_to_image() {
                    Ok(image) => {
                        let _ = image.save(&path);
                    }
                    Err(error) => tracing::error!(%error, "cannot capture"),
                }
                cx.quit();
            });
        })
        .detach();
    }
}

impl Workspace {
    fn frame(&self, window: &Window) -> Frame {
        Frame {
            window: f32::from(window.viewport_size().width),
            sidebar_open: self.sidebar_open,
            thread_open: self.pane.is_some(),
        }
    }

    pub fn begin_resize(&mut self, edge: Edge, x: f32) {
        let start_width = match edge {
            Edge::Sidebar => self.shown.sidebar,
            Edge::Thread => self.shown.thread,
        };
        self.resize = Some(Resize {
            edge,
            start_x: x,
            start_width,
        });
    }

    pub fn drag_resize(&mut self, x: f32, window: &Window, cx: &mut Context<Self>) {
        let Some(resize) = self.resize else { return };
        let moved = x - resize.start_x;
        let width = match resize.edge {
            Edge::Sidebar => resize.start_width + moved,
            Edge::Thread => resize.start_width - moved,
        };
        self.widths = self
            .widths
            .with_dragged(resize.edge, width, self.frame(window));
        cx.notify();
    }

    pub fn end_resize(&mut self, cx: &mut Context<Self>) {
        if self.resize.take().is_some() {
            self.widths.save(&self.rt);
            cx.notify();
        }
    }

    pub fn reset_width(&mut self, edge: Edge, cx: &mut Context<Self>) {
        self.widths = self.widths.with_default(edge);
        self.widths.save(&self.rt);
        cx.notify();
    }

    /// The grip on a pane's inner edge. It has no look of its own until the pointer finds it.
    fn render_edge(&self, edge: Edge, cx: &mut Context<Self>) -> impl IntoElement {
        let grip = div()
            .absolute()
            .top_0()
            .bottom_0()
            .w(px(EDGE_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    if event.click_count == 2 {
                        this.reset_width(edge, cx);
                    } else {
                        this.begin_resize(edge, f32::from(event.position.x));
                        cx.notify();
                    }
                    cx.stop_propagation();
                }),
            );
        let grip = match edge {
            Edge::Sidebar => grip
                .id("edge-sidebar")
                .left(px(self.shown.sidebar - EDGE_WIDTH)),
            // The gap between the sheets, which sit 8 from the window's edge.
            Edge::Thread => grip.id("edge-thread").right(px(self.shown.thread + 8.)),
        };
        let dragging = self.resize.is_some_and(|resize| resize.edge == edge);
        grip.when(dragging, |grip| grip.bg(theme::hairline()))
            .hover(|grip| grip.bg(theme::hairline()))
    }

    /// Notices the release wherever it happens. A listener on the root would miss it when
    /// something occludes the root, and reading would then wait for the next click. The button
    /// can also come up outside the window, which the next move reveals.
    fn render_pointer_watch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let released = view.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = released.update(cx, |this, cx| this.release_pointer(cx));
                    }
                });
                let moved = view.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.pressed_button.is_none() {
                        let _ = moved.update(cx, |this, cx| this.release_pointer(cx));
                    }
                });
            },
        )
        .absolute()
        .size_0()
    }

    fn release_pointer(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.pointer_down) {
            cx.notify();
        }
    }

    /// Takes every pointer event during a drag, so nothing under it reacts and the cursor holds.
    fn render_resize_cover(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("resize-cover")
            .absolute()
            .inset_0()
            .occlude()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                // The button can come up outside the window, where no release reaches us.
                if event.pressed_button == Some(MouseButton::Left) {
                    this.drag_resize(f32::from(event.position.x), window, cx);
                } else {
                    this.end_resize(cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.end_resize(cx)),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.shown = self.widths.fitted(self.frame(window));
        let main = self.render_main(window, cx);
        let pane = self.render_pane(window, cx);
        let overlay = self.render_overlay(cx);
        // Read positions depend on layout, so check them after the frame.
        cx.on_next_frame(window, |this, window, cx| {
            // While dragging (the scrollbar, usually) rows fly past unread. The release counts.
            if window.is_window_active() && !this.pointer_down {
                for timeline in [
                    this.timeline.as_ref(),
                    this.pane.as_ref().and_then(|p| p.timeline.as_ref()),
                ]
                .into_iter()
                .flatten()
                {
                    timeline.track_read(&this.rt);
                }
            }
            this.dismiss_reached(window, cx);
            this.load_more_if_near_end(cx);
        });
        div()
            .key_context("Workspace")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .bg(theme::window())
            .font_family(theme::SANS)
            .capture_any_mouse_down(cx.listener(|this, _, _, _| this.pointer_down = true))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                this.dismiss(window, cx);
            }))
            .capture_action(cx.listener(|this, _: &input::Escape, window, cx| {
                if this.dismiss(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &FindInChannel, window, cx| {
                this.open_search(window, cx);
            }))
            .capture_action(cx.listener(|this, _: &input::Search, window, cx| {
                this.open_search(window, cx);
                cx.stop_propagation();
            }))
            .child(self.render_pointer_watch(cx))
            .children(self.sidebar_open.then(|| self.render_sidebar(cx)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .gap(px(8.))
                    .pt(px(8.))
                    .pr(px(8.))
                    .pb(px(8.))
                    .pl(px(if self.sidebar_open { 0. } else { 8. }))
                    .child(main)
                    .children(pane),
            )
            .children(
                self.sidebar_open
                    .then(|| self.render_edge(Edge::Sidebar, cx)),
            )
            .children(
                self.pane
                    .is_some()
                    .then(|| self.render_edge(Edge::Thread, cx)),
            )
            .children(self.resize.is_some().then(|| self.render_resize_cover(cx)))
            .children(overlay)
    }
}
