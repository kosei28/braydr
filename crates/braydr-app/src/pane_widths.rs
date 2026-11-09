//! The widths of the sidebar and the thread sheet, which the user can drag.

use crate::timeline::Rt;

const KEY: &str = "pane_widths";
const SIDEBAR: Range = Range {
    default: 240.,
    min: 180.,
    max: 400.,
};
/// The main sheet keeps at least this much when a pane grows or the window shrinks. The thread
/// sheet has the same floor and no ceiling: its content column stops growing by itself.
const MAIN_MIN: f32 = 360.;
const THREAD: Range = Range {
    default: 416.,
    min: MAIN_MIN,
    max: f32::INFINITY,
};
/// The sheets' margin to the window's edge, and the gap between them.
const MARGIN: f32 = 8.;
/// Narrower than this, a sidebar beside the sheets would crowd the main sheet, so it slides
/// over them as a drawer instead.
pub const DRAWER_BELOW: f32 = 720.;

struct Range {
    default: f32,
    min: f32,
    max: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Sidebar,
    Thread,
}

/// What is open, which with the window's width decides how much room the panes can have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub window: f32,
    pub sidebar_open: bool,
    pub thread_open: bool,
}

/// How the panes are drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub window: f32,
    pub sidebar: f32,
    pub thread: f32,
    /// The sidebar slides over the sheets instead of sitting beside them.
    pub drawer: bool,
    /// The thread sheet covers the main sheet instead of sitting beside it.
    pub thread_over: bool,
}

impl Frame {
    fn drawer(self) -> bool {
        self.window < DRAWER_BELOW
    }

    /// What sits beside the main sheet. The thread sheet does while both sheets fit at their
    /// minimums, and covers the main sheet once they do not.
    fn docked(self) -> Self {
        let sidebar_open = self.sidebar_open && !self.drawer();
        let beside = Self {
            sidebar_open,
            thread_open: true,
            ..self
        };
        let sidebar_min = if sidebar_open { SIDEBAR.min } else { 0. };
        Self {
            sidebar_open,
            thread_open: self.thread_open && beside.room() >= sidebar_min + THREAD.min,
            ..self
        }
    }

    /// The main sheet's width with no thread sheet beside it, given the sidebar beside it.
    fn main_alone(self, sidebar: f32) -> f32 {
        let left = if self.sidebar_open { sidebar } else { MARGIN };
        self.window - left - MARGIN
    }

    /// The room for the sidebar and the thread sheet together.
    fn room(self) -> f32 {
        let chrome = MARGIN
            + if self.sidebar_open { 0. } else { MARGIN }
            + if self.thread_open { MARGIN } else { 0. };
        self.window - chrome - MAIN_MIN
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneWidths {
    pub sidebar: f32,
    pub thread: f32,
}

impl Default for PaneWidths {
    fn default() -> Self {
        Self {
            sidebar: SIDEBAR.default,
            thread: THREAD.default,
        }
    }
}

impl PaneWidths {
    pub fn load(rt: &Rt) -> Self {
        rt.store()
            .value(KEY)
            .ok()
            .flatten()
            .and_then(|text| Self::decode(&text))
            .unwrap_or_default()
    }

    pub fn save(self, rt: &Rt) {
        if let Err(error) = rt.store().set_value(KEY, &self.encode()) {
            tracing::warn!(%error, "cannot save the pane widths");
        }
    }

    fn encode(self) -> String {
        format!("{} {}", self.sidebar, self.thread)
    }

    fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let mut number = || parts.next()?.parse::<f32>().ok().filter(|n| n.is_finite());
        Some(Self {
            sidebar: number()?.clamp(SIDEBAR.min, SIDEBAR.max),
            thread: number()?.clamp(THREAD.min, THREAD.max),
        })
    }

    /// What to draw. A narrow window takes from the thread sheet first, then from the sidebar,
    /// then lays the thread sheet over the main sheet and turns the sidebar into a drawer,
    /// without forgetting the chosen widths: they come back when the window grows.
    pub fn layout(self, frame: Frame) -> Layout {
        let docked = frame.docked();
        let fitted = self.fitted(docked);
        let thread_over = frame.thread_open && !docked.thread_open;
        Layout {
            window: frame.window,
            sidebar: fitted.sidebar,
            thread: if thread_over {
                docked.main_alone(fitted.sidebar)
            } else {
                fitted.thread
            },
            drawer: frame.drawer(),
            thread_over,
        }
    }

    /// Fits the panes beside the main sheet.
    fn fitted(self, frame: Frame) -> Self {
        let sidebar = if frame.sidebar_open { self.sidebar } else { 0. };
        let thread = if frame.thread_open { self.thread } else { 0. };
        let over = (sidebar + thread - frame.room()).max(0.);
        let from_thread = if frame.thread_open {
            over.min(thread - THREAD.min)
        } else {
            0.
        };
        let from_sidebar = if frame.sidebar_open {
            (over - from_thread).min(sidebar - SIDEBAR.min)
        } else {
            0.
        };
        Self {
            sidebar: self.sidebar - from_sidebar,
            thread: self.thread - from_thread,
        }
    }

    /// Sets one pane from a drag, leaving the main sheet its share of the window. The drawer
    /// has the window instead.
    pub fn with_dragged(self, edge: Edge, width: f32, frame: Frame) -> Self {
        let drawer = frame.drawer();
        let frame = frame.docked();
        let shown = self.fitted(frame);
        match edge {
            Edge::Sidebar if drawer => Self {
                sidebar: limit(width, &SIDEBAR, frame.window - MARGIN),
                ..self
            },
            Edge::Sidebar => {
                let other = if frame.thread_open { shown.thread } else { 0. };
                Self {
                    sidebar: limit(width, &SIDEBAR, frame.room() - other),
                    ..self
                }
            }
            Edge::Thread => {
                let other = if frame.sidebar_open {
                    shown.sidebar
                } else {
                    0.
                };
                Self {
                    thread: limit(width, &THREAD, frame.room() - other),
                    ..self
                }
            }
        }
    }

    pub fn with_default(self, edge: Edge) -> Self {
        match edge {
            Edge::Sidebar => Self {
                sidebar: SIDEBAR.default,
                ..self
            },
            Edge::Thread => Self {
                thread: THREAD.default,
                ..self
            },
        }
    }
}

fn limit(width: f32, range: &Range, room: f32) -> f32 {
    width.min(room).min(range.max).max(range.min)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Frame = Frame {
        window: 1440.,
        sidebar_open: true,
        thread_open: true,
    };

    fn widths(sidebar: f32, thread: f32) -> PaneWidths {
        PaneWidths { sidebar, thread }
    }

    #[test]
    fn round_trips() {
        let saved = widths(300.5, 500.);
        assert_eq!(PaneWidths::decode(&saved.encode()), Some(saved));
    }

    #[test]
    fn rejects_damaged_text() {
        for text in ["", "300", "300 x", "NaN 400"] {
            assert_eq!(PaneWidths::decode(text), None, "{text}");
        }
    }

    #[test]
    fn brings_saved_widths_back_into_range() {
        assert_eq!(PaneWidths::decode("10 9999"), Some(widths(180., 9999.)));
    }

    #[test]
    fn keeps_widths_that_fit() {
        let chosen = widths(300., 500.);
        assert_eq!(chosen.fitted(WIDE), chosen);
    }

    #[test]
    fn a_narrow_window_takes_from_the_thread_sheet_first() {
        let frame = Frame {
            window: 1000.,
            ..WIDE
        };
        // Room is 1000 - 16 - 360 = 624.
        assert_eq!(widths(240., 416.).fitted(frame), widths(240., 384.));
    }

    #[test]
    fn then_from_the_sidebar_and_no_further_than_the_minimums() {
        let frame = Frame {
            window: 860.,
            ..WIDE
        };
        // Room is 484, with the thread sheet at its minimum of 360.
        assert_eq!(widths(240., 416.).fitted(frame), widths(180., 360.));
        let tiny = Frame {
            window: 640.,
            ..WIDE
        };
        assert_eq!(widths(240., 416.).fitted(tiny), widths(180., 360.));
    }

    #[test]
    fn a_closed_pane_takes_no_room() {
        let frame = Frame {
            window: 800.,
            sidebar_open: true,
            thread_open: false,
        };
        assert_eq!(widths(400., 720.).fitted(frame), widths(400., 720.));
    }

    #[test]
    fn a_drag_stops_at_the_range() {
        let chosen = widths(240., 416.);
        let sidebar = |width| chosen.with_dragged(Edge::Sidebar, width, WIDE).sidebar;
        assert_eq!(sidebar(50.), 180.);
        assert_eq!(sidebar(320.), 320.);
        assert_eq!(sidebar(900.), 400.);
        let thread = |width| chosen.with_dragged(Edge::Thread, width, WIDE).thread;
        assert_eq!(thread(50.), 360.);
        // No ceiling but the main sheet's share: 1440 - 16 - 360 - 240.
        assert_eq!(thread(9999.), 824.);
    }

    #[test]
    fn a_drag_leaves_the_main_sheet_its_share() {
        let frame = Frame {
            window: 1100.,
            ..WIDE
        };
        // Room is 724, and the sidebar holds 240.
        let dragged = widths(240., 416.).with_dragged(Edge::Thread, 700., frame);
        assert_eq!(dragged, widths(240., 484.));
    }

    fn layout(window: f32, sidebar_open: bool) -> Layout {
        widths(240., 416.).layout(Frame {
            window,
            sidebar_open,
            thread_open: true,
        })
    }

    #[test]
    fn the_thread_sheet_sits_beside_the_main_sheet_while_both_fit() {
        // 180 + 360 + 360 and the margins.
        let tight = layout(916., true);
        assert!(!tight.thread_over);
        assert_eq!((tight.sidebar, tight.thread), (180., 360.));
        // Without the sidebar, 8 + 360 + 8 + 360 + 8.
        assert!(!layout(744., false).thread_over);
    }

    #[test]
    fn then_covers_the_main_sheet() {
        let over = layout(915., true);
        assert!(over.thread_over && !over.drawer);
        // The main sheet's place: 915 - 240 - 8.
        assert_eq!((over.sidebar, over.thread), (240., 667.));
        assert!(layout(743., false).thread_over);
    }

    #[test]
    fn a_narrow_window_turns_the_sidebar_into_a_drawer() {
        assert!(!layout(DRAWER_BELOW, true).drawer);
        let narrow = layout(DRAWER_BELOW - 1., true);
        assert!(narrow.drawer && narrow.thread_over);
        assert_eq!(narrow.sidebar, 240.);
    }

    #[test]
    fn the_drawer_drags_within_its_range() {
        let frame = Frame {
            window: 600.,
            sidebar_open: true,
            thread_open: true,
        };
        let chosen = widths(240., 416.);
        assert_eq!(
            chosen.with_dragged(Edge::Sidebar, 9999., frame).sidebar,
            400.
        );
        assert_eq!(chosen.with_dragged(Edge::Sidebar, 50., frame).sidebar, 180.);
    }

    #[test]
    fn resets_one_edge() {
        assert_eq!(
            widths(300., 500.).with_default(Edge::Thread),
            widths(300., 416.)
        );
    }
}
