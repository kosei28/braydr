//! The window: sidebar, main sheet, and right sheet (`docs/design-system.md` 3 and 4).

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use braydr_core::notify::{self, View};
use braydr_core::text::head;
use braydr_core::{Author, LaunchConfig, Message, StateAction, Thread, validate};
use braydr_herdr::Activity;
use braydr_runtime::SessionState;
use gpui_kit::base::text::{CodeBlock, TextView as BaseTextView, TextViewStyle};
use gpui_kit::component::input::{self, Input, InputEvent, InputState, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent, ClipboardItem,
    Context, CursorStyle, DispatchPhase, Div, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement as _, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, SystemNotification, Transformation, WeakEntity,
    Window, canvas, div, linear_color_stop, linear_gradient, list, percentage, px,
};

use crate::overlay::{LaunchFields, Overlay};
use crate::pane_widths::{Edge, Frame, Layout, PaneWidths};
use crate::timeline::{OwnedScope, Row, Rt, ThreadLink, Timeline};
use crate::{settings, theme, ui, window_state};

gpui_kit::actions!(braydr, [Send, Dismiss, OpenSettings, FindInChannel]);

const BODY_LINE_HEIGHT: f32 = 23.;
const NOTIFICATION_CHARS: usize = 120;
/// A message's room inside its hover and highlight area. Lists take it off their own padding, so
/// the text stays on the same axis as the rest of the sheet.
const MESSAGE_INSET_X: f32 = 10.;
const MESSAGE_INSET_Y: f32 = 7.;
/// How far the open thread's bracket reaches right, along its slanted ends.
const OPEN_BRACKET_WIDTH: f32 = 6.;
const COPIED_FOR: Duration = Duration::from_millis(1500);
/// Both sheets center their content in a column no wider than this.
const COLUMN_MAX: f32 = 720.;
const COMPOSER_HEIGHT: f32 = 44.;
/// `gpui-base`'s scrollbar, at its widest.
const SCROLLBAR_WIDTH: f32 = 16.;
/// Both sheets' room between their edges and the content.
const COLUMN_PAD: f32 = 24.;
/// How far a list fades out above the composer. The last row keeps this much room below it, so
/// the fade covers nothing at the end.
pub const FADE: f32 = 24.;

/// One row of a sheet's content: centered, and no wider than the column.
fn column(pad: f32) -> Div {
    div().w_full().max_w(px(COLUMN_MAX)).px(px(pad))
}

fn centered_row(content: impl IntoElement) -> Div {
    div().w_full().flex().justify_center().child(content)
}

/// The composer and what sits on it, under a list that fades out into it.
fn composer_area(content: Div) -> Div {
    div()
        .flex_none()
        .w_full()
        .relative()
        .pb(px(16.))
        .child(
            // Short of the scrollbar, which stays as it is.
            div()
                .absolute()
                .left_0()
                .right(px(SCROLLBAR_WIDTH))
                .top(px(-FADE))
                .h(px(FADE))
                .bg(linear_gradient(
                    180.,
                    linear_color_stop(theme::sheet_clear(), 0.),
                    linear_color_stop(theme::sheet(), 1.),
                )),
        )
        .child(centered_row(content))
}

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

#[cfg(test)]
impl ThreadPane {
    pub fn parent(&self) -> &Message {
        &self.parent
    }

    pub fn reply_text(&self, cx: &gpui_kit::App) -> String {
        self.input.read(cx).value().to_string()
    }
}

/// A channel row being dragged to a new place in the list.
struct DraggedChannel {
    id: String,
    index: usize,
    name: String,
}

struct DraggedChannelPreview {
    name: String,
}

impl Render for DraggedChannelPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ui::floating(7.)
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .text_size(px(13.))
            .text_color(theme::ink())
            .child(self.name.clone())
    }
}

pub struct ChannelRow {
    pub channel: braydr_core::Channel,
    pub state: SessionState,
    pub stopped: bool,
    blocked: bool,
    pub start_failed: bool,
    unresolved: i64,
    main_unread: bool,
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
    /// Unsent replies by channel and parent message, so a thread's reply survives leaving it,
    /// including one written before the thread exists.
    reply_drafts: HashMap<(String, String), Entity<TextareaState>>,
    send_failed: bool,
    pub pane: Option<ThreadPane>,
    /// What each channel's right sheet shows, or showed when the user left the channel, by its
    /// parent message. Opened again on return; kept only while the app runs.
    open_threads: HashMap<String, String>,
    pub create: Option<CreateForm>,
    pub overlay: Option<Overlay>,
    /// Whether the sidebar sits beside the sheets in a wide window.
    pub sidebar_open: bool,
    /// Whether the sidebar is out over the sheets in a narrow window.
    pub drawer_open: bool,
    pub widths: PaneWidths,
    /// `widths` fitted to the window, as last drawn.
    pub shown: Layout,
    /// The window's height, as last drawn.
    pub window_height: f32,
    resize: Option<Resize>,
    pub pointer_down: bool,
    bounds_changed: bool,
    highlight: Option<String>,
    pub copied: Option<String>,
    copy_generation: u64,
    announced_through: Option<String>,
    pub notified: HashSet<String>,
    any_unread: bool,
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
            reply_drafts: HashMap::new(),
            send_failed: false,
            pane: None,
            open_threads: HashMap::new(),
            create: None,
            overlay: None,
            sidebar_open: true,
            drawer_open: false,
            widths,
            shown: widths.layout(Frame {
                window: f32::from(window.viewport_size().width),
                sidebar_open: true,
                thread_open: false,
            }),
            window_height: f32::from(window.viewport_size().height),
            resize: None,
            pointer_down: false,
            bounds_changed: false,
            highlight: None,
            copied: None,
            copy_generation: 0,
            announced_through,
            notified: HashSet::new(),
            any_unread: false,
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
                let main_unread = store
                    .has_unread(braydr_core::Scope::Main { channel_id: &id })
                    .unwrap_or(false);
                let unresolved_unread = store.has_unread_unresolved_thread(&id).unwrap_or(false);
                drop(store);
                let state = self.rt.cached_state(&id);
                ChannelRow {
                    stopped: state != SessionState::Running,
                    blocked: self.rt.activity(&id) == Activity::Blocked,
                    start_failed: self.rt.start_failed(&id),
                    state,
                    channel,
                    unresolved,
                    main_unread,
                    unresolved_unread,
                }
            })
            .collect();
        self.any_unread = self.channels.iter().any(|row| row.main_unread)
            || self.rt.store().has_unread_thread().unwrap_or(false);
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

    /// Shows a channel with the thread it had open when the user left it.
    fn select(&mut self, channel_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.show_channel(channel_id, true, window, cx);
    }

    /// Shows a channel's main timeline, and with `restore` the thread it had open.
    fn show_channel(
        &mut self,
        channel_id: String,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Choosing the channel already shown leaves its sheet as it is.
        let kept = self
            .pane
            .take()
            .filter(|pane| pane.channel_id == channel_id);
        self.create = None;
        self.overlay = None;
        self.drawer_open = false;
        self.send_failed = false;
        self.timeline = Some(Timeline::open(
            &self.rt,
            OwnedScope::Main(channel_id.clone()),
            None,
        ));
        let opened = OwnedScope::Main(channel_id.clone());
        self.selected = Some(channel_id.clone());
        self.dismiss_opened(&opened, window, cx);
        if kept.is_some() {
            self.pane = kept;
        } else if restore {
            self.reopen_thread(&channel_id, window, cx);
        }
        // Even with the sheet open, typing goes to the main composer.
        let draft = self.current_draft(window, cx);
        draft.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn reopen_thread(&mut self, channel_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(parent_id) = self.open_threads.get(channel_id) else {
            return;
        };
        let parent = self.rt.store().message(channel_id, parent_id);
        match parent {
            Ok(parent) => self.open_thread_revealed(parent, window, cx),
            Err(error) => {
                tracing::warn!(%error, "cannot reopen the thread");
                self.open_threads.remove(channel_id);
            }
        }
    }

    fn move_channel(&mut self, channel_id: &str, index: usize, cx: &mut Context<Self>) {
        if let Err(error) = self.rt.store().move_channel(channel_id, index) {
            tracing::warn!(%error, "cannot move the channel");
        }
        self.reload_channels();
        self.rt.bump();
        cx.notify();
    }

    /// Moves on from a channel that no longer exists.
    pub fn finish_delete(
        &mut self,
        channel_id: &str,
        thread_ids: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drafts.remove(channel_id);
        self.reply_drafts
            .retain(|(channel, _), _| channel != channel_id);
        self.open_threads.remove(channel_id);
        let tags = std::iter::once(format!("main:{channel_id}"))
            .chain(thread_ids.iter().map(|id| format!("thread:{id}")));
        for tag in tags {
            cx.dismiss_system_notification(&tag);
            self.notified.remove(&tag);
        }
        self.overlay = None;
        self.pane = None;
        self.timeline = None;
        self.selected = None;
        self.reload_channels();
        match self
            .channels
            .first()
            .map(|row| row.channel.channel_id.clone())
        {
            Some(first) => self.select(first, window, cx),
            None => self.start_create(window, cx),
        }
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
        let draft = Self::new_draft("メッセージ", window, cx);
        self.drafts.insert(channel_id, draft.clone());
        draft
    }

    /// Each parent message keeps its own unsent reply, whether or not its thread exists yet.
    fn reply_draft(
        &mut self,
        parent: &Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let key = (parent.channel_id.clone(), parent.message_id.clone());
        if let Some(draft) = self.reply_drafts.get(&key) {
            return draft.clone();
        }
        let draft = Self::new_draft("返信", window, cx);
        self.reply_drafts.insert(key, draft.clone());
        draft
    }

    fn new_draft(
        placeholder: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let draft = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .placeholder(placeholder)
        });
        cx.subscribe(&draft, |this, draft, _: &InputEvent, cx| {
            this.keep_thread_end(&draft);
            cx.notify();
        })
        .detach();
        draft
    }

    /// A thread reads down from its top, so a reply growing a line would push its end out of
    /// view. Pins the end before that layout when the user was there.
    fn keep_thread_end(&self, draft: &Entity<TextareaState>) {
        let Some(pane) = self.pane.as_ref().filter(|pane| &pane.input == draft) else {
            return;
        };
        if let Some(timeline) = pane.timeline.as_ref().filter(|t| t.is_at_bottom()) {
            timeline.list.scroll_to_end();
        }
    }

    pub fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drawer_open = false;
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

    /// Opens a thread from somewhere other than its parent, which is scrolled to the top.
    pub fn open_thread_revealed(
        &mut self,
        parent: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(timeline) = &self.timeline {
            timeline.reveal(&parent.message_id);
        }
        self.open_thread(parent, window, cx);
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
        let input = self.reply_draft(&parent, window, cx);
        input.update(cx, |state, cx| state.focus(window, cx));
        let opened = thread
            .as_ref()
            .map(|t| OwnedScope::Thread(t.thread_id.clone()));
        self.open_threads
            .insert(channel_id.clone(), parent.message_id.clone());
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

    pub fn close_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.pane.take() {
            self.open_threads.remove(&pane.channel_id);
        }
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

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Esc closes what floats on top, then the drawer, then the creation form. Returns whether
    /// it closed anything.
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.overlay.is_some() {
            self.close_overlay(window, cx);
            return true;
        }
        if self.drawer_open {
            self.drawer_open = false;
            cx.notify();
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

    /// The timelines the user can see. A thread sheet laid over the main sheet hides it.
    fn on_screen(&self) -> impl Iterator<Item = &Timeline> {
        let pane = self.pane.as_ref().and_then(|p| p.timeline.as_ref());
        let main = self
            .timeline
            .as_ref()
            .filter(|_| pane.is_none() || !self.shown.thread_over);
        main.into_iter().chain(pane)
    }

    fn displayed(&self, scope: &OwnedScope) -> Option<&Timeline> {
        self.on_screen().find(|timeline| &timeline.scope == scope)
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
        let scopes: Vec<OwnedScope> = self
            .on_screen()
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
            // A notification can outlive its channel.
            if self.rt.store().channel(channel_id).is_ok() {
                self.select(channel_id.to_owned(), window, cx);
            }
        } else if let Some(thread_id) = tag.strip_prefix("thread:") {
            let store = self.rt.store();
            let thread = store
                .channel_of_thread(thread_id)
                .ok()
                .flatten()
                .and_then(|channel_id| store.thread(&channel_id, thread_id).ok());
            drop(store);
            if let Some(thread) = thread {
                // The notification's thread takes the sheet, so the channel's own is not opened.
                if self.selected.as_ref() != Some(&thread.channel_id) {
                    self.show_channel(thread.channel_id.clone(), false, window, cx);
                }
                self.open_thread_revealed(thread.parent_message, window, cx);
            }
        }
        // The window may not be active yet, but the user just asked for this timeline.
        cx.dismiss_system_notification(tag);
        self.notified.remove(tag);
    }

    // Rendering

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut rows = div().flex().flex_col();
        let dragging = cx.has_active_drag();
        for (index, row) in self.channels.iter().enumerate() {
            let id = row.channel.channel_id.clone();
            let selected = self.selected.as_deref() == Some(id.as_str());
            let glyph = div().size(px(16.)).flex().items_center().justify_center();
            let glyph = if row.stopped || row.blocked {
                glyph.child(div().size(px(7.)).rounded_full().bg(theme::warn()))
            } else if row.main_unread {
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
            let dragged = DraggedChannel {
                id: id.clone(),
                index,
                name: row.channel.name.clone(),
            };
            // Above the row while dragging, so the line can show where it lands without moving
            // anything. A row dragged downward lands below the one it is dropped on.
            let drop_target = dragging.then(|| {
                div()
                    .absolute()
                    .inset_0()
                    .drag_over::<DraggedChannel>(move |style, dragged, _, _| {
                        if dragged.index < index {
                            style.border_b_2().border_color(theme::accent())
                        } else if dragged.index > index {
                            style.border_t_2().border_color(theme::accent())
                        } else {
                            style
                        }
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedChannel, _, cx| {
                        this.move_channel(&dragged.id, index, cx);
                    }))
            });
            let mut line = div()
                .id(SharedString::from(id.clone()))
                .debug_selector(|| format!("channel-row-{index}"))
                .relative()
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
                .on_drag(dragged, |dragged, _, _, cx| {
                    cx.new(|_| DraggedChannelPreview {
                        name: dragged.name.clone(),
                    })
                })
                .children(drop_target)
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
                    .child(self.sidebar_toggle("sidebar-toggle", cx)),
            )
            .child(
                div()
                    .id("open-unread")
                    .h(px(30.))
                    .mb(px(8.))
                    .px(px(8.))
                    .gap(px(8.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::tint()))
                    .text_size(px(13.))
                    .text_color(theme::ink())
                    .on_click(cx.listener(|this, _, window, cx| this.open_unread(window, cx)))
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(ui::icon("inbox", 14., theme::ink2())),
                    )
                    .child(div().flex_1().child("未読"))
                    .children(
                        self.any_unread
                            .then(|| div().size(px(7.)).rounded_full().bg(theme::unread())),
                    ),
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

    /// Each place gets its own id: buttons that share one also share their click state, so a
    /// hidden one can swallow the click.
    fn sidebar_toggle(&self, id: &'static str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        ui::icon_button(id, "panel-left", theme::ink2())
            .debug_selector(move || id.to_owned())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, _, cx| {
                if this.shown.drawer {
                    this.drawer_open = !this.drawer_open;
                } else {
                    this.sidebar_open = !this.sidebar_open;
                }
                cx.notify();
            }))
    }

    /// Whether the sidebar sits beside the sheets, which then need no room for the toggle.
    pub fn sidebar_docked(&self) -> bool {
        self.sidebar_open && !self.shown.drawer
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
        let copy_scope = format!("{is_main}-{}", message.message_id);
        let body: AnyElement = match message.author {
            Author::User => div()
                .debug_selector(|| "user-body".into())
                .max_w(px(520.))
                .bg(theme::ink())
                .rounded(px(14.))
                .px(px(14.))
                .py(px(9.))
                .text_size(px(14.))
                .line_height(px(BODY_LINE_HEIGHT))
                .text_color(theme::white())
                .child(user_text(
                    SharedString::from(format!("user-{is_main}-{}", message.message_id)),
                    &message.text,
                    cx,
                ))
                .into_any_element(),
            Author::Agent => div()
                .debug_selector(|| "agent-body".into())
                .w_full()
                .text_size(px(14.))
                .line_height(px(BODY_LINE_HEIGHT))
                .text_color(theme::ink())
                .child(
                    TextView::markdown(markdown_id, message.text.clone())
                        .code_block_actions(code_copy(cx.weak_entity(), &copy_scope))
                        .on_link_click(link_click(cx.weak_entity())),
                )
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
        // The parent of the thread open in the right sheet, drafts included.
        let open = is_main
            && self
                .pane
                .as_ref()
                .is_some_and(|pane| pane.parent.message_id == message.message_id);
        let mut row = div()
            .group(group)
            .relative()
            .w_full()
            .px(px(MESSAGE_INSET_X))
            .py(px(MESSAGE_INSET_Y))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(5.))
            .children(open.then(|| {
                open_bracket()
                    .absolute()
                    .left_0()
                    .top(px(MESSAGE_INSET_Y))
                    .bottom(px(MESSAGE_INSET_Y))
                    .w(px(OPEN_BRACKET_WIDTH))
            }))
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
            .pl(px(16.))
            .pr(px(8.))
            .py(px(8.))
            .min_h(px(COMPOSER_HEIGHT))
            .bg(theme::sheet())
            // A half circle at each end while it is one line. The send button sits on the same
            // centre: 8 of padding and the 1 of border around 26 make 44.
            .rounded(px(COMPOSER_HEIGHT / 2.))
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
            .pl(px(if self.sidebar_docked() { 20. } else { 72. }))
            .pr(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down)
            .child(
                // The name gives way first in a narrow sheet.
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .children(
                        (!self.sidebar_docked())
                            .then(|| self.sidebar_toggle("main-sidebar-toggle", cx)),
                    )
                    .child(
                        div()
                            .id("channel-menu")
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, stop_drag)
                            .on_click(cx.listener(|this, _, window, cx| this.open_menu(window, cx)))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::ink())
                                    .child(row.channel.name.clone()),
                            )
                            .child(div().flex_none().child(ui::icon(
                                "chevron-down",
                                14.,
                                theme::ink3(),
                            ))),
                    )
                    .children((row.stopped || row.blocked).then(|| {
                        div()
                            .flex()
                            .flex_none()
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
                    .flex_none()
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
        let composer = column(COLUMN_PAD)
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
                centered_row(
                    column(COLUMN_PAD - MESSAGE_INSET_X)
                        .when(ix + 1 == rows.len(), |last| last.pb(px(FADE)))
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
            .child(composer_area(composer))
    }

    fn render_banner(&self, row: &ChannelRow, cx: &mut Context<Self>) -> Option<AnyElement> {
        // In a narrow sheet the text wraps, and the action moves under it to the right.
        let base = |text: &'static str| {
            div()
                .min_h(px(40.))
                .pl(px(14.))
                .pr(px(8.))
                .py(px(5.))
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(12.))
                .gap_y(px(4.))
                .rounded(px(10.))
                .bg(theme::warn_tint())
                .text_size(px(13.))
                .text_color(theme::ink())
                .child(div().flex_grow(1.).min_w_0().py(px(5.)).child(text))
        };
        let action = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .ml_auto()
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
                    .ml_auto()
                    .min_w_0()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(6.))
                    .cursor_pointer()
                    .font_family(theme::MONO)
                    .text_size(px(12.))
                    .text_color(theme::ink2())
                    .child(div().min_w_0().truncate().child(command))
                    .child(div().flex_none().child(copy_icon))
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
        // Laid over the main sheet, it takes over the main header's place by the traffic lights.
        let toggle = self.shown.thread_over && !self.sidebar_docked();
        let header = div()
            .h(px(44.))
            .flex_none()
            .pl(px(if toggle { 72. } else { 20. }))
            .pr(px(8.))
            .flex()
            .items_center()
            .justify_between()
            .on_mouse_down(MouseButton::Left, ui::title_bar_mouse_down)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .text_color(theme::ink2())
                    .children(toggle.then(|| self.sidebar_toggle("thread-sidebar-toggle", cx)))
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
                        .pt(px(1.))
                        .pb(px(FADE))
                        .child(centered_row(column(COLUMN_PAD - MESSAGE_INSET_X).child(
                            self.message(&pane.parent, None, usize::MAX / 2, false, cx),
                        ))),
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
                            centered_row(
                                column(COLUMN_PAD - MESSAGE_INSET_X)
                                    .when(ix + 1 == rows.len(), |last| last.pb(px(FADE)))
                                    .child(this.message_row(&row, ix + 1_000_000, false, cx)),
                            )
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
                .child(composer_area(
                    column(COLUMN_PAD)
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
                )),
        )
    }

    fn render_create(&self, form: &CreateForm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let name = form.name.read(cx).value();
        let ready = validate::channel_name(&name).is_ok()
            && !form.fields.command.read(cx).value().trim().is_empty();
        div()
            .size_full()
            .px(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(480.))
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
                // `WIDTHxHEIGHT`, which the window's minimum still bounds.
                let size = std::env::var("BRAYDR_SCREENSHOT_SIZE")
                    .ok()
                    .and_then(|value| {
                        let (width, height) = value.split_once('x')?;
                        Some(gpui_kit::size(
                            px(width.parse().ok()?),
                            px(height.parse().ok()?),
                        ))
                    });
                if let Some(size) = size {
                    window.resize(size);
                }
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
                    "unread" => this.open_unread(window, cx),
                    "menu" => this.open_menu(window, cx),
                    // Where a link would sit in the first agent message of the seed.
                    "link" => this.open_link_menu(
                        "https://ci.example/runs/1".into(),
                        Some("CI の実行ログ".into()),
                        gpui_kit::point(px(720.), px(390.)),
                        window,
                        cx,
                    ),
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
                    "drawer" => this.drawer_open = true,
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

    /// The sidebar slid out over the sheets, which a click beside it puts away.
    fn render_drawer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("drawer-backdrop")
                    .absolute()
                    .inset_0()
                    .bg(theme::dim())
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.drawer_open = false;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("drawer")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .bg(theme::window())
                    .border_r_1()
                    .border_color(theme::hairline())
                    .shadow_lg()
                    .occlude()
                    .child(self.render_sidebar(cx)),
            )
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
        self.shown = self.widths.layout(self.frame(window));
        self.window_height = f32::from(window.viewport_size().height);
        if !self.shown.drawer {
            self.drawer_open = false;
        }
        let docked = self.sidebar_docked();
        let main = self.render_main(window, cx);
        let pane = self.render_pane(window, cx);
        let (beside, over) = if self.shown.thread_over {
            (None, pane)
        } else {
            (pane, None)
        };
        let overlay = self.render_overlay(cx);
        // Read positions depend on layout, so check them after the frame.
        cx.on_next_frame(window, |this, window, cx| {
            // While dragging (the scrollbar, usually) rows fly past unread. The release counts.
            if window.is_window_active() && !this.pointer_down {
                for timeline in this.on_screen() {
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
            .children(docked.then(|| self.render_sidebar(cx)))
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
                    .pl(px(if docked { 0. } else { 8. }))
                    .child(main)
                    .children(beside),
            )
            // Over the main sheet, in its place. The main sheet stays laid out underneath, so it
            // comes back as it was.
            .children(over.map(|pane| {
                div()
                    .occlude()
                    .absolute()
                    .top(px(8.))
                    .bottom(px(8.))
                    .right(px(8.))
                    .child(pane)
            }))
            .children(
                (self.pane.is_some() && !self.shown.thread_over)
                    .then(|| self.render_edge(Edge::Thread, cx)),
            )
            .children(self.drawer_open.then(|| self.render_drawer(cx)))
            .children((docked || self.drawer_open).then(|| self.render_edge(Edge::Sidebar, cx)))
            .children(self.resize.is_some().then(|| self.render_resize_cover(cx)))
            .children(overlay)
    }
}

/// The copy button in the corner of an agent message's code blocks, shown while the pointer
/// is over the block.
fn code_copy(
    workspace: WeakEntity<Workspace>,
    scope: &str,
) -> impl Fn(&CodeBlock, &mut Window, &mut App) -> AnyElement + std::marker::Send + Sync + 'static {
    let scope = scope.to_owned();
    move |block, _, cx| {
        let code = block.code().to_string();
        // Blocks of one message are told apart by their text; identical blocks copy the same.
        let key = format!("code-{scope}-{code}");
        let icon = workspace
            .upgrade()
            .map(|workspace| workspace.read(cx).copy_icon(&key, theme::ink2()));
        let workspace = workspace.clone();
        div()
            .id("copy")
            .debug_selector(|| "code-copy".into())
            .size(px(26.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.))
            .cursor_pointer()
            .hover(|style| style.bg(theme::tint()))
            .children(icon)
            .on_click(move |_, _, cx| {
                let _ = workspace.update(cx, |this, cx| this.copy(key.clone(), code.clone(), cx));
            })
            .into_any_element()
    }
}

/// A user's message as plain text that takes part in the window's text selection.
///
/// A text view keeps selection in reading order with the agent's messages, and a fence
/// without a language keeps the text as typed: no Markdown, every space and line break.
/// The style takes away the code block's look.
fn user_text(id: SharedString, text: &str, cx: &App) -> BaseTextView {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let ticks = "`".repeat(longest.max(2) + 1);
    let block = gpui_kit::StyleRefinement::default()
        .p_0()
        .border_0()
        .font_family(theme::SANS)
        .text_size(px(14.))
        .line_height(px(BODY_LINE_HEIGHT));
    BaseTextView::markdown(id, format!("{ticks}\n{text}\n{ticks}")).style(
        TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
            .with_foreground(theme::white())
            .with_code_background(gpui_kit::transparent_black())
            .with_border(gpui_kit::transparent_black())
            .with_selection(theme::user_selection().into())
            .with_code_block(block),
    )
}

/// The bracket beside the parent of the open thread: a stroke down the left whose ends slant
/// right at 45 degrees, rounded where they bend and at their tips.
fn open_bracket() -> gpui_kit::Canvas<()> {
    canvas(
        |_, _, _| {},
        |bounds, _, window, _| {
            const STROKE: f32 = 2.;
            let half = STROKE / 2.;
            let (w, h) = (OPEN_BRACKET_WIDTH, bounds.size.height.as_f32());
            let at = |x: f32, y: f32| bounds.origin + gpui_kit::point(px(x), px(y));
            // The slant meets the stem at (half, w): the curve starts back along the slant and
            // ends down the stem by the same length, so it leaves and joins both tangentially.
            let bend = 1.5;
            let along = bend * std::f32::consts::SQRT_2;
            let mut path = gpui_kit::PathBuilder::stroke(px(STROKE));
            path.move_to(at(w, half));
            path.line_to(at(half + bend, w - bend));
            path.curve_to(at(half, w + along), at(half, w));
            path.line_to(at(half, h - w - along));
            path.curve_to(at(half + bend, h - w + bend), at(half, h - w));
            path.line_to(at(w, h - half));
            if let Ok(path) = path.build() {
                window.paint_path(path, theme::accent());
            }
            // The stroke ends square; a dot on each tip rounds it.
            for y in [half, h - half] {
                let tip = gpui_kit::Bounds::new(
                    at(w - half, y - half),
                    gpui_kit::size(px(STROKE), px(STROKE)),
                );
                window.paint_quad(gpui_kit::fill(tip, theme::accent()).corner_radii(px(half)));
            }
        },
    )
}

/// Opens a link as the text view would, and offers to copy it on a right click.
fn link_click(
    workspace: WeakEntity<Workspace>,
) -> impl Fn(&SharedString, &ClickEvent, &mut Window, &mut App) + std::marker::Send + Sync + 'static
{
    move |url, event, window, cx| {
        if event.is_right_click() {
            let text = gpui_kit::base::GlobalState::clicked_link_text(cx);
            let at = event.position();
            let _ = workspace.update(cx, |this, cx| {
                this.open_link_menu(url.to_string(), text.map(|t| t.to_string()), at, window, cx);
            });
            return;
        }
        // What the text view does without a handler.
        let opens = match event {
            ClickEvent::Mouse(click) => {
                matches!(click.up.button, MouseButton::Left | MouseButton::Middle)
            }
            ClickEvent::Keyboard(_) => true,
            ClickEvent::Touch(click) => !click.long_press,
        };
        if opens {
            cx.open_url(url);
        }
    }
}
