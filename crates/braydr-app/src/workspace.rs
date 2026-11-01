//! The window: sidebar, main sheet, and right sheet (`docs/design-system.md` 3 and 4).

use std::sync::Arc;
use std::time::Duration;

use braydr_core::read::{self, Laid};
use braydr_core::{Author, LaunchConfig, Message, Scope, StateAction, StateEvent, Thread, launch};
use braydr_herdr::Client;
use braydr_runtime::{Runtime, SessionState};
use chrono::{Local, TimeZone};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::text::TextView;
use gpui_kit::{
    AnyElement, AppContext as _, ClipboardItem, Context, Div, Entity, InteractiveElement as _,
    IntoElement, ListAlignment, ListOffset, ListState, MouseButton, ParentElement as _,
    PathPromptOptions, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    div, list, px,
};

use crate::theme;

gpui_kit::actions!(braydr, [Send]);

type Rt = Arc<Runtime<Client>>;

fn time(ms: i64) -> String {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or_else(String::new, |at| at.format("%H:%M").to_string())
}

fn owned(scope: Scope<'_>) -> OwnedScope {
    match scope {
        Scope::Main { channel_id } => OwnedScope::Main(channel_id.to_owned()),
        Scope::Thread { thread_id } => OwnedScope::Thread(thread_id.to_owned()),
    }
}

#[derive(Clone, PartialEq)]
enum OwnedScope {
    Main(String),
    Thread(String),
}

impl OwnedScope {
    fn as_scope(&self) -> Scope<'_> {
        match self {
            OwnedScope::Main(channel_id) => Scope::Main { channel_id },
            OwnedScope::Thread(thread_id) => Scope::Thread { thread_id },
        }
    }
}

#[derive(Clone)]
enum Row {
    Message(Message),
    Event(StateEvent),
    Unread,
}

/// One scrolling timeline and its read tracking.
struct Timeline {
    scope: OwnedScope,
    rows: Vec<Row>,
    list: ListState,
    divider_before: Option<String>,
}

impl Timeline {
    fn open(rt: &Rt, scope: OwnedScope) -> Self {
        let divider_before = rt.store().first_unread(scope.as_scope()).ok().flatten();
        // Main reads from the bottom up; a thread reads down from its parent.
        let alignment = match scope {
            OwnedScope::Main(_) => ListAlignment::Bottom,
            OwnedScope::Thread(_) => ListAlignment::Top,
        };
        let mut timeline = Self {
            scope,
            rows: Vec::new(),
            list: ListState::new(0, alignment, px(600.)),
            divider_before,
        };
        timeline.reload(rt);
        match timeline.divider_row() {
            Some(row) => timeline.list.scroll_to(ListOffset {
                item_ix: row,
                offset_in_item: px(0.),
            }),
            None => timeline.list.scroll_to_end(),
        }
        timeline
    }

    fn divider_row(&self) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, Row::Unread))
    }

    fn reload(&mut self, rt: &Rt) {
        let mut messages = Vec::new();
        let mut cursor = None;
        loop {
            let Ok(page) =
                rt.store()
                    .list_posts(self.scope.as_scope(), cursor.as_deref(), Some(100))
            else {
                break;
            };
            let more = page.next_cursor.clone();
            messages.splice(0..0, page.items);
            match more {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let events = match &self.scope {
            OwnedScope::Thread(thread_id) => rt.store().state_events(thread_id).unwrap_or_default(),
            OwnedScope::Main(_) => Vec::new(),
        };
        let mut rows = Vec::with_capacity(messages.len() + events.len() + 1);
        let mut events = events.into_iter().peekable();
        for message in messages {
            while events
                .peek()
                .is_some_and(|event| event.event_id < message.message_id)
            {
                rows.push(Row::Event(events.next().expect("peeked")));
            }
            if self.divider_before.as_deref() == Some(message.message_id.as_str()) {
                rows.push(Row::Unread);
            }
            rows.push(Row::Message(message));
        }
        rows.extend(events.map(Row::Event));
        let (old, new) = (self.rows.len(), rows.len());
        let changed = old != new;
        self.rows = rows;
        if changed {
            let at_end = self.is_at_bottom();
            self.list.splice(0..old, new);
            if at_end {
                self.list.scroll_to_end();
            }
        } else {
            self.list.remeasure();
        }
    }

    fn is_at_bottom(&self) -> bool {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return true;
        };
        let viewport = self.list.viewport_bounds();
        self.list
            .bounds_for_item(last)
            .is_some_and(|bounds| bounds.bottom() <= viewport.bottom() + px(2.))
    }

    /// Advances the read position to what the user has actually seen.
    fn track_read(&self, rt: &Rt) {
        let viewport = self.list.viewport_bounds();
        let mut laid = Vec::new();
        let mut ix = self.list.logical_scroll_top().item_ix;
        while let (Some(row), Some(bounds)) = (self.rows.get(ix), self.list.bounds_for_item(ix)) {
            if bounds.top() >= viewport.bottom() {
                break;
            }
            if let Row::Message(message) = row {
                laid.push((message.message_id.clone(), f32::from(bounds.bottom())));
            }
            ix += 1;
        }
        let refs: Vec<Laid<'_>> = laid
            .iter()
            .map(|(id, bottom)| Laid {
                message_id: id,
                bottom: *bottom,
            })
            .collect();
        if let Some(seen) = read::newest_seen(&refs, f32::from(viewport.bottom())) {
            let scope = self.scope.as_scope();
            let current = rt.store().read_position(scope).ok().flatten();
            if read::advance(current.as_deref(), Some(seen)) != current.as_deref() {
                let _ = rt.store().mark_read(scope, seen);
            }
        }
    }
}

/// The right sheet: a thread, or a draft reply before the thread exists.
struct ThreadPane {
    channel_id: String,
    parent: Message,
    thread: Option<Thread>,
    timeline: Option<Timeline>,
    input: Entity<TextareaState>,
}

struct ChannelRow {
    channel: braydr_core::Channel,
    unresolved: i64,
    main_unread: bool,
    stopped: bool,
}

struct CreateForm {
    name: Entity<InputState>,
    command: Entity<TextareaState>,
    prompt: Entity<TextareaState>,
    cwd: String,
}

pub struct Workspace {
    rt: Rt,
    revision: u64,
    channels: Vec<ChannelRow>,
    selected: Option<String>,
    timeline: Option<Timeline>,
    input: Entity<TextareaState>,
    pane: Option<ThreadPane>,
    create: Option<CreateForm>,
}

impl Workspace {
    pub fn new(rt: Rt, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder("メッセージ"));
        let mut this = Self {
            rt,
            revision: 0,
            channels: Vec::new(),
            selected: None,
            timeline: None,
            input,
            pane: None,
            create: None,
        };
        this.reload_channels();
        if let Some(first) = this
            .channels
            .first()
            .map(|row| row.channel.channel_id.clone())
        {
            this.select(first, cx);
        }
        // Development aid: writes the rendered window to a PNG, then quits.
        #[cfg(feature = "screenshot")]
        if let Some(path) = std::env::var_os("BRAYDR_SCREENSHOT") {
            cx.spawn_in(window, async move |this, cx| {
                if std::env::var_os("BRAYDR_SCREENSHOT_THREAD").is_some() {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    let _ = this.update_in(cx, |this, window, cx| {
                        let parent = this.timeline.as_ref().and_then(|timeline| {
                            timeline.rows.iter().find_map(|row| match row {
                                Row::Message(m) if m.child_thread_id.is_some() => Some(m.clone()),
                                _ => None,
                            })
                        });
                        if let Some(parent) = parent {
                            this.open_thread(parent, window, cx);
                        }
                    });
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
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
        // Agents write through the tool socket on another thread.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let alive = this.update(cx, |this, cx| {
                    let revision = this.rt.revision();
                    if revision != this.revision {
                        this.revision = revision;
                        this.reload(cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        this
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
                    .has_unread(Scope::Main { channel_id: &id })
                    .unwrap_or(false);
                drop(store);
                let stopped = !matches!(self.rt.session_state(&id), Ok(SessionState::Running));
                ChannelRow {
                    channel,
                    unresolved,
                    main_unread,
                    stopped,
                }
            })
            .collect();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.reload_channels();
        let rt = self.rt.clone();
        if let Some(timeline) = &mut self.timeline {
            timeline.reload(&rt);
        }
        if let Some(pane) = &mut self.pane {
            if pane.thread.is_none() {
                pane.thread = rt
                    .store()
                    .thread_for_parent(&pane.channel_id, &pane.parent.message_id)
                    .ok()
                    .flatten();
            } else if let Some(thread) = &pane.thread {
                pane.thread = rt.store().thread(&pane.channel_id, &thread.thread_id).ok();
            }
            if let (Some(thread), None) = (&pane.thread, &pane.timeline) {
                pane.timeline = Some(Timeline::open(
                    &rt,
                    OwnedScope::Thread(thread.thread_id.clone()),
                ));
            } else if let Some(timeline) = &mut pane.timeline {
                timeline.reload(&rt);
            }
        }
        cx.notify();
    }

    fn select(&mut self, channel_id: String, cx: &mut Context<Self>) {
        self.create = None;
        self.pane = None;
        self.timeline = Some(Timeline::open(
            &self.rt,
            owned(Scope::Main {
                channel_id: &channel_id,
            }),
        ));
        self.selected = Some(channel_id);
        cx.notify();
    }

    fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("チャンネル名"));
        let command =
            cx.new(|cx| TextareaState::new(window, cx).default_value(launch::DEFAULT_COMMAND));
        let prompt =
            cx.new(|cx| TextareaState::new(window, cx).default_value(launch::DEFAULT_PROMPT));
        let cwd = std::env::var("HOME").unwrap_or_default();
        name.update(cx, |state, cx| state.focus(window, cx));
        self.selected = None;
        self.timeline = None;
        self.pane = None;
        self.create = Some(CreateForm {
            name,
            command,
            prompt,
            cwd,
        });
        cx.notify();
    }

    fn pick_directory(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update(cx, |this, cx| {
                    if let Some(form) = &mut this.create {
                        form.cwd = path.display().to_string();
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn submit_create(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.create else { return };
        let name = form.name.read(cx).value().to_string();
        let launch = LaunchConfig {
            cwd: form.cwd.clone(),
            command: form.command.read(cx).value().trim().to_owned(),
            prompt: form.prompt.read(cx).value().trim().to_owned(),
        };
        if name.trim().is_empty() || launch.command.is_empty() {
            return;
        }
        match self.rt.create_channel(&name, &launch) {
            Ok((channel, failure)) => {
                if let Some(failure) = failure {
                    tracing::warn!(%failure, "session did not start");
                }
                self.reload_channels();
                self.select(channel.channel_id, cx);
            }
            Err(error) => tracing::warn!(%error, "cannot create the channel"),
        }
    }

    fn send_main(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected.clone() else {
            return;
        };
        let text = self.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        match self.rt.post_main(&channel_id, &text) {
            Ok(_) => {
                self.input
                    .update(cx, |state, cx| state.set_value("", window, cx));
                if let Some(timeline) = &self.timeline {
                    timeline.list.scroll_to_end();
                }
            }
            Err(error) => tracing::warn!(%error, "cannot post"),
        }
    }

    fn send_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = &self.pane else { return };
        let text = pane.input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        let input = pane.input.clone();
        match self
            .rt
            .reply(&pane.channel_id, &pane.parent.message_id, &text)
        {
            Ok(_) => input.update(cx, |state, cx| state.set_value("", window, cx)),
            Err(error) => tracing::warn!(%error, "cannot reply"),
        }
    }

    fn open_thread(&mut self, parent: Message, window: &mut Window, cx: &mut Context<Self>) {
        let channel_id = parent.channel_id.clone();
        let thread = self
            .rt
            .store()
            .thread_for_parent(&channel_id, &parent.message_id)
            .ok()
            .flatten();
        let timeline = thread
            .as_ref()
            .map(|t| Timeline::open(&self.rt, OwnedScope::Thread(t.thread_id.clone())));
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder("返信"));
        input.update(cx, |state, cx| state.focus(window, cx));
        self.pane = Some(ThreadPane {
            channel_id,
            parent,
            thread,
            timeline,
            input,
        });
        cx.notify();
    }

    fn toggle_resolved(&mut self, cx: &mut Context<Self>) {
        let Some(pane) = &self.pane else { return };
        let Some(thread) = &pane.thread else { return };
        let mut store = self.rt.store();
        let result = if thread.resolved_at.is_some() {
            store.user_reopen(&pane.channel_id, &thread.thread_id)
        } else {
            store.user_resolve(&pane.channel_id, &thread.thread_id)
        };
        drop(store);
        if let Err(error) = result {
            tracing::warn!(%error, "cannot change the thread state");
        }
        self.reload(cx);
    }

    // Rendering

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut rows = div().flex().flex_col();
        for row in &self.channels {
            let id = row.channel.channel_id.clone();
            let selected = self.selected.as_deref() == Some(id.as_str());
            let glyph = div().size(px(16.)).flex().items_center().justify_center();
            let glyph = if row.stopped {
                glyph.child(
                    div()
                        .size(px(9.))
                        .rounded_full()
                        .border_1()
                        .border_color(theme::warn()),
                )
            } else if row.main_unread && !selected {
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
            rows = rows.child(
                div()
                    .id(SharedString::from(id.clone()))
                    .h(px(30.))
                    .px(px(8.))
                    .gap(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .when_selected(selected)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx)))
                    .child(glyph)
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .child(row.channel.name.clone()),
                    )
                    .children(count),
            );
        }
        div()
            .w(px(240.))
            .h_full()
            .flex()
            .flex_col()
            .px(px(10.))
            .child(
                div()
                    .h(px(60.))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
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
                            .text_size(px(16.))
                            .child("+")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_create(window, cx)),
                            ),
                    ),
            )
            .child(rows)
    }

    fn message_row(
        &self,
        row: &Row,
        ix: usize,
        is_main: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match row {
            Row::Unread => div()
                .py(px(10.))
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
                    StateAction::Resolved => ("✓", "が解決"),
                    StateAction::Reopened => ("↺", "が再開"),
                };
                div()
                    .py(px(8.))
                    .flex()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .text_color(theme::ink2())
                    .child(mark)
                    .child(format!("{who}{what}"))
                    .child(
                        div()
                            .text_color(theme::ink3())
                            .child(time(event.created_at)),
                    )
                    .into_any_element()
            }
            Row::Message(message) => self.message(message, ix, is_main, cx).into_any_element(),
        }
    }

    fn message(
        &self,
        message: &Message,
        ix: usize,
        is_main: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let group = SharedString::from(format!("message-{}", message.message_id));
        let body: AnyElement = match message.author {
            Author::User => div()
                .max_w(px(520.))
                .bg(theme::ink())
                .rounded(px(14.))
                .px(px(14.))
                .py(px(9.))
                .text_size(px(14.))
                .text_color(theme::white())
                .child(message.text.clone())
                .into_any_element(),
            Author::Agent => div()
                .w_full()
                .text_size(px(14.))
                .text_color(theme::ink())
                .child(TextView::markdown(("markdown", ix), message.text.clone()))
                .into_any_element(),
        };
        let mut footer = div().h(px(26.)).flex().items_center().gap(px(6.)).child(
            div()
                .text_size(px(12.))
                .text_color(theme::ink3())
                .child(time(message.created_at)),
        );
        if is_main {
            let thread = message
                .child_thread_id
                .as_ref()
                .and_then(|id| self.rt.store().thread(&message.channel_id, id).ok());
            let parent = message.clone();
            let open = cx
                .listener(move |this, _, window, cx| this.open_thread(parent.clone(), window, cx));
            footer = match thread {
                Some(thread) => {
                    let done = thread.resolved_at.is_some();
                    let unread = self
                        .rt
                        .store()
                        .has_unread(Scope::Thread {
                            thread_id: &thread.thread_id,
                        })
                        .unwrap_or(false);
                    let open_now = self
                        .pane
                        .as_ref()
                        .and_then(|p| p.thread.as_ref())
                        .is_some_and(|t| t.thread_id == thread.thread_id);
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
                                    .child(time(thread.last_message_at)),
                            )
                            .children(
                                (unread && !open_now)
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
                        .child("↩ スレッドで返信"),
                ),
            };
        }
        let text = message.text.clone();
        footer = footer.child(
            div()
                .id(("copy", ix))
                .invisible()
                .group_hover(group.clone(), |style| style.visible())
                .h(px(26.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(8.))
                .bg(theme::tint())
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(theme::ink2())
                .child("コピー")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                }),
        );
        div()
            .group(group)
            .w_full()
            .pb(px(14.))
            .flex()
            .flex_col()
            .items_start()
            .gap(px(5.))
            .child(body)
            .child(footer)
    }

    fn composer(
        &self,
        state: &Entity<TextareaState>,
        on_send: fn(&mut Self, &mut Window, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        div()
            .key_context("Composer")
            .on_action(cx.listener(move |this, _: &Send, window, cx| on_send(this, window, cx)))
            .w_full()
            .flex()
            .items_end()
            .gap(px(10.))
            .pl(px(14.))
            .pr(px(10.))
            .py(px(10.))
            .bg(theme::sheet())
            .rounded(px(12.))
            .border_1()
            .border_color(theme::field_border())
            .child(
                div().flex_1().child(
                    Textarea::new(state)
                        .appearance(false)
                        .bordered(false)
                        .h(px(44.)),
                ),
            )
            .child(
                div()
                    .id("send")
                    .size(px(26.))
                    .flex_none()
                    .rounded_full()
                    .bg(theme::tint())
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(theme::ink2())
                    .child("↑")
                    .on_click(cx.listener(move |this, _, window, cx| on_send(this, window, cx))),
            )
    }

    fn render_main(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
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
        let (Some(timeline), Some(channel)) = (
            &self.timeline,
            self.channels
                .iter()
                .find(|row| Some(&row.channel.channel_id) == self.selected.as_ref()),
        ) else {
            return sheet
                .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move());
        };
        let header = div()
            .h(px(44.))
            .flex_none()
            .pl(px(20.))
            .pr(px(12.))
            .flex()
            .items_center()
            .justify_between()
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(theme::ink())
                            .child(channel.channel.name.clone()),
                    )
                    .children(channel.stopped.then(|| {
                        div()
                            .text_size(px(12.))
                            .text_color(theme::warn())
                            .child("停止中")
                    })),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::ink2())
                    .child(format!("未解決 {}", channel.unresolved)),
            );
        let rt = self.rt.clone();
        if window.is_window_active() {
            timeline.track_read(&rt);
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
                            .px(px(24.))
                            .child(this.message_row(&row, ix, true, cx)),
                    )
                    .into_any_element()
            }),
        )
        .size_full();
        let silent = self
            .selected
            .as_ref()
            .is_some_and(|id| self.rt.is_silent(id));
        let session = channel.channel.session_id.clone();
        let banner = if channel.stopped {
            Some(self.banner("セッションが停止しています", None, theme::warn_tint()))
        } else if silent {
            Some(self.banner(
                "しばらくエージェントの応答がありません",
                session.map(|s| format!("hc attach {s}")),
                theme::tint(),
            ))
        } else {
            None
        };
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
                    .child(
                        div()
                            .w_full()
                            .max_w(px(720.))
                            .px(px(24.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .children(banner)
                            .child(self.composer(&self.input, Self::send_main, cx)),
                    ),
            )
    }

    fn banner(
        &self,
        text: &'static str,
        command: Option<String>,
        background: gpui_kit::Rgba,
    ) -> impl IntoElement + use<> {
        div()
            .h(px(40.))
            .px(px(14.))
            .flex()
            .items_center()
            .justify_between()
            .rounded(px(10.))
            .bg(background)
            .text_size(px(13.))
            .text_color(theme::ink())
            .child(text)
            .children(command.map(|command| {
                let copied = command.clone();
                div()
                    .id("attach-command")
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(theme::ink2())
                    .child(command)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                    })
            }))
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
                            .rounded(px(7.))
                            .border_1()
                            .border_color(rgba_border())
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .child(if resolved { "再開" } else { "✓ 解決" })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_resolved(cx)))
                    }))
                    .child(
                        div()
                            .id("close-thread")
                            .size(px(28.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(theme::ink3())
                            .child("×")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pane = None;
                                cx.notify();
                            })),
                    ),
            );
        let parent = div()
            .flex_none()
            .px(px(20.))
            .pt(px(8.))
            .pb(px(6.))
            .child(self.message(&pane.parent, usize::MAX / 2, false, cx));
        let mut replies = div().flex_1().min_h_0().relative();
        if let Some(timeline) = &pane.timeline {
            if window.is_window_active() {
                timeline.track_read(&self.rt);
            }
            let rows = timeline.rows.clone();
            replies = replies
                .child(
                    list(
                        timeline.list.clone(),
                        cx.processor(move |this, ix: usize, _, cx| {
                            let row = rows[ix].clone();
                            div()
                                .w_full()
                                .px(px(20.))
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
        Some(
            div()
                .w(px(416.))
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
                .child(parent)
                .child(div().flex_none().h(px(1.)).bg(theme::hairline()))
                .child(replies)
                .child(
                    div()
                        .flex_none()
                        .px(px(16.))
                        .pb(px(16.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .children(resolved.then(|| {
                            div()
                                .text_size(px(12.))
                                .text_color(theme::ink2())
                                .child("↺ 投稿すると再開します")
                        }))
                        .child(self.composer(&pane.input, Self::send_thread, cx)),
                ),
        )
    }

    fn render_create(&self, form: &CreateForm, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mono =
            |state: &Entity<TextareaState>| div().w_full().child(Textarea::new(state).h(px(40.)));
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .child(
                div()
                    .w(px(480.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .pb(px(8.))
                            .child(Input::new(&form.name).appearance(false).bordered(false)),
                    )
                    .child(
                        div()
                            .id("pick-directory")
                            .h(px(36.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::field_border())
                            .cursor_pointer()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .child(tilde(&form.cwd))
                            .on_click(cx.listener(|this, _, _, cx| this.pick_directory(cx))),
                    )
                    .child(mono(&form.command))
                    .child(mono(&form.prompt))
                    .child(
                        div().pt(px(6.)).flex().justify_end().child(
                            div()
                                .id("create")
                                .h(px(32.))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .rounded(px(7.))
                                .bg(theme::accent())
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(theme::white())
                                .child("作成")
                                .on_click(cx.listener(|this, _, _, cx| this.submit_create(cx))),
                        ),
                    ),
            )
    }
}

fn rgba_border() -> gpui_kit::Rgba {
    gpui_kit::rgba(0x00000018)
}

fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_owned(),
    }
}

trait WhenSelected {
    fn when_selected(self, selected: bool) -> Self;
}

impl<E: gpui_kit::Styled> WhenSelected for E {
    fn when_selected(self, selected: bool) -> Self {
        if selected {
            self.bg(theme::selected())
        } else {
            self
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = self.render_main(window, cx);
        let pane = self.render_pane(window, cx);
        // Read positions depend on layout, so check them after the frame.
        cx.on_next_frame(window, |this, window, _| {
            if window.is_window_active()
                && let Some(timeline) = &this.timeline
            {
                timeline.track_read(&this.rt);
            }
        });
        div()
            .size_full()
            .flex()
            .bg(theme::window())
            .font_family(".SystemUIFont")
            .child(self.render_sidebar(cx))
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
                    .child(main)
                    .children(pane),
            )
    }
}
