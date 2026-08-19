//! Theme + low-level rendering helpers, ported from the original
//! `zellij-flash`. The original's `render.rs` was a hand-rolled ANSI
//! emitter (`render::flush`) because Zellij's WASM host couldn't use
//! `crossterm` directly. On Herdr the plugin owns a real PTY, so this
//! port uses `ratatui`'s `CrosstermBackend` instead — no ANSI emitter
//! needed, just the theme + span-building helpers that the render methods
//! in `main.rs` call.
//!
//! Phase 2 scope: theme (16 color roles, Catppuccin Macchiato defaults
//! hardcoded), `build_line_spans` (cursor + base ANSI text — jump
//! labels, search highlights, and selection styling arrive in later
//! phases), and the `sel_range_for_line` / `center_x_for_width` helpers.
//!
//! ANSI color reproduction is a permanent capability (merged via
//! `feature/ansi-color`): `build_line_spans` takes `&[StyledChar]`
//! cells that each carry a base `Style` parsed from SGR escapes, and
//! applies overlays with a **replace** policy — the cursor (and later
//! selection/jump/search) cell fully overrides the base ANSI style on
//! the cells it touches. See PLANNING.md §11 "Data model".

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// Parse a "#rrggbb" or "rrggbb" hex string into a ratatui Color.
#[allow(dead_code)]
pub fn parse_hex_color(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Runtime color theme. Defaults to Catppuccin Macchiato. All 16 fields
/// will be overridable via `color_*` keys in `config.toml` (Phase 9);
/// for now they're hardcoded.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Theme {
    pub sel_bg: Color,
    pub sel_fg: Color,
    pub cursor_bg: Color,
    pub cursor_fg: Color,
    pub gutter_cursor: Color,
    pub gutter_dim: Color,
    pub sel_indicator: Color,
    pub footer_dim: Color,
    pub footer_key: Color,
    pub jump_label_bg: Color,
    pub jump_label_fg: Color,
    pub jump_match_fg: Color,
    pub jump_partial_fg: Color,
    pub search_match_bg: Color,
    pub search_current_bg: Color,
    pub search_fg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        // Catppuccin Macchiato palette
        let base = Color::Rgb(36, 39, 58); // #24273a
        let overlay0 = Color::Rgb(110, 115, 141); // #6e738d
        let text = Color::Rgb(202, 211, 245); // #cad3f5
        let yellow = Color::Rgb(238, 212, 159); // #eed49f
        let blue = Color::Rgb(138, 173, 244); // #8aadf4
        let teal = Color::Rgb(139, 213, 202); // #8bd5ca
        let subtext1 = Color::Rgb(184, 192, 224); // #b8c0e0
        let peach = Color::Rgb(245, 169, 127); // #f5a97f
        let red = Color::Rgb(237, 135, 150); // #ed8796
        let green = Color::Rgb(166, 218, 149); // #a6da95
        Self {
            sel_bg: blue,
            sel_fg: base,
            cursor_bg: text,
            cursor_fg: base,
            gutter_cursor: yellow,
            gutter_dim: overlay0,
            sel_indicator: teal,
            footer_dim: overlay0,
            footer_key: subtext1,
            jump_label_bg: peach,
            jump_label_fg: base,
            jump_match_fg: red,
            jump_partial_fg: yellow,
            search_match_bg: green,
            search_current_bg: yellow,
            search_fg: base,
        }
    }
}

/// Compute centered x for a percentage width string, e.g. "90%" → "5%".
/// Returns None for non-percentage or unusual values. (Carried over from
/// the original; unused on Herdr since the popup is centered by the host,
/// but kept for parity reference.)
#[allow(dead_code)]
pub fn center_x_for_width(width: &str) -> Option<String> {
    let pct: u32 = width.strip_suffix('%')?.parse().ok()?;
    if pct >= 100 {
        return Some("0%".to_string());
    }
    Some(format!("{}%", (100 - pct) / 2))
}

/// Selection col range for a single visible line, given the normalized
/// selection (start, end). Returns None if this line is outside the
/// selection. Returned range is (sel_start_col, sel_end_col) in char
/// indices, inclusive. (Used from Phase 4 onward; kept here for parity.)
#[allow(dead_code)]
pub fn sel_range_for_line(
    start: (usize, usize),
    end: (usize, usize),
    line: usize,
    line_len: usize,
) -> Option<(usize, usize)> {
    let (sl, sc) = start;
    let (el, ec) = end;
    if line < sl || line > el {
        return None;
    }
    let col_start = if line == sl { sc } else { 0 };
    let col_end = if line == el {
        ec
    } else {
        line_len.saturating_sub(1)
    };
    Some((col_start, col_end))
}

/// Build ratatui spans for one line of content.
///
/// Cells carry a base `Style` parsed from ANSI SGR escapes. The overlay
/// policy is **replace**: the cursor cell and selection cells fully
/// override the base ANSI style on the cells they touch — they do not
/// merge. This keeps the overlay predictable and matches how the plain-
/// text original looked. Later phases add jump labels and search
/// highlights on top of this — same replace policy.
///
/// Priority per character cell (highest wins):
///   1. Cursor cell → cursor style (inverted), replacing base ANSI style
///   2. Selection cell → selection style (blue bg), replacing base ANSI style
///   3. Base ANSI style from the parsed cell
///
/// `cells`: the visible slice of the line (after horizontal scroll),
/// each carrying its base style.
/// `cursor_col`: display-column index of the cursor on this line, or None.
/// `sel_range`: optional `(start_col, end_col)` inclusive display-column
/// range for the selection on this line, or None if this line isn't in
/// the selection. Coordinates are already in display space (relative to
/// `cells`, i.e. after horizontal scroll).
pub fn build_line_spans(
    cells: &[crate::StyledChar],
    cursor_col: Option<usize>,
    sel_range: Option<(usize, usize)>,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let cursor_style = Style::default().bg(theme.cursor_bg).fg(theme.cursor_fg);
    let sel_style = Style::default().bg(theme.sel_bg).fg(theme.sel_fg);

    let mut styled: Vec<(char, Style)> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            if cursor_col == Some(i) {
                (cell.ch, cursor_style) // cursor replaces base
            } else if let Some((s, e)) = sel_range {
                if i >= s && i <= e {
                    (cell.ch, sel_style) // selection replaces base
                } else {
                    (cell.ch, cell.style)
                }
            } else {
                (cell.ch, cell.style)
            }
        })
        .collect();

    // Cursor or selection past end of line: render a blank styled cell.
    // The cursor wins over selection for the past-end cell (cursor is the
    // active position); a selection extending past EOL only paints the
    // real cells, not a virtual one.
    let past_end = cursor_col.map(|c| c >= cells.len()).unwrap_or(false);
    if past_end {
        styled.push((' ', cursor_style));
    }

    // Merge consecutive cells with the same style into spans.
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style = Style::default();

    for (ch, style) in styled {
        if style != run_style {
            if !run.is_empty() {
                spans.push(Span::styled(run.clone(), run_style));
                run.clear();
            }
            run_style = style;
        }
        run.push(ch);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, run_style));
    }
    if spans.is_empty() {
        spans.push(Span::raw(""));
    }
    spans
}
