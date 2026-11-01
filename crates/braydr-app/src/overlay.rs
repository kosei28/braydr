//! Floating panels, the channel menu, and dialogs (`docs/design-system.md` 4).

use braydr_core::text::excerpt;
use braydr_core::{LaunchConfig, Message, Scope, Thread, ThreadFilter, validate};
use braydr_runtime::SessionState;
use gpui_kit::component::input::{Input, InputEvent, InputState, TextareaState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Div, Entity, FontWeight, HighlightStyle,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, PathPromptOptions,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, StyledText, Subscription, Window,
    div, px,
};

use crate::timeline::OwnedScope;
use crate::workspace::Workspace;
use crate::{theme, ui};

const PAGE: u32 = 20;

pub struct UnresolvedPanel {
    threads: Vec<(Thread, bool)>,
    more: Option<String>,
    scroll: ScrollHandle,
}

pub struct SearchPanel {
    input: Entity<InputState>,
    query: String,
    results: Vec<Message>,
    count: i64,
    more: Option<String>,
    scroll: ScrollHandle,
    _changes: Subscription,
}

impl SearchPanel {
    /// Development aid for screenshots.
    #[cfg(feature = "screenshot")]
    pub fn set_query(&self, query: &str, window: &mut Window, cx: &mut Context<Workspace>) {
        self.input.update(cx, |state, cx| {
            state.set_value(query.to_owned(), window, cx)
        });
    }
}

#[cfg(test)]
impl SearchPanel {
    pub fn result_texts(&self) -> Vec<String> {
        self.results.iter().map(|m| m.text.clone()).collect()
    }
}

pub struct RenameDialog {
    channel_id: String,
    input: Entity<InputState>,
    _changes: Subscription,
}

pub struct DeleteDialog {
    channel_id: String,
    name: String,
    busy: bool,
    error: Option<String>,
}

/// The fields shared by channel creation and new sessions.
pub struct LaunchFields {
    pub cwd: String,
    pub command: Entity<TextareaState>,
    pub prompt: Entity<TextareaState>,
}

impl LaunchFields {
    pub fn new(launch: LaunchConfig, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let command = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 4)
                .default_value(launch.command.clone())
        });
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .default_value(launch.prompt.clone())
        });
        Self {
            cwd: launch.cwd,
            command,
            prompt,
        }
    }

    pub fn read(&self, cx: &Context<Workspace>) -> LaunchConfig {
        LaunchConfig {
            cwd: self.cwd.clone(),
            command: self.command.read(cx).value().trim().to_owned(),
            prompt: self.prompt.read(cx).value().trim().to_owned(),
        }
    }

    pub fn render(&self, cx: &mut Context<Workspace>) -> Div {
        let text = |state: &Entity<TextareaState>| {
            ui::field()
                .px(px(8.))
                .font_family(theme::MONO)
                .child(ui::plain_textarea(state, 12.5))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                ui::field()
                    .id("pick-directory")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .child(ui::icon("folder", 14., theme::ink3()))
                    .child(div().truncate().child(tilde(&self.cwd)))
                    .on_click(cx.listener(|this, _, _, cx| this.pick_directory(cx))),
            )
            .child(text(&self.command))
            .child(text(&self.prompt))
    }
}

pub struct SessionDialog {
    channel_id: String,
    live: bool,
    fields: LaunchFields,
    busy: bool,
    error: Option<String>,
}

pub enum Overlay {
    Menu,
    Unresolved(UnresolvedPanel),
    Search(SearchPanel),
    Rename(RenameDialog),
    Delete(DeleteDialog),
    Session(SessionDialog),
}

impl Overlay {
    pub fn is_empty_unresolved(&self) -> bool {
        matches!(self, Overlay::Unresolved(panel) if panel.threads.is_empty())
    }
}

pub fn tilde(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if path.starts_with(&home) => format!("~{}", &path[home.len()..]),
        _ => path.to_owned(),
    }
}

impl Workspace {
    pub fn pick_directory(&mut self, cx: &mut Context<Self>) {
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
                    let cwd = path.display().to_string();
                    if let Some(Overlay::Session(dialog)) = &mut this.overlay {
                        dialog.fields.cwd = cwd;
                    } else if let Some(form) = &mut this.create {
                        form.fields.cwd = cwd;
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // Opening

    pub fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release_focus(window, cx);
        self.overlay = Some(Overlay::Menu);
        cx.notify();
    }

    pub fn open_unresolved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.release_focus(window, cx);
        let mut panel = UnresolvedPanel {
            threads: Vec::new(),
            more: None,
            scroll: ScrollHandle::new(),
        };
        self.load_unresolved(&mut panel, PAGE as usize);
        self.overlay = Some(Overlay::Unresolved(panel));
        cx.notify();
    }

    /// Reads at least `count` rows from the start, so updates re-sort what is already loaded.
    fn load_unresolved(&self, panel: &mut UnresolvedPanel, count: usize) {
        let Some(channel_id) = &self.selected else {
            return;
        };
        let store = self.rt.store();
        let mut threads = Vec::new();
        let mut cursor = None;
        let mut more = None;
        while let Ok(page) = store.list_threads(
            channel_id,
            ThreadFilter::Unresolved,
            cursor.as_deref(),
            Some(PAGE),
        ) {
            threads.extend(page.items);
            more = page.next_cursor;
            if more.is_none() || threads.len() >= count {
                break;
            }
            cursor.clone_from(&more);
        }
        panel.threads = threads
            .into_iter()
            .map(|thread| {
                let unread = store
                    .has_unread(Scope::Thread {
                        thread_id: &thread.thread_id,
                    })
                    .unwrap_or(false);
                (thread, unread)
            })
            .collect();
        panel.more = more;
    }

    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx));
        input.update(cx, |state, cx| state.focus(window, cx));
        let changes = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.run_search(cx);
            }
        });
        self.overlay = Some(Overlay::Search(SearchPanel {
            input,
            query: String::new(),
            results: Vec::new(),
            count: 0,
            more: None,
            scroll: ScrollHandle::new(),
            _changes: changes,
        }));
        cx.notify();
    }

    pub fn run_search(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected.clone() else {
            return;
        };
        let Some(Overlay::Search(panel)) = &mut self.overlay else {
            return;
        };
        panel.query = panel.input.read(cx).value().trim().to_owned();
        panel.results.clear();
        panel.more = None;
        panel.count = 0;
        // A blank or overlong query simply has no results.
        let store = self.rt.store();
        if let Ok(page) = store.search(&channel_id, &panel.query, None, Some(PAGE)) {
            panel.count = store.search_count(&channel_id, &panel.query).unwrap_or(0);
            panel.results = page.items;
            panel.more = page.next_cursor;
        }
        cx.notify();
    }

    /// Loads the next page once a panel is scrolled near its end.
    pub fn load_more_if_near_end(&mut self, cx: &mut Context<Self>) {
        let Some(channel_id) = self.selected.clone() else {
            return;
        };
        let near_end =
            |scroll: &ScrollHandle| scroll.offset().y.abs() + px(120.) >= scroll.max_offset().y;
        match self.overlay.take() {
            Some(Overlay::Unresolved(mut panel)) => {
                if panel.more.is_some() && near_end(&panel.scroll) {
                    let count = panel.threads.len() + PAGE as usize;
                    self.load_unresolved(&mut panel, count);
                    cx.notify();
                }
                self.overlay = Some(Overlay::Unresolved(panel));
            }
            Some(Overlay::Search(mut panel)) => {
                if let Some(cursor) = panel.more.clone()
                    && near_end(&panel.scroll)
                    && let Ok(page) =
                        self.rt
                            .store()
                            .search(&channel_id, &panel.query, Some(&cursor), Some(PAGE))
                {
                    panel.results.extend(page.items);
                    panel.more = page.next_cursor;
                    cx.notify();
                }
                self.overlay = Some(Overlay::Search(panel));
            }
            other => self.overlay = other,
        }
    }

    /// Keeps an open panel current when the data changes.
    pub fn reload_overlay(&mut self, cx: &mut Context<Self>) {
        if let Some(Overlay::Unresolved(mut panel)) = self.overlay.take() {
            let count = panel.threads.len().max(PAGE as usize);
            self.load_unresolved(&mut panel, count);
            self.overlay = Some(Overlay::Unresolved(panel));
        } else if matches!(self.overlay, Some(Overlay::Search(_))) {
            self.run_search(cx);
        }
    }

    pub fn open_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let (channel_id, name) = (row.channel.channel_id.clone(), row.channel.name.clone());
        let input = cx.new(|cx| InputState::new(window, cx).default_value(name));
        input.update(cx, |state, cx| state.focus(window, cx));
        let changes = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
        self.overlay = Some(Overlay::Rename(RenameDialog {
            channel_id,
            input,
            _changes: changes,
        }));
        cx.notify();
    }

    fn submit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Rename(dialog)) = &self.overlay else {
            return;
        };
        let name = dialog.input.read(cx).value().to_string();
        let channel_id = dialog.channel_id.clone();
        let result = self.rt.store().rename_channel(&channel_id, &name);
        match result {
            Ok(()) => {
                self.rt.bump();
                self.close_overlay(window, cx);
            }
            Err(error) => tracing::warn!(%error, "cannot rename the channel"),
        }
    }

    fn open_delete(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.overlay = Some(Overlay::Delete(DeleteDialog {
            channel_id: row.channel.channel_id.clone(),
            name: row.channel.name.clone(),
            busy: false,
            error: None,
        }));
        cx.notify();
    }

    /// Stopping the session talks to herdr-core, so it stays off the UI thread.
    fn submit_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Delete(dialog)) = &mut self.overlay else {
            return;
        };
        if dialog.busy {
            return;
        }
        dialog.busy = true;
        dialog.error = None;
        let channel_id = dialog.channel_id.clone();
        let rt = self.rt.clone();
        let deleted = channel_id.clone();
        let task = cx.background_spawn(async move { rt.delete_channel(&deleted) });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(thread_ids) => this.finish_delete(&channel_id, &thread_ids, window, cx),
                Err(error) => {
                    tracing::warn!(?error, "cannot delete the channel");
                    if let Some(Overlay::Delete(dialog)) = &mut this.overlay {
                        dialog.busy = false;
                        dialog.error =
                            Some("セッションを停止できないため、削除できませんでした。".to_owned());
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn open_session_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let channel = row.channel.clone();
        let live = !row.stopped;
        self.release_focus(window, cx);
        let fields = LaunchFields::new(
            LaunchConfig {
                cwd: channel.cwd,
                command: channel.command,
                prompt: channel.prompt,
            },
            window,
            cx,
        );
        self.overlay = Some(Overlay::Session(SessionDialog {
            channel_id: channel.channel_id,
            live,
            fields,
            busy: false,
            error: None,
        }));
        cx.notify();
    }

    fn submit_session(&mut self, cx: &mut Context<Self>) {
        let Some(Overlay::Session(dialog)) = &mut self.overlay else {
            return;
        };
        let launch = LaunchConfig {
            cwd: dialog.fields.cwd.clone(),
            command: dialog.fields.command.read(cx).value().trim().to_owned(),
            prompt: dialog.fields.prompt.read(cx).value().trim().to_owned(),
        };
        if launch.command.is_empty() || dialog.busy {
            return;
        }
        dialog.busy = true;
        dialog.error = None;
        let channel_id = dialog.channel_id.clone();
        self.start_session(channel_id, launch, cx);
        cx.notify();
    }

    /// Stopping and starting talk to herdr-core, so they stay off the UI thread.
    pub fn start_session(
        &mut self,
        channel_id: String,
        launch: LaunchConfig,
        cx: &mut Context<Self>,
    ) {
        let rt = self.rt.clone();
        let task = cx.background_spawn(async move {
            rt.new_session(&channel_id, &launch)
                .map(|_| ())
                .map_err(|error| format!("{error:?}"))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if matches!(this.overlay, Some(Overlay::Session(_))) {
                            this.overlay = None;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "cannot start the session");
                        if let Some(Overlay::Session(dialog)) = &mut this.overlay {
                            dialog.busy = false;
                            dialog.error = Some(error);
                        }
                    }
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    // Rendering

    pub fn render_overlay(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let overlay = self.overlay.as_ref()?;
        let content = match overlay {
            Overlay::Menu => self.render_menu(cx),
            Overlay::Unresolved(panel) => self.render_unresolved(panel, cx),
            Overlay::Search(panel) => self.render_search(panel, cx),
            Overlay::Rename(dialog) => self.render_rename(dialog, cx),
            Overlay::Delete(dialog) => self.render_delete(dialog, cx),
            Overlay::Session(dialog) => self.render_session(dialog, cx),
        };
        let dimmed = matches!(
            overlay,
            Overlay::Rename(_) | Overlay::Delete(_) | Overlay::Session(_)
        );
        let mut backdrop = div().id("backdrop").absolute().inset_0().occlude();
        if dimmed {
            backdrop = backdrop.bg(theme::dim());
        } else {
            // Dialogs close only through their buttons or Esc, so typed input is not lost.
            backdrop = backdrop.on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
            );
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .child(backdrop)
                .child(content)
                .into_any_element(),
        )
    }

    fn sheet_left(&self) -> f32 {
        if self.sidebar_open {
            self.shown.sidebar
        } else {
            8.
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(row) = self.selected_row() else {
            return div().into_any_element();
        };
        let colored_item = |id: &'static str,
                            icon_name: &'static str,
                            label: &'static str,
                            text: gpui_kit::Rgba,
                            icon: gpui_kit::Rgba| {
            div()
                .id(id)
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(7.))
                .cursor_pointer()
                .hover(|style| style.bg(theme::tint()))
                .text_size(px(13.))
                .text_color(text)
                .child(ui::icon(icon_name, 14., icon))
                .child(label)
        };
        let item = |id: &'static str, icon_name: &'static str, label: &'static str| {
            colored_item(id, icon_name, label, theme::ink(), theme::ink2())
        };
        let reason = match &row.state {
            SessionState::Stopped { reason, exit_code }
                if reason.is_some() || exit_code.is_some() =>
            {
                let parts: Vec<String> = reason
                    .iter()
                    .cloned()
                    .chain(exit_code.iter().map(|code| format!("exit {code}")))
                    .collect();
                Some(
                    div()
                        .px(px(10.))
                        .py(px(6.))
                        .text_size(px(12.))
                        .text_color(theme::warn())
                        .child(parts.join(" · ")),
                )
            }
            _ => None,
        };
        let attach = row.channel.session_id.as_ref().map(|session| {
            let command = format!("hc attach {session}");
            let copied = command.clone();
            let copy_icon = self.copy_icon("menu-attach", theme::ink3());
            div()
                .id("menu-attach")
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_between()
                .rounded(px(7.))
                .cursor_pointer()
                .hover(|style| style.bg(theme::tint()))
                .font_family(theme::MONO)
                .text_size(px(12.))
                .text_color(theme::ink2())
                .child(command)
                .child(copy_icon)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.copy("menu-attach".to_owned(), copied.clone(), cx);
                }))
        });
        ui::floating(10.)
            .absolute()
            .top(px(48.))
            .left(px(self.sheet_left()
                + 12.
                + if self.sidebar_open { 0. } else { 64. }))
            .w(px(260.))
            .p(px(4.))
            .flex()
            .flex_col()
            .occlude()
            .children(reason)
            .child(
                item("menu-rename", "pencil", "名前を変更")
                    .on_click(cx.listener(|this, _, window, cx| this.open_rename(window, cx))),
            )
            .child(div().my(px(4.)).h(px(1.)).bg(theme::hairline()))
            .children(attach)
            .child(
                item("menu-session", "rotate-ccw", "新しいセッションを開始").on_click(
                    cx.listener(|this, _, window, cx| this.open_session_dialog(window, cx)),
                ),
            )
            .child(div().my(px(4.)).h(px(1.)).bg(theme::hairline()))
            .child(
                colored_item(
                    "menu-delete",
                    "trash-2",
                    "チャンネルを削除",
                    theme::warn(),
                    theme::warn(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.open_delete(cx))),
            )
            .into_any_element()
    }

    fn render_unresolved(&self, panel: &UnresolvedPanel, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().p(px(6.));
        for (ix, (thread, unread)) in panel.threads.iter().enumerate() {
            let parent = thread.parent_message.clone();
            rows = rows.child(
                div()
                    .id(("unresolved", ix))
                    .px(px(10.))
                    .py(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::tint()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_overlay(window, cx);
                        this.open_thread(parent.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .line_clamp(2)
                            .child(braydr_core::text::head(&thread.parent_message.text, 200)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(12.))
                            .text_color(theme::ink3())
                            .child(format!("{}件の返信", thread.reply_count))
                            .child(ui::when(thread.last_message_at))
                            .children(
                                unread
                                    .then(|| div().size(px(7.)).rounded_full().bg(theme::unread())),
                            ),
                    ),
            );
        }
        // Under the button, which has the search icon on its right.
        let right = 8. + if self.pane.is_some() { 424. } else { 0. } + 12. + 32.;
        ui::floating(14.)
            .absolute()
            .top(px(48.))
            .right(px(right))
            .w(px(408.))
            .min_h(px(44.))
            .max_h(px(560.))
            .overflow_hidden()
            .occlude()
            .child(
                div()
                    .relative()
                    .max_h(px(560.))
                    .child(
                        div()
                            .id("unresolved-list")
                            .max_h(px(560.))
                            .overflow_y_scroll()
                            .track_scroll(&panel.scroll)
                            .child(rows),
                    )
                    .child(
                        gpui_kit::base::Scrollbar::vertical(&panel.scroll)
                            .mode(gpui_kit::base::ScrollbarMode::Always),
                    ),
            )
            .into_any_element()
    }

    fn render_search(&self, panel: &SearchPanel, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = div().flex().flex_col().p(px(6.));
        for (ix, message) in panel.results.iter().enumerate() {
            let (line, range) = excerpt(&message.text, &panel.query, 24, 160);
            let highlights: Vec<_> = range
                .into_iter()
                .map(|range| {
                    (
                        range,
                        HighlightStyle {
                            background_color: Some(theme::match_mark().into()),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let picked = message.clone();
            rows = rows.child(
                div()
                    .id(("search-result", ix))
                    .h(px(36.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .rounded(px(8.))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme::tint()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_search_result(picked.clone(), window, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .text_color(theme::ink())
                            .child(StyledText::new(line).with_highlights(highlights)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(theme::ink3())
                            .child(ui::when(message.created_at)),
                    ),
            );
        }
        let has_query = !panel.query.is_empty();
        div()
            .absolute()
            .top(px(56.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                ui::floating(14.)
                    .w(px(600.))
                    .overflow_hidden()
                    .occlude()
                    .child(
                        div()
                            .h(px(48.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(ui::icon("search", 16., theme::ink3()))
                            .child(
                                div().flex_1().text_size(px(16.)).child(
                                    Input::new(&panel.input)
                                        .appearance(false)
                                        .bordered(false)
                                        .p_0()
                                        .line_height(px(24.))
                                        .text_size(px(16.)),
                                ),
                            )
                            .children(has_query.then(|| {
                                div()
                                    .flex_none()
                                    .text_size(px(12.))
                                    .text_color(theme::ink3())
                                    .child(format!("{}件", panel.count))
                            })),
                    )
                    .children((!panel.results.is_empty()).then(|| {
                        div()
                            .border_t_1()
                            .border_color(theme::hairline())
                            .relative()
                            .max_h(px(420.))
                            .child(
                                div()
                                    .id("search-results")
                                    .max_h(px(420.))
                                    .overflow_y_scroll()
                                    .track_scroll(&panel.scroll)
                                    .child(rows),
                            )
                            .child(
                                gpui_kit::base::Scrollbar::vertical(&panel.scroll)
                                    .mode(gpui_kit::base::ScrollbarMode::Always),
                            )
                    })),
            )
            .into_any_element()
    }

    fn dialog(&self, width: f32) -> Div {
        ui::floating(14.)
            .w(px(width))
            .p(px(20.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .occlude()
    }

    fn render_rename(&self, dialog: &RenameDialog, cx: &mut Context<Self>) -> AnyElement {
        let valid = validate::channel_name(&dialog.input.read(cx).value()).is_ok();
        centered(
            self.dialog(320.)
                .child(heading("チャンネル名の変更"))
                .child(
                    ui::field().child(
                        Input::new(&dialog.input)
                            .appearance(false)
                            .bordered(false)
                            .p_0()
                            .h_auto(),
                    ),
                )
                .child(
                    actions()
                        .child(ui::quiet_button("rename-cancel", "キャンセル").on_click(
                            cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
                        ))
                        .child(ui::primary_button("rename-save", "保存", valid).on_click(
                            cx.listener(move |this, _, window, cx| {
                                if valid {
                                    this.submit_rename(window, cx);
                                }
                            }),
                        )),
                ),
        )
    }

    fn render_delete(&self, dialog: &DeleteDialog, cx: &mut Context<Self>) -> AnyElement {
        let description = format!(
            "「{}」のメッセージとスレッドをすべて削除します。実行中のセッションも終了します。元に戻せません。",
            dialog.name
        );
        let busy = dialog.busy;
        centered(
            self.dialog(360.)
                .child(heading("チャンネルの削除"))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::ink2())
                        .child(description),
                )
                .children(dialog.error.clone().map(|error| {
                    div()
                        .text_size(px(12.))
                        .text_color(theme::warn())
                        .child(error)
                }))
                .child(
                    actions()
                        .child(ui::quiet_button("delete-cancel", "キャンセル").on_click(
                            cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
                        ))
                        .child(
                            ui::primary_button("delete-confirm", "削除", !busy)
                                .bg(theme::warn())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.submit_delete(window, cx);
                                })),
                        ),
                ),
        )
    }

    fn render_session(&self, dialog: &SessionDialog, cx: &mut Context<Self>) -> AnyElement {
        let description = if dialog.live {
            "進行中の作業は中断され、エージェントの作業コンテキストは引き継がれません。"
        } else {
            "エージェントの作業コンテキストは引き継がれません。"
        };
        let ready = !dialog.busy && !dialog.fields.command.read(cx).value().trim().is_empty();
        centered(
            self.dialog(440.)
                .child(heading("新しいセッションの開始"))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme::ink2())
                        .child(description),
                )
                .child(dialog.fields.render(cx))
                .children(dialog.error.clone().map(|error| {
                    div()
                        .text_size(px(12.))
                        .text_color(theme::warn())
                        .child(error)
                }))
                .child(
                    actions()
                        .child(ui::quiet_button("session-cancel", "キャンセル").on_click(
                            cx.listener(|this, _, window, cx| this.close_overlay(window, cx)),
                        ))
                        .child(ui::primary_button("session-start", "開始", ready).on_click(
                            cx.listener(move |this, _, _, cx| {
                                if ready {
                                    this.submit_session(cx);
                                }
                            }),
                        )),
                ),
        )
    }

    pub fn open_search_result(
        &mut self,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlay(window, cx);
        match OwnedScope::of(&message) {
            OwnedScope::Thread(thread_id) => {
                let thread = self.rt.store().thread(&message.channel_id, &thread_id);
                if let Ok(thread) = thread {
                    self.open_thread(thread.parent_message, window, cx);
                    if let Some(timeline) = self.pane.as_ref().and_then(|p| p.timeline.as_ref()) {
                        timeline.reveal(&message.message_id);
                    }
                }
            }
            OwnedScope::Main(_) => {
                if let Some(timeline) = &self.timeline {
                    timeline.reveal(&message.message_id);
                }
            }
        }
        self.flash(message.message_id, cx);
    }
}

fn heading(text: &'static str) -> impl IntoElement {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::ink())
        .child(text)
}

fn actions() -> Div {
    div().pt(px(4.)).flex().justify_end().gap(px(6.))
}

fn centered(dialog: Div) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(dialog)
        .into_any_element()
}
