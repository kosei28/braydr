//! Remembers the main window's position, size, and display between launches.

use gpui_kit::{App, Bounds, DisplayId, Window, WindowBounds, point, px, size};

use crate::timeline::Rt;

const KEY: &str = "main_window";
const DEFAULT_SIZE: (f32, f32) = (1440., 900.);
const MIN_SIZE: (f32, f32) = (640., 480.);
/// How much of the window must be on screen for the user to get hold of it.
const MIN_VISIBLE: f32 = 100.;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Windowed,
    Maximized,
    Fullscreen,
}

/// Position relative to the display's top left.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct Saved {
    mode: Mode,
    rect: Rect,
    display: Option<String>,
}

impl Saved {
    fn encode(&self) -> String {
        let mode = match self.mode {
            Mode::Windowed => "windowed",
            Mode::Maximized => "maximized",
            Mode::Fullscreen => "fullscreen",
        };
        let Rect {
            x,
            y,
            width,
            height,
        } = self.rect;
        let mut text = format!("{mode} {x} {y} {width} {height}");
        if let Some(display) = &self.display {
            text.push(' ');
            text.push_str(display);
        }
        text
    }

    fn decode(text: &str) -> Option<Self> {
        let mut parts = text.split_whitespace();
        let mode = match parts.next()? {
            "windowed" => Mode::Windowed,
            "maximized" => Mode::Maximized,
            "fullscreen" => Mode::Fullscreen,
            _ => return None,
        };
        let mut number = || parts.next()?.parse::<f32>().ok().filter(|n| n.is_finite());
        let rect = Rect {
            x: number()?,
            y: number()?,
            width: number()?,
            height: number()?,
        };
        Some(Self {
            mode,
            rect,
            display: parts.next().map(str::to_owned),
        })
    }
}

/// Keeps the window usable on a display that may have changed since it was saved.
fn fit(rect: Rect, display: (f32, f32)) -> Rect {
    let width = rect.width.max(MIN_SIZE.0).min(display.0);
    let height = rect.height.max(MIN_SIZE.1).min(display.1);
    let reachable = rect.x + width >= MIN_VISIBLE
        && rect.x <= display.0 - MIN_VISIBLE
        && rect.y >= 0.
        && rect.y <= display.1 - MIN_VISIBLE;
    if reachable {
        Rect {
            width,
            height,
            ..rect
        }
    } else {
        Rect {
            x: (display.0 - width) / 2.,
            y: (display.1 - height) / 2.,
            width,
            height,
        }
    }
}

/// Where to open the window: the saved place if it still makes sense, otherwise centered.
pub fn restore(rt: &Rt, cx: &App) -> (WindowBounds, Option<DisplayId>) {
    let saved = rt
        .store()
        .value(KEY)
        .ok()
        .flatten()
        .and_then(|text| Saved::decode(&text));
    let Some(saved) = saved else {
        let default = size(px(DEFAULT_SIZE.0), px(DEFAULT_SIZE.1));
        return (
            WindowBounds::Windowed(Bounds::centered(None, default, cx)),
            None,
        );
    };
    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.uuid().ok().map(|uuid| uuid.to_string()) == saved.display)
        .or_else(|| cx.primary_display());
    let area = display.as_ref().map_or(DEFAULT_SIZE, |display| {
        let area = display.bounds().size;
        (f32::from(area.width), f32::from(area.height))
    });
    let rect = fit(saved.rect, area);
    let bounds = Bounds::new(
        point(px(rect.x), px(rect.y)),
        size(px(rect.width), px(rect.height)),
    );
    let bounds = match saved.mode {
        Mode::Windowed => WindowBounds::Windowed(bounds),
        Mode::Maximized => WindowBounds::Maximized(bounds),
        Mode::Fullscreen => WindowBounds::Fullscreen(bounds),
    };
    (bounds, display.map(|display| display.id()))
}

pub fn save(rt: &Rt, window: &Window, cx: &App) {
    let (mode, bounds) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (Mode::Windowed, bounds),
        WindowBounds::Maximized(bounds) => (Mode::Maximized, bounds),
        WindowBounds::Fullscreen(bounds) => (Mode::Fullscreen, bounds),
    };
    let saved = Saved {
        mode,
        rect: Rect {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        },
        display: window
            .display(cx)
            .and_then(|display| display.uuid().ok())
            .map(|uuid| uuid.to_string()),
    };
    if let Err(error) = rt.store().set_value(KEY, &saved.encode()) {
        tracing::warn!(%error, "cannot save the window position");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn round_trips() {
        let saved = Saved {
            mode: Mode::Maximized,
            rect: rect(10.5, -20., 1440., 900.),
            display: Some("ABC-123".into()),
        };
        assert_eq!(Saved::decode(&saved.encode()), Some(saved));
        let no_display = Saved {
            mode: Mode::Windowed,
            rect: rect(1., 2., 3., 4.),
            display: None,
        };
        assert_eq!(Saved::decode(&no_display.encode()), Some(no_display));
    }

    #[test]
    fn rejects_damaged_text() {
        for text in [
            "",
            "tiny 1 2 3 4",
            "windowed 1 2 3",
            "windowed 1 2 x 4",
            "windowed NaN 2 3 4",
        ] {
            assert_eq!(Saved::decode(text), None, "{text}");
        }
    }

    #[test]
    fn keeps_a_window_that_still_fits() {
        let place = rect(100., 50., 1000., 700.);
        assert_eq!(fit(place, (1920., 1080.)), place);
    }

    #[test]
    fn keeps_a_window_partly_off_the_side() {
        let place = rect(-500., 50., 1000., 700.);
        assert_eq!(fit(place, (1920., 1080.)), place);
    }

    #[test]
    fn shrinks_to_a_smaller_display() {
        let fitted = fit(rect(0., 0., 3000., 2000.), (1440., 900.));
        assert_eq!((fitted.width, fitted.height), (1440., 900.));
    }

    #[test]
    fn enforces_a_usable_size() {
        let fitted = fit(rect(10., 10., 10., 10.), (1920., 1080.));
        assert_eq!((fitted.width, fitted.height), MIN_SIZE);
    }

    #[test]
    fn centers_a_window_left_on_a_removed_display() {
        assert_eq!(
            fit(rect(3000., 200., 1000., 700.), (1920., 1080.)),
            rect(460., 190., 1000., 700.)
        );
    }

    #[test]
    fn centers_a_window_whose_title_area_is_out_of_reach() {
        assert_eq!(
            fit(rect(100., -50., 1000., 700.), (1920., 1080.)),
            rect(460., 190., 1000., 700.)
        );
    }
}
