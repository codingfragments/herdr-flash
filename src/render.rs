//! Theme + low-level rendering helpers, ported from the original
//! `zellij-flash`. The original's `render.rs` was a hand-rolled ANSI
//! emitter (`render::flush`) because Zellij's WASM host couldn't use
//! `crossterm` directly. On Herdr the plugin owns a real PTY, so this
//! port uses `ratatui`'s `CrosstermBackend` instead — no ANSI emitter
//! needed, just the theme + span-building helpers that the render methods
//! in `main.rs` call.
//!
//! ANSI color reproduction is a permanent capability (merged via
//! `feature/ansi-color`): `build_line_spans` takes `&[StyledChar]`
//! cells that each carry a base `Style` parsed from SGR escapes, and
//! applies overlays with a **replace** policy — every overlay (cursor,
//! selection, jump label, jump match, partial) fully overrides the base
//! ANSI style on the cells it touches. See PLANNING.md §11 "Data model".

use ratatui::style::{Color, Modifier, Style};
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

/// Jump overlay data for one line, in display space.
///
/// `labels` = (display_col, label_char) for labeled matches on this line.
/// `partial_cols` = display cols for partial-match highlights (too many
/// matches to label). `typed_len` is the length of the typed prefix (for
/// prefix-match highlighting on labeled matches).
#[derive(Default)]
pub struct JumpOverlay<'a> {
    pub labels: &'a [(usize, char)],
    pub partial_cols: &'a [usize],
    pub typed_len: usize,
}

/// Search overlay data for one line, in display space.
///
/// `matches` = (display_col, is_current) for search matches on this line.
/// `query_len` is the length of the search query (for highlighting the
/// full match span, not just the start col).
#[derive(Default)]
pub struct SearchOverlay<'a> {
    pub matches: &'a [(usize, bool)],
    pub query_len: usize,
}

/// Build ratatui spans for one line of content.
///
/// Cells carry a base `Style` parsed from ANSI SGR escapes. The overlay
/// policy is **replace**: every overlay (cursor, selection, jump label,
/// jump match, partial) fully overrides the base ANSI style on the cells
/// it touches — no merge. Later phases add search highlights on top of
/// this — same replace policy.
///
/// Priority per character cell (highest wins):
///   1. Jump label → label style (peach bg, bold)
///   2. Jump prefix match → match style (red fg, bold) — chars before the
///      label on a labeled match
///   3. Partial match → partial style (yellow fg, bold) — all typed chars
///      when too many matches to label
///   4. Cursor → cursor style (inverted)
///   5. Current search match → search-current style (yellow bg, bold);
///      non-current search match → search style (green bg)
///   6. Selection → selection style (blue bg)
///   7. Base ANSI style from the parsed cell
///
/// `cells`: the visible slice of the line (after horizontal scroll),
/// each carrying its base style.
/// `cursor_col`: display-column index of the cursor on this line, or None.
/// `sel_range`: optional `(start_col, end_col)` inclusive display-column
/// range for the selection on this line, or None.
/// `sel_pad`: number of selection-styled spaces to append past the line's
/// visible content (block mode only, so the rectangle stays visible on
/// short lines). 0 in stream mode.
/// `jump`: jump overlay data for this line (empty when not in Jump mode).
/// `search`: search overlay data for this line (empty when not in Search mode).
pub fn build_line_spans(
    cells: &[crate::StyledChar],
    cursor_col: Option<usize>,
    sel_range: Option<(usize, usize)>,
    sel_pad: usize,
    jump: JumpOverlay,
    search: SearchOverlay,
    theme: &Theme,
) -> Vec<Span<'static>> {
    let cursor_style = Style::default().bg(theme.cursor_bg).fg(theme.cursor_fg);
    let sel_style = Style::default().bg(theme.sel_bg).fg(theme.sel_fg);
    let label_style = Style::default()
        .bg(theme.jump_label_bg)
        .fg(theme.jump_label_fg)
        .add_modifier(Modifier::BOLD);
    let match_style = Style::default()
        .fg(theme.jump_match_fg)
        .add_modifier(Modifier::BOLD);
    let partial_style = Style::default()
        .fg(theme.jump_partial_fg)
        .add_modifier(Modifier::BOLD);
    let search_style = Style::default()
        .bg(theme.search_match_bg)
        .fg(theme.search_fg);
    let search_cur_style = Style::default()
        .bg(theme.search_current_bg)
        .fg(theme.search_fg)
        .add_modifier(Modifier::BOLD);

    let mut styled: Vec<(char, Style)> = cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            // 1. Jump label (highest).
            if let Some(&(_, lc)) = jump.labels.iter().find(|&&(lc, _)| lc == i) {
                return (lc, label_style);
            }
            // 2. Jump prefix match chars (labeled matches: chars before the label).
            let in_jump_match = jump.typed_len > 1
                && jump.labels.iter().any(|&(label_col, _)| {
                    let start = label_col.saturating_sub(jump.typed_len - 1);
                    i >= start && i < label_col
                });
            if in_jump_match {
                return (cell.ch, match_style);
            }
            // 3. Partial match chars (too many matches: highlight all typed chars).
            let in_partial = jump.typed_len > 0
                && jump.partial_cols.iter().any(|&label_col| {
                    let start = label_col.saturating_sub(jump.typed_len - 1);
                    i >= start && i <= label_col
                });
            if in_partial {
                return (cell.ch, partial_style);
            }
            // 4. Cursor.
            if cursor_col == Some(i) {
                return (cell.ch, cursor_style);
            }
            // 5. Search match (current > non-current).
            if search.query_len > 0 {
                for &(mc, is_cur) in search.matches {
                    if i >= mc && i < mc + search.query_len {
                        return (
                            cell.ch,
                            if is_cur {
                                search_cur_style
                            } else {
                                search_style
                            },
                        );
                    }
                }
            }
            // 6. Selection.
            if let Some((s, e)) = sel_range {
                if i >= s && i <= e {
                    return (cell.ch, sel_style);
                }
            }
            // 7. Base ANSI style.
            (cell.ch, cell.style)
        })
        .collect();

    // Past-end rendering. Two cases can append blank styled cells beyond
    // the line's visible content:
    //   - Cursor past end of line (cursor_col >= cells.len()): one cell,
    //     cursor style — unless it falls inside the selection (then sel).
    //   - Block selection past end (sel_pad > 0): `sel_pad` cells with
    //     selection style, so the rectangle stays visible on short lines.
    // When the cursor (a block corner) sits past EOL *inside* the pad,
    // that one cell shows the cursor style instead of the selection
    // style, so the corner stays visible. (sel_pad is 0 in stream mode,
    // so the cursor past-end path is the stream-mode behavior, unchanged.)
    let cursor_past_end = cursor_col.map(|c| c >= cells.len()).unwrap_or(false);
    if sel_pad > 0 {
        let start = cells.len();
        for i in 0..sel_pad {
            let col = start + i;
            let style = if cursor_past_end && cursor_col == Some(col) {
                cursor_style
            } else {
                sel_style
            };
            styled.push((' ', style));
        }
    } else if cursor_past_end {
        let style = if let Some((s, e)) = sel_range {
            if cells.len() >= s && cells.len() <= e {
                sel_style
            } else {
                cursor_style
            }
        } else {
            cursor_style
        };
        styled.push((' ', style));
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
