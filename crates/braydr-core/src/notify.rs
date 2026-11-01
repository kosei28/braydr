//! Native notification rules (`docs/product-spec.md` 4.8).

use crate::Scope;

/// What the window is showing for a timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct View {
    pub window_focused: bool,
    pub timeline_displayed: bool,
    pub at_bottom: bool,
}

pub fn should_notify(view: View) -> bool {
    !(view.window_focused && view.timeline_displayed && view.at_bottom)
}

/// Dismiss when the user opens the timeline or reaches its bottom with the window focused.
pub fn should_dismiss(view: View, just_opened: bool) -> bool {
    view.window_focused && view.timeline_displayed && (just_opened || view.at_bottom)
}

/// One notification per timeline: the same tag replaces the older one.
pub fn tag(scope: Scope<'_>) -> String {
    match scope {
        Scope::Main { channel_id } => format!("main:{channel_id}"),
        Scope::Thread { thread_id } => format!("thread:{thread_id}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn view(window_focused: bool, timeline_displayed: bool, at_bottom: bool) -> View {
        View {
            window_focused,
            timeline_displayed,
            at_bottom,
        }
    }

    #[test]
    fn notifies_unless_the_message_is_in_view() {
        assert!(!should_notify(view(true, true, true)));
        assert!(should_notify(view(false, true, true)));
        assert!(should_notify(view(true, false, false)));
        assert!(should_notify(view(true, true, false)));
    }

    #[test]
    fn opening_dismisses_even_above_the_bottom() {
        assert!(should_dismiss(view(true, true, false), true));
        assert!(!should_dismiss(view(true, true, false), false));
        assert!(should_dismiss(view(true, true, true), false));
        assert!(!should_dismiss(view(false, true, true), true));
    }

    #[test]
    fn tags_are_per_timeline() {
        assert_eq!(tag(Scope::Main { channel_id: "c" }), "main:c");
        assert_eq!(tag(Scope::Thread { thread_id: "t" }), "thread:t");
    }
}
