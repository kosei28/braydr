//! Tokens from `docs/design-system.md`.

use gpui_kit::{Hsla, Rgba, rgb, rgba};

pub fn c(hex: u32) -> Rgba {
    rgb(hex)
}

pub fn window() -> Rgba {
    c(0xEFEFEC)
}
pub fn sheet() -> Rgba {
    c(0xFFFFFF)
}
/// `sheet` with no opacity, where a fade into it starts.
pub fn sheet_clear() -> Rgba {
    rgba(0xFFFFFF00)
}
pub fn tint() -> Rgba {
    c(0xF4F4F1)
}
pub fn selected() -> Rgba {
    c(0xE2E2DE)
}
pub fn hairline() -> Rgba {
    rgba(0x0000000F)
}
pub fn field_border() -> Rgba {
    rgba(0x00000022)
}
pub fn ink() -> Rgba {
    c(0x18181A)
}
pub fn ink2() -> Rgba {
    c(0x5C5C60)
}
pub fn ink3() -> Rgba {
    c(0x8E8E92)
}
pub fn ink4() -> Rgba {
    c(0xBDBDBF)
}
pub fn accent() -> Rgba {
    c(0x2F4BFF)
}
pub fn accent_soft() -> Rgba {
    rgba(0x2F4BFFB3)
}
pub fn accent_tint() -> Rgba {
    rgba(0x2F4BFF12)
}
pub fn unread() -> Rgba {
    c(0xF08C00)
}
pub fn unread_line() -> Rgba {
    rgba(0xF08C0050)
}
pub fn unread_text() -> Rgba {
    c(0xB86200)
}
pub fn warn() -> Rgba {
    c(0xD6295E)
}
pub fn warn_tint() -> Rgba {
    rgba(0xD6295E12)
}
pub fn white() -> Hsla {
    gpui_kit::white()
}
pub fn match_mark() -> Rgba {
    c(0xFFEDB3)
}
pub fn dim() -> Rgba {
    rgba(0x00000033)
}

pub const SANS: &str = "IBM Plex Sans JP";
pub const MONO: &str = "IBM Plex Mono";

/// Code colors, from the design system's palette: quiet comments, keywords in `accent`.
pub const SYNTAX: &str = r##"{
  "name": "Braydr",
  "appearance": "light",
  "style": {
    "syntax": {
      "comment": { "color": "#59636E" },
      "comment.doc": { "color": "#59636E" },
      "keyword": { "color": "#CF222E" },
      "preproc": { "color": "#CF222E" },
      "tag": { "color": "#116329" },
      "string": { "color": "#0A3069" },
      "string.escape": { "color": "#0A3069" },
      "string.special": { "color": "#0A3069" },
      "string.regex": { "color": "#116329" },
      "string.special.symbol": { "color": "#0550AE" },
      "number": { "color": "#0550AE" },
      "boolean": { "color": "#0550AE" },
      "constant": { "color": "#0550AE" },
      "attribute": { "color": "#0550AE" },
      "property": { "color": "#0550AE" },
      "variable.special": { "color": "#953800" },
      "type": { "color": "#6639BA" },
      "constructor": { "color": "#6639BA" },
      "function": { "color": "#6639BA" },
      "title": { "color": "#6639BA" },
      "variable": { "color": "#1F2328" },
      "emphasis": { "font_style": "italic" },
      "emphasis.strong": { "font_weight": 700 }
    }
  }
}"##;
