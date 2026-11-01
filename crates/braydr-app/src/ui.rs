//! Small pieces shared by the window, the overlays, and settings.

use chrono::{Datelike, Local, TimeZone};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::{
    App, Div, ElementId, Entity, FontWeight, InteractiveElement as _, MouseDownEvent,
    ParentElement as _, Rgba, SharedString, Stateful, Styled as _, Svg, Window, div, px, svg,
};

use crate::theme;

/// For the areas that stand in for a title bar: drag moves the window, and a double click does
/// what the system setting says, which is zoom unless the user changed it.
pub fn title_bar_mouse_down(event: &MouseDownEvent, window: &mut Window, _: &mut App) {
    if event.click_count == 2 {
        window.titlebar_double_click();
    } else {
        window.start_window_move();
    }
}

pub fn icon(name: &'static str, size: f32, color: Rgba) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// `14:20` today, `10/31 14:20` this year, otherwise the full date.
pub fn when(ms: i64) -> String {
    let Some(at) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let now = Local::now();
    if at.date_naive() == now.date_naive() {
        at.format("%H:%M").to_string()
    } else if at.year() == now.year() {
        at.format("%-m/%-d %H:%M").to_string()
    } else {
        at.format("%Y/%-m/%-d %H:%M").to_string()
    }
}

/// `accent` confirm button. Looks and acts inert while `enabled` is false.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    let button = div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .bg(theme::accent())
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::white())
        .child(label.into());
    if enabled {
        button.cursor_pointer()
    } else {
        button.opacity(0.4)
    }
}

pub fn quiet_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(30.))
        .px(px(12.))
        .flex()
        .items_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(|style| style.bg(theme::tint()))
        .text_size(px(13.))
        .text_color(theme::ink2())
        .child(label.into())
}

pub fn icon_button(id: impl Into<ElementId>, name: &'static str, color: Rgba) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(|style| style.bg(theme::tint()))
        .child(icon(name, 16., color))
}

/// A multi-line input without its own frame. Its 4px inner padding counts toward the caller's.
pub fn plain_textarea(state: &Entity<TextareaState>, text_size: f32) -> Textarea {
    Textarea::new(state)
        .appearance(false)
        .bordered(false)
        .xsmall()
        .text_size(px(text_size))
}

/// A bordered input frame.
pub fn field() -> Div {
    div()
        .w_full()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::field_border())
        .bg(theme::sheet())
        .text_size(px(13.))
        .text_color(theme::ink())
}

/// The face of a floating panel, menu, or dialog.
pub fn floating(radius: f32) -> Div {
    div()
        .bg(theme::sheet())
        .rounded(px(radius))
        .border_1()
        .border_color(theme::hairline())
        .shadow_lg()
}
