//! `herdr-flash` — Phase 2: scrollback view with relative line numbers,
//! cursor, footer, arrow-key + half-page navigation.
//!
//! The popup opens a real PTY (Herdr popup placement). This binary reads
//! the source pane's scrollback via `pane.read`, renders it with `ratatui`
//! driving a `crossterm` backend directly, and runs an event loop until
//! `Esc` closes the popup.

mod flash;
mod render;
mod socket_client;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as CtEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use crossterm::{cursor, execute};
use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::Terminal;

use render::Theme;

/// Debounce window for identical consecutive key events.
///
/// Herdr's popup PTY runs in legacy keyboard mode, where crossterm can't
/// distinguish a genuine press from an OS key-repeat or a flaky
/// double-delivery — every key event arrives as `KeyEventKind::Press`.
/// A single tap can therefore produce two `Press` events and fire the
/// bound action twice (visible as the cursor moving to its first stop,
/// then jumping again). We skip an identical key+modifiers event that
/// arrives within this window of the previous one. Different keys always
/// pass; legitimate rapid distinct-key input is unaffected. Tuned to
/// catch near-instant duplicate delivery without eating deliberate
/// fast double-taps (a human double-tap is ~100ms+ apart).
const KEY_DEBOUNCE: Duration = Duration::from_millis(40);

// ── Launch context ────────────────────────────────────────────────────────────

/// Launch context: which pane this popup was opened relative to.
struct LaunchContext {
    focused_pane_id: String,
}

/// Reads the launch context from `HERDR_PLUGIN_CONTEXT_JSON` (set by Herdr
/// for a real plugin-pane invocation). Falls back to `HERDR_ACTIVE_PANE_ID`
/// for manual dev-testing.
fn launch_context() -> Result<LaunchContext, String> {
    if let Ok(context_json) = std::env::var("HERDR_PLUGIN_CONTEXT_JSON") {
        let context: serde_json::Value = serde_json::from_str(&context_json)
            .map_err(|e| format!("invalid context JSON: {e}"))?;
        let focused_pane_id = context
            .get("focused_pane_id")
            .and_then(|v| v.as_str())
            .ok_or(
                "context JSON has no focused_pane_id (nothing was focused before this popup opened)",
            )?
            .to_string();
        return Ok(LaunchContext { focused_pane_id });
    }
    let focused_pane_id = std::env::var("HERDR_ACTIVE_PANE_ID").map_err(|_| {
        "neither HERDR_PLUGIN_CONTEXT_JSON nor HERDR_ACTIVE_PANE_ID is set".to_string()
    })?;
    Ok(LaunchContext { focused_pane_id })
}

/// Read the source pane's scrollback via `pane.read` with
/// `source = "recent_unwrapped"`, requesting `format = "ansi"` +
/// `strip_ansi = false` so the response carries raw SGR escapes. The
/// caller parses these into per-char ratatui `Style` cells via
/// `parse_ansi_lines` — see the "Data model" note in PLANNING.md §11.
/// (The plain-text parity path would be `format = "text"` +
/// `strip_ansi = true`, the API default; this plugin adopts ANSI color
/// as a permanent beyond-parity capability.)
fn read_scrollback(socket_path: &str, pane_id: &str) -> Result<String, String> {
    let params = serde_json::json!({
        "pane_id": pane_id,
        "source": "recent_unwrapped",
        "format": "ansi",
        "strip_ansi": false,
    });
    let result = socket_client::request(socket_path, "pane.read", params)
        .map_err(|e| format!("pane.read failed: {e}"))?;
    result
        .get("read")
        .and_then(|v| v.get("text"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "pane.read response had no \"read.text\" field".to_string())
}

// ── ANSI → styled cells ───────────────────────────────────────────────────────

/// One character cell carrying its base style parsed from ANSI SGR.
///
/// This is the line model for the whole plugin: `State::lines` holds
/// `Vec<Vec<StyledChar>>`. Later overlays (cursor, selection, jump
/// labels, search highlights) *replace* this base style on the cells
/// they touch — the overlay policy (see PLANNING.md §11 "Data model").
/// Text-only operations (motions, jump matching, search matching, copy,
/// insert) read the `ch` field or use `styled_line_to_plain_text`.
#[derive(Clone, Copy, Debug)]
pub struct StyledChar {
    pub ch: char,
    pub style: Style,
}

/// Parse ANSI-styled text into one `Vec<StyledChar>` per line.
///
/// `ansi-to-tui`'s `IntoText` parses the whole block in one pass so SGR
/// state carries across line boundaries correctly; we then flatten each
/// resulting `Line`'s spans into per-char `(char, Style)` cells. On any
/// parse failure we fall back to plain unstyled chars so the view never
/// breaks — color reproduction is a best-effort enhancement, not a
/// crash-the-plugin concern.
fn parse_ansi_lines(text: &str) -> Vec<Vec<StyledChar>> {
    use ansi_to_tui::IntoText as _;
    match text.into_text() {
        Ok(t) => t
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .flat_map(|span| {
                        span.content.chars().map(move |ch| StyledChar {
                            ch,
                            style: span.style,
                        })
                    })
                    .collect::<Vec<StyledChar>>()
            })
            .collect(),
        Err(_) => text
            .lines()
            .map(|l| {
                l.chars()
                    .map(|ch| StyledChar {
                        ch,
                        style: Style::default(),
                    })
                    .collect()
            })
            .collect(),
    }
}

/// Extract plain text (no styles) from a line of styled cells.
///
/// This is the conversion contract for the action boundary: copy
/// (`arboard`) and insert (`pane.send_text`) both target plain `String`,
/// so the styled-cell model used for rendering must be flattened back
/// to text. Search and jump matching also operate on plain text
/// extracted this way, not on the `Style` field.
#[allow(dead_code)] // used from Phase 4 (selected_text) / Phase 8 (copy/insert)
fn styled_line_to_plain_text(line: &[StyledChar]) -> String {
    line.iter().map(|c| c.ch).collect()
}

/// Extract the full captured text as plain lines joined by `\n`.
#[allow(dead_code)] // used from Phase 8 (copy/insert of multi-line selections)
fn styled_lines_to_plain_text(lines: &[Vec<StyledChar>]) -> String {
    lines
        .iter()
        .map(|line| styled_line_to_plain_text(line))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Mode ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Mode {
    Normal,
    /// Word-jump: user types a prefix, labels appear on visible matches.
    /// `labels` = (line, jump_col, label_col, label_char), sorted by distance
    /// from cursor. `jump_col` is the match start; `label_col` is where the
    /// label glyph is rendered (last char of the prefix, clamped to the
    /// last real character so EOL matches don't render past the line end).
    /// `partial_matches` = (line, jump_col, label_col) for matches when
    /// there are too many to label (partial-highlight fallback, no labels).
    /// `start_selection` = true when entered via `S` (plant anchor on jump).
    Jump {
        typed: String,
        labels: Vec<flash::JumpLabel>,
        partial_matches: Vec<flash::PartialMatch>,
        start_selection: bool,
    },
    /// Line-jump: every visible line (except the cursor line) gets a
    /// label in the gutter. `labels` = (line, label_char). Typing a
    /// label jumps the cursor to that line (column preserved, clamped).
    /// `start_selection` = true when entered via `L` (plant anchor on jump).
    LineJump {
        labels: Vec<(usize, char)>,
        start_selection: bool,
    },
    // Search, Confirm arrive in Phases 7–8.
}

// ── State ────────────────────────────────────────────────────────────────────

struct State {
    lines: Vec<Vec<StyledChar>>,
    cursor: (usize, usize),
    /// Selection anchor. When `Some`, the selection spans
    /// `min(anchor, cursor)..=max(anchor, cursor)` in stream order and
    /// renders with a blue background (replace policy — overrides the
    /// base ANSI style on touched cells). Orthogonal to `mode`: every
    /// cursor move (arrows, motions — and later, jump/search-nav) extends
    /// the selection while the anchor is set. `Space` toggles it; the Esc
    /// chain clears it before closing the popup.
    anchor: Option<(usize, usize)>,
    /// Preferred column for vertical movement (vim-style): moving up/down
    /// clamps the cursor to the line length but remembers this value,
    /// snapping back to it when a later line is long enough. Horizontal
    /// moves update it; vertical moves don't.
    preferred_col: usize,
    scroll_y: usize,
    scroll_x: usize,
    content_rows: usize,
    content_cols: usize,
    theme: Theme,
    #[allow(dead_code)]
    mode: Mode,
    message: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            cursor: (0, 0),
            anchor: None,
            preferred_col: 0,
            scroll_y: 0,
            scroll_x: 0,
            content_rows: 24,
            content_cols: 80,
            theme: Theme::default(),
            mode: Mode::Normal,
            message: None,
        }
    }
}

impl State {
    // ── Cursor movement ───────────────────────────────────────────────────────

    fn line_len(&self, line: usize) -> usize {
        self.lines.get(line).map(|l| l.len()).unwrap_or(0)
    }

    /// Character at a position, reading the `ch` field of the styled cell.
    /// Returns `None` at EOL (col == line_len) or past the buffer end —
    /// matching the original's `char_at` semantics so the motion helpers
    /// port unchanged. Text operations read chars, never styles (see the
    /// "Data model" note in PLANNING.md §11).
    fn char_at(&self, line: usize, col: usize) -> Option<char> {
        self.lines.get(line)?.get(col).map(|c| c.ch)
    }

    fn move_up(&mut self) {
        if self.cursor.0 == 0 {
            return;
        }
        self.cursor.0 -= 1;
        self.cursor.1 = self.preferred_col.min(self.line_len(self.cursor.0));
        self.scroll_cursor_into_view();
    }

    fn move_down(&mut self) {
        if self.cursor.0 + 1 >= self.lines.len() {
            return;
        }
        self.cursor.0 += 1;
        self.cursor.1 = self.preferred_col.min(self.line_len(self.cursor.0));
        self.scroll_cursor_into_view();
    }

    fn move_left(&mut self) {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
            self.preferred_col = self.cursor.1;
            self.scroll_x_into_view();
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.line_len(self.cursor.0);
            self.preferred_col = self.cursor.1;
            self.scroll_cursor_into_view();
        }
    }

    fn move_right(&mut self) {
        let len = self.line_len(self.cursor.0);
        if self.cursor.1 < len {
            self.cursor.1 += 1;
            self.preferred_col = self.cursor.1;
            self.scroll_x_into_view();
        } else if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
            self.preferred_col = 0;
            self.scroll_cursor_into_view();
        }
    }

    fn page_up(&mut self) {
        let half = (self.content_rows / 2).max(1);
        self.cursor.0 = self.cursor.0.saturating_sub(half);
        self.cursor.1 = self.preferred_col.min(self.line_len(self.cursor.0));
        self.recenter_scroll();
    }

    fn page_down(&mut self) {
        let half = (self.content_rows / 2).max(1);
        let last = self.lines.len().saturating_sub(1);
        self.cursor.0 = (self.cursor.0 + half).min(last);
        self.cursor.1 = self.preferred_col.min(self.line_len(self.cursor.0));
        self.recenter_scroll();
    }

    // ── Word motions (Phase 3) ───────────────────────────────────────────────
    //
    // Ported from the original `zellij-flash`. The only adaptation is
    // `char_at`, which reads `StyledChar.ch` instead of indexing a
    // `String`. The motion bodies, `cclass`, `next_pos`/`prev_pos` are
    // verbatim — they operate on chars, never styles ("Data model" note,
    // PLANNING.md §11). All motions end with `scroll_cursor_into_view`
    // so the viewport follows the cursor exactly as arrow keys do.

    /// Advance one step in stream order, wrapping at line ends.
    fn next_pos(&self, line: usize, col: usize) -> Option<(usize, usize)> {
        if col < self.line_len(line) {
            Some((line, col + 1))
        } else if line + 1 < self.lines.len() {
            Some((line + 1, 0))
        } else {
            None
        }
    }

    /// Retreat one step in stream order, wrapping at line starts.
    fn prev_pos(&self, line: usize, col: usize) -> Option<(usize, usize)> {
        if col > 0 {
            Some((line, col - 1))
        } else if line > 0 {
            Some((line - 1, self.line_len(line - 1)))
        } else {
            None
        }
    }

    /// Character class at a position. EOL (col == line_len) counts as Space.
    /// `wide = true` → WORD mode: only Space vs NonSpace.
    /// `wide = false` → word mode: Space | Word | Other.
    fn cclass(&self, line: usize, col: usize, wide: bool) -> u8 {
        match self.char_at(line, col) {
            None => 0, // EOL = space
            Some(c) if c.is_whitespace() => 0,
            Some(_) if wide => 1,
            Some(c) if c.is_alphanumeric() || c == '_' => 1,
            Some(_) => 2,
        }
    }

    /// `w` / `W` — forward to start of next word.
    fn motion_w(&mut self, wide: bool) {
        let (mut line, mut col) = self.cursor;
        let start = self.cclass(line, col, wide);
        // Skip current class run.
        while let Some((nl, nc)) = self.next_pos(line, col) {
            (line, col) = (nl, nc);
            if self.cclass(line, col, wide) != start {
                break;
            }
        }
        // Skip spaces.
        while self.cclass(line, col, wide) == 0 {
            let Some((nl, nc)) = self.next_pos(line, col) else {
                break;
            };
            (line, col) = (nl, nc);
        }
        self.cursor = (line, col);
        self.scroll_cursor_into_view();
    }

    /// `b` / `B` — backward to start of previous word.
    fn motion_b(&mut self, wide: bool) {
        let (mut line, mut col) = self.cursor;
        // Retreat one step first.
        let Some((nl, nc)) = self.prev_pos(line, col) else {
            return;
        };
        (line, col) = (nl, nc);
        // Skip spaces backward.
        while self.cclass(line, col, wide) == 0 {
            let Some((nl, nc)) = self.prev_pos(line, col) else {
                break;
            };
            (line, col) = (nl, nc);
        }
        // Skip same-class run backward to find its start.
        let target = self.cclass(line, col, wide);
        while let Some((nl, nc)) = self.prev_pos(line, col) {
            if self.cclass(nl, nc, wide) == target {
                (line, col) = (nl, nc);
            } else {
                break;
            }
        }
        self.cursor = (line, col);
        self.scroll_cursor_into_view();
    }

    /// `e` / `E` — forward to end of current / next word.
    fn motion_e(&mut self, wide: bool) {
        let (mut line, mut col) = self.cursor;
        // Advance one step first.
        let Some((nl, nc)) = self.next_pos(line, col) else {
            return;
        };
        (line, col) = (nl, nc);
        // Skip spaces.
        while self.cclass(line, col, wide) == 0 {
            let Some((nl, nc)) = self.next_pos(line, col) else {
                break;
            };
            (line, col) = (nl, nc);
        }
        // Advance through current class run until class changes.
        let target = self.cclass(line, col, wide);
        while let Some((nl, nc)) = self.next_pos(line, col) {
            if self.cclass(nl, nc, wide) == target {
                (line, col) = (nl, nc);
            } else {
                break;
            }
        }
        self.cursor = (line, col);
        self.scroll_cursor_into_view();
    }

    /// `0` — start of line.
    fn motion_line_start(&mut self) {
        self.cursor.1 = 0;
        self.scroll_x_into_view();
    }

    /// `$` — end of line (last char, not past it).
    fn motion_line_end(&mut self) {
        let len = self.line_len(self.cursor.0);
        self.cursor.1 = len.saturating_sub(1);
        self.scroll_x_into_view();
    }

    // ── Selection (Phase 4) ───────────────────────────────────────────────────
    //
    // Ported from the original. `anchor` is orthogonal to `mode`: every
    // cursor move (arrows, motions — and later, jump/search-nav) extends
    // the selection while the anchor is set. `Space` toggles; the Esc
    // chain clears the anchor before closing the popup. Selection renders
    // with the replace policy (blue bg overrides base ANSI style).

    /// Normalized selection range `(start, end)` in stream order, or None
    /// when no anchor is set. `start <= end` lexicographically.
    fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.anchor?;
        let cursor = self.cursor;
        if anchor <= cursor {
            Some((anchor, cursor))
        } else {
            Some((cursor, anchor))
        }
    }

    /// Extract the selected text as a plain string (styles stripped) via
    /// `styled_line_to_plain_text`. This is the conversion contract for
    /// the action boundary (copy/insert land in Phase 8) — no style bytes
    /// reach the output.
    #[allow(dead_code)] // used from Phase 8 (copy/insert)
    fn selected_text(&self) -> Option<String> {
        let ((sl, sc), (el, ec)) = self.selection_range()?;

        if sl == el {
            let line = self.lines.get(sl)?;
            let start = sc.min(line.len());
            let end = (ec + 1).min(line.len());
            return Some(styled_line_to_plain_text(&line[start..end]));
        }

        let mut out = String::new();
        if let Some(line) = self.lines.get(sl) {
            let start = sc.min(line.len());
            out.push_str(&styled_line_to_plain_text(&line[start..]));
            out.push('\n');
        }
        for l in sl + 1..el {
            if let Some(line) = self.lines.get(l) {
                out.push_str(&styled_line_to_plain_text(line));
                out.push('\n');
            }
        }
        if let Some(line) = self.lines.get(el) {
            let end = (ec + 1).min(line.len());
            out.push_str(&styled_line_to_plain_text(&line[..end]));
        }
        Some(out)
    }

    /// `(line_count, char_count)` for the active selection, or None.
    /// Used by the footer's `SEL N lines M chars` indicator.
    fn selection_info(&self) -> Option<(usize, usize)> {
        let ((sl, sc), (el, ec)) = self.selection_range()?;
        let lines = el - sl + 1;
        let chars = if sl == el {
            ec.saturating_sub(sc) + 1
        } else {
            let first = self.line_len(sl).saturating_sub(sc) + 1; // +1 for newline
            let last = ec + 1;
            let mid: usize = (sl + 1..el).map(|l| self.line_len(l) + 1).sum();
            first + mid + last
        };
        Some((lines, chars))
    }

    // ── Word-jump (Phase 5) ───────────────────────────────────────────────────

    /// Jump the cursor to `(line, col)` and recenter the viewport.
    fn jump_to(&mut self, line: usize, col: usize) {
        self.cursor = (line, col);
        self.recenter_scroll();
    }

    /// Recompute labels for the current typed prefix and update `mode`.
    fn recompute_jump(&mut self, typed: String, start_selection: bool) {
        let (labels, partial_matches) = flash::compute_jump_labels(
            &self.lines,
            self.scroll_y,
            self.content_rows,
            self.cursor,
            &typed,
        );
        self.mode = Mode::Jump {
            typed,
            labels,
            partial_matches,
            start_selection,
        };
    }

    /// Handle a key while in `Mode::Jump`. Returns `true` if the popup
    /// should stay open (always — jump never closes the popup).
    fn handle_key_jump(
        &mut self,
        key: &crossterm::event::KeyEvent,
        typed: String,
        labels: Vec<flash::JumpLabel>,
        start_selection: bool,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Backspace => {
                let mut t = typed;
                t.pop();
                self.recompute_jump(t, start_selection);
            }
            KeyCode::Char(c) => {
                // If labels are showing and c matches a label → jump.
                if !labels.is_empty() {
                    if let Some(&(line, jump_col, _, _)) =
                        labels.iter().find(|&&(_, _, _, lc)| lc == c)
                    {
                        self.jump_to(line, jump_col);
                        if start_selection {
                            self.anchor = Some(self.cursor);
                        }
                        self.mode = Mode::Normal;
                        return true;
                    }
                }
                // Otherwise append to the search string and recompute.
                // Accept printable chars with no modifiers or Shift-only
                // (crossterm delivers uppercase when Shift is held).
                let only_shift = key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT);
                if !c.is_control() && (key.modifiers.is_empty() || only_shift) {
                    let mut t = typed;
                    t.push(c);
                    self.recompute_jump(t, start_selection);
                }
            }
            _ => {}
        }
        true
    }

    /// Handle a key while in `Mode::LineJump`. Returns `true` if the popup
    /// should stay open (always — line-jump never closes the popup).
    fn handle_key_line_jump(
        &mut self,
        key: &crossterm::event::KeyEvent,
        labels: Vec<(usize, char)>,
        start_selection: bool,
    ) -> bool {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Char(c) => {
                if let Some(&(line, _)) = labels.iter().find(|&&(_, lc)| lc == c) {
                    // Preserve col if it fits on the target line, else clamp.
                    let col = self.cursor.1.min(self.line_len(line));
                    self.jump_to(line, col);
                    if start_selection {
                        self.anchor = Some(self.cursor);
                    }
                    self.mode = Mode::Normal;
                } else {
                    // Unrecognised char → cancel line-jump (per the original).
                    self.mode = Mode::Normal;
                }
            }
            _ => {}
        }
        true
    }

    fn scroll_cursor_into_view(&mut self) {
        if self.cursor.0 < self.scroll_y {
            self.scroll_y = self.cursor.0;
        } else if self.cursor.0 >= self.scroll_y + self.content_rows {
            self.scroll_y = self.cursor.0 + 1 - self.content_rows;
        }
        self.scroll_x_into_view();
    }

    fn recenter_scroll(&mut self) {
        let ideal = self.cursor.0.saturating_sub(self.content_rows / 2);
        let max_scroll = self.lines.len().saturating_sub(self.content_rows);
        self.scroll_y = ideal.min(max_scroll);
        self.scroll_x_into_view();
    }

    fn scroll_x_into_view(&mut self) {
        let avail = self.avail_w();
        if avail == 0 {
            return;
        }
        if self.cursor.1 < self.scroll_x {
            self.scroll_x = self.cursor.1;
        } else if self.cursor.1 + 1 >= self.scroll_x + avail {
            // +1 accounts for the `…` indicator occupying the last display
            // column when the line overflows — scroll before the cursor
            // lands on it.
            self.scroll_x = self.cursor.1 + 2 - avail;
        }
    }

    fn gutter_w(&self) -> usize {
        let max_dist = self.content_rows.saturating_sub(1);
        max_dist.to_string().len().max(1) + 2
    }

    fn avail_w(&self) -> usize {
        self.content_cols.saturating_sub(self.gutter_w())
    }

    // ── Rendering ────────────────────────────────────────────────────────────

    fn render_all(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 20 || area.height < 5 {
            Paragraph::new("too small")
                .style(Style::default().fg(self.theme.footer_dim))
                .render(area, buf);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(4)])
            .split(area);

        self.render_content(chunks[0], buf);
        self.render_footer(chunks[1], buf);
    }

    fn render_content(&self, area: Rect, buf: &mut Buffer) {
        let inner = area;

        if self.lines.is_empty() {
            Paragraph::new("No content captured.")
                .style(Style::default().fg(self.theme.footer_dim))
                .render(inner, buf);
            return;
        }

        let viewport_h = inner.height as usize;
        let total = self.lines.len();
        let cursor_line = self.cursor.0.min(total.saturating_sub(1));
        let cursor_col = self.cursor.1;

        let scroll_y = self.scroll_y.min(total.saturating_sub(1));
        let visible_end = (scroll_y + viewport_h).min(total);
        let visible = &self.lines[scroll_y..visible_end];

        let max_dist = viewport_h.saturating_sub(1);
        let num_w = max_dist.to_string().len().max(1);
        let gutter_w = num_w + 2;
        let avail_w = (inner.width as usize).saturating_sub(gutter_w);

        let gutter_dim = Style::default()
            .fg(self.theme.gutter_dim)
            .add_modifier(Modifier::DIM);
        let gutter_cursor_style = Style::default()
            .fg(self.theme.gutter_cursor)
            .add_modifier(Modifier::BOLD);
        // When a selection anchor is set, the current-line gutter marker
        // (► + line number) switches to the selection indicator color
        // (teal) — an always-in-view cue that selection mode is active,
        // independent of where the cursor sits relative to the anchor.
        let gutter_sel_style = Style::default()
            .fg(self.theme.sel_indicator)
            .add_modifier(Modifier::BOLD);

        // Normalized selection range (stream order), computed once for the
        // whole viewport. Per-line display ranges are derived inside the
        // map via `sel_range_for_line`, then shifted into display space
        // (relative to scroll_x).
        let sel = self.selection_range();

        // Jump overlay data (Phase 5): extract from Mode::Jump once for
        // the viewport. Per-line labels/partials are filtered inside the
        // map and shifted to display space.
        let (jump_typed_len, jump_all_labels, jump_all_partials) = match &self.mode {
            Mode::Jump {
                typed,
                labels,
                partial_matches,
                start_selection: _,
            } => (
                typed.chars().count(),
                labels.clone(),
                partial_matches.clone(),
            ),
            _ => (0, Vec::new(), Vec::new()),
        };

        // Line-jump labels (Phase 6): (line, label_char) for the gutter.
        let line_jump_labels: &[(usize, char)] =
            if let Mode::LineJump { ref labels, .. } = self.mode {
                labels
            } else {
                &[]
            };
        let line_jump_label_style = Style::default()
            .bg(self.theme.jump_label_bg)
            .fg(self.theme.jump_label_fg)
            .add_modifier(Modifier::BOLD);

        let content_lines: Vec<Line<'static>> = visible
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let abs = scroll_y + i;
                let is_cursor_line = abs == cursor_line;
                let dist = (abs as isize - cursor_line as isize).unsigned_abs();

                let (gutter_str, gutter_style) =
                    if let Some(&(_, lc)) = line_jump_labels.iter().find(|&&(l, _)| l == abs) {
                        // In LineJump mode, replace the gutter number with the label.
                        (format!("{:>w$}  ", lc, w = num_w), line_jump_label_style)
                    } else {
                        (
                            format!(
                                "{:>w$}{}",
                                dist,
                                if is_cursor_line { "► " } else { "  " },
                                w = num_w
                            ),
                            if is_cursor_line {
                                if self.anchor.is_some() {
                                    gutter_sel_style
                                } else {
                                    gutter_cursor_style
                                }
                            } else {
                                gutter_dim
                            },
                        )
                    };
                let gutter = Span::styled(gutter_str, gutter_style);

                let scroll_x = self.scroll_x;
                let logical_len = text.len();
                let has_right_overflow = logical_len > scroll_x + avail_w;
                let has_left_overflow = scroll_x > 0;

                let visible_w = if has_right_overflow {
                    avail_w.saturating_sub(1)
                } else {
                    avail_w
                };
                let cells: Vec<StyledChar> = text
                    .iter()
                    .copied()
                    .skip(scroll_x)
                    .take(visible_w)
                    .collect();

                let cur_col = if is_cursor_line {
                    Some(cursor_col.saturating_sub(scroll_x))
                } else {
                    None
                };

                // Selection range for this line, in display space.
                // `sel_range_for_line` returns logical (start_col, end_col)
                // inclusive; shift by scroll_x so they index into `cells`.
                // Skip if the selection on this line is entirely off-screen
                // (ends before scroll_x, or starts after the visible window).
                let sel_disp = sel
                    .and_then(|(s, e)| render::sel_range_for_line(s, e, abs, logical_len))
                    .and_then(|(s, e)| {
                        let visible_end = scroll_x + cells.len();
                        if e < scroll_x || s >= visible_end {
                            None
                        } else {
                            let ds = s.saturating_sub(scroll_x);
                            let de = e
                                .min(visible_end.saturating_sub(1))
                                .saturating_sub(scroll_x);
                            Some((ds, de.min(cells.len().saturating_sub(1))))
                        }
                    });

                // Jump overlay for this line: filter labels and partials
                // to those on `abs`, shift label_col to display space.
                let line_labels: Vec<(usize, char)> = jump_all_labels
                    .iter()
                    .filter(|&&(l, _, _, _)| l == abs)
                    .filter_map(|&(_, _, label_col, lc)| {
                        let disp = label_col.saturating_sub(scroll_x);
                        if disp < visible_w {
                            Some((disp, lc))
                        } else {
                            None
                        }
                    })
                    .collect();
                let line_partials: Vec<usize> = jump_all_partials
                    .iter()
                    .filter(|&&(l, _, _)| l == abs)
                    .filter_map(|&(_, _, label_col)| {
                        let disp = label_col.saturating_sub(scroll_x);
                        if disp < visible_w {
                            Some(disp)
                        } else {
                            None
                        }
                    })
                    .collect();
                let jump = render::JumpOverlay {
                    labels: &line_labels,
                    partial_cols: &line_partials,
                    typed_len: jump_typed_len,
                };

                let mut spans = vec![gutter];
                if has_left_overflow {
                    spans.push(Span::styled(
                        "…",
                        Style::default().fg(self.theme.footer_dim),
                    ));
                }
                spans.extend(render::build_line_spans(
                    &cells,
                    cur_col,
                    sel_disp,
                    jump,
                    &self.theme,
                ));
                if has_right_overflow {
                    spans.push(Span::styled(
                        "…",
                        Style::default().fg(self.theme.footer_dim),
                    ));
                }
                Line::from(spans)
            })
            .collect();

        Paragraph::new(content_lines).render(inner, buf);
    }

    fn render_footer(&self, area: Rect, buf: &mut Buffer) {
        let bold = Style::default()
            .fg(self.theme.footer_key)
            .add_modifier(Modifier::BOLD);
        let dim = Style::default().fg(self.theme.footer_dim);

        let (cline, ccol) = self.cursor;
        let pos_str = if self.scroll_x > 0 {
            format!("{}:{}  +{}", cline + 1, ccol + 1, self.scroll_x)
        } else {
            format!("{}:{}", cline + 1, ccol + 1)
        };

        // Status line: profile label, line count, cursor pos, selection info.
        let mut line1_spans = vec![
            Span::raw(" "),
            Span::styled("[scrollback]", dim),
            Span::raw("  "),
            Span::styled(format!("{} lines", self.lines.len()), dim),
            Span::raw("  "),
            Span::styled(pos_str, dim),
        ];
        if let Some((nlines, nchars)) = self.selection_info() {
            line1_spans.push(Span::raw("  "));
            line1_spans.push(Span::styled(
                format!("SEL {} lines {} chars", nlines, nchars),
                Style::default().fg(self.theme.sel_indicator),
            ));
        }
        let line1 = Line::from(line1_spans);

        // Key-hint line: in Jump/LineJump mode, show the mode state;
        // otherwise the Normal-mode keymap.
        let line2 = if let Mode::Jump {
            typed,
            labels,
            partial_matches,
            start_selection,
        } = &self.mode
        {
            let prefix = if *start_selection { "[SEL] " } else { "" };
            let hint = if !partial_matches.is_empty() {
                format!(
                    "{}jump: {}  ({} matches, keep typing…)",
                    prefix,
                    typed,
                    partial_matches.len()
                )
            } else if labels.is_empty() && !typed.is_empty() {
                format!("{}jump: {}  (no matches)", prefix, typed)
            } else if labels.is_empty() {
                format!("{}jump: type to search…", prefix)
            } else {
                format!("{}jump: {}  ({} matches)", prefix, typed, labels.len())
            };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    hint,
                    Style::default()
                        .fg(self.theme.jump_label_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Esc", bold),
                Span::raw(":cancel"),
            ])
        } else if let Mode::LineJump {
            labels,
            start_selection,
        } = &self.mode
        {
            let prefix = if *start_selection { "[SEL] " } else { "" };
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{}line jump — {} lines labeled", prefix, labels.len()),
                    Style::default()
                        .fg(self.theme.jump_label_bg)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Esc", bold),
                Span::raw(":cancel"),
            ])
        } else {
            let mut line2_spans = vec![
                Span::raw(" "),
                Span::styled("↑↓←→", bold),
                Span::raw(":move  "),
                Span::styled("w/W/b/B/e/E", bold),
                Span::raw(":word  "),
                Span::styled("s/S", bold),
                Span::raw(":jump  "),
                Span::styled("l/L", bold),
                Span::raw(":line  "),
                Span::styled("Space", bold),
                Span::raw(":select  "),
                Span::styled("Esc", bold),
                Span::raw(":close"),
            ];
            if let Some(msg) = &self.message {
                line2_spans.push(Span::raw("    "));
                line2_spans.push(Span::styled(
                    msg.clone(),
                    Style::default().fg(self.theme.sel_indicator),
                ));
            }
            Line::from(line2_spans)
        };

        Paragraph::new(vec![line1, line2])
            .block(Block::default().borders(Borders::ALL))
            .render(area, buf);
    }
}

// ── Terminal setup + event loop ────────────────────────────────────────────────

fn run(state: &mut State) -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    // ratatui's diff renderer assumes it starts from a blank terminal;
    // without this, cells that render blank in the first frame don't get
    // force-written, leaving old scrollback showing through.
    terminal.clear()?;

    let result = run_loop(&mut terminal, state);

    disable_raw_mode()?;
    let _ = execute!(terminal.backend_mut(), cursor::Show);
    let _ = terminal.clear();

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut State,
) -> io::Result<()> {
    // Last key event processed, for same-key debounce (see KEY_DEBOUNCE).
    let mut last_key: Option<(KeyCode, KeyModifiers, Instant)> = None;

    loop {
        // Update geometry + clamp scroll_y BEFORE drawing so the first
        // frame is correct (otherwise the first draw uses the stale
        // default content_rows=24, putting the cursor mid-screen, then
        // the clamp on the next iteration causes a visible jump).
        state.content_rows = terminal.size()?.height.saturating_sub(4) as usize;
        state.content_cols = terminal.size()?.width as usize;
        let max_scroll = state.lines.len().saturating_sub(state.content_rows);
        state.scroll_y = state.scroll_y.min(max_scroll);

        terminal.draw(|f| state.render_all(f.area(), f.buffer_mut()))?;

        let CtEvent::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // Same-key debounce: skip an identical key+modifiers event
        // arriving within KEY_DEBOUNCE of the previous one. This absorbs
        // flaky double-delivery and the first OS key-repeat tick in
        // legacy keyboard mode, where repeats are indistinguishable from
        // presses. Different keys always pass.
        let now = Instant::now();
        let is_duplicate = last_key.is_some_and(|(code, mods, when)| {
            code == key.code && mods == key.modifiers && now.duration_since(when) < KEY_DEBOUNCE
        });
        last_key = Some((key.code, key.modifiers, now));
        if is_duplicate {
            continue;
        }

        // Any keypress clears the transient message.
        state.message = None;

        let only_shift = key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);

        // ── Mode dispatch (Phase 5+) ────────────────────────────────────
        // When in a mode (Jump/LineJump/Search/Confirm), keys go to the
        // mode handler, not the Normal-mode keymap. Esc is handled inside
        // each mode handler (it cancels the mode without touching the
        // anchor, per the original).
        if let Mode::Jump {
            ref typed,
            ref labels,
            partial_matches: _,
            start_selection,
        } = state.mode
        {
            state.handle_key_jump(&key, typed.clone(), labels.clone(), start_selection);
            continue;
        }
        if let Mode::LineJump {
            ref labels,
            start_selection,
        } = state.mode
        {
            state.handle_key_line_jump(&key, labels.clone(), start_selection);
            continue;
        }

        match key.code {
            // Esc cancel chain (Phase 4/5): in a mode → cancel mode (handled
            // above before reaching here); else if anchor set → clear anchor;
            // else → close the popup.
            KeyCode::Esc => {
                if state.anchor.is_some() {
                    state.anchor = None;
                } else {
                    break;
                }
            }
            KeyCode::Up => state.move_up(),
            KeyCode::Down => state.move_down(),
            KeyCode::Left if only_shift => {
                state.scroll_x = state.scroll_x.saturating_sub(5);
            }
            KeyCode::Right if only_shift => {
                let max_x = state
                    .lines
                    .iter()
                    .map(|l| l.len())
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(state.avail_w().saturating_sub(1));
                state.scroll_x = (state.scroll_x + 5).min(max_x);
            }
            KeyCode::Left => state.move_left(),
            KeyCode::Right => state.move_right(),
            KeyCode::PageUp => state.page_up(),
            KeyCode::PageDown => state.page_down(),
            // ── Word motions (Phase 3) ────────────────────────────────────
            // Crossterm delivers the uppercase char when Shift is held, so
            // `W`/`B`/`E` map to the WORD (wide) variants.
            KeyCode::Char('w') => state.motion_w(false),
            KeyCode::Char('W') => state.motion_w(true),
            KeyCode::Char('b') => state.motion_b(false),
            KeyCode::Char('B') => state.motion_b(true),
            KeyCode::Char('e') => state.motion_e(false),
            KeyCode::Char('E') => state.motion_e(true),
            KeyCode::Char('0') => state.motion_line_start(),
            KeyCode::Char('$') => state.motion_line_end(),
            // ── Selection (Phase 4) ─────────────────────────────────────
            // `Space` toggles: set anchor at cursor, or if already set,
            // swap cursor/anchor (jump cursor to the old anchor end, anchor
            // the old cursor). Clearing is via Esc, not Space.
            KeyCode::Char(' ') => {
                if let Some(anchor) = state.anchor {
                    state.anchor = Some(state.cursor);
                    state.cursor = anchor;
                    state.scroll_cursor_into_view();
                } else {
                    state.anchor = Some(state.cursor);
                }
            }
            // ── Word-jump (Phase 5) ──────────────────────────────────────
            // `s` enters Jump mode; `S` (and `Shift-s`) enters select-jump
            // (plants the anchor at the destination on completion).
            KeyCode::Char('s') => {
                state.recompute_jump(String::new(), false);
            }
            KeyCode::Char('S') => {
                state.recompute_jump(String::new(), true);
            }
            // ── Line-jump (Phase 6) ───────────────────────────────────────
            // `l` enters LineJump mode; `L` (and `Shift-l`) enters
            // select-line-jump (plants the anchor at the destination).
            KeyCode::Char('l') => {
                let labels = flash::compute_line_labels(
                    &state.lines,
                    state.scroll_y,
                    state.content_rows,
                    state.cursor,
                );
                state.mode = Mode::LineJump {
                    labels,
                    start_selection: false,
                };
            }
            KeyCode::Char('L') => {
                let labels = flash::compute_line_labels(
                    &state.lines,
                    state.scroll_y,
                    state.content_rows,
                    state.cursor,
                );
                state.mode = Mode::LineJump {
                    labels,
                    start_selection: true,
                };
            }
            _ => {}
        }
    }
    Ok(())
}

fn main() {
    if let Err(message) = (|| {
        let ctx = launch_context()?;
        let socket_path = std::env::var("HERDR_SOCKET_PATH")
            .map_err(|_| "HERDR_SOCKET_PATH is not set".to_string())?;
        let text = read_scrollback(&socket_path, &ctx.focused_pane_id)?;

        let mut state = State {
            lines: parse_ansi_lines(&text),
            ..State::default()
        };
        if state.lines.is_empty() {
            state.lines.push(Vec::new());
        }

        // Start at the bottom of the captured text, matching the original.
        // scroll_y = MAX so the first run_loop clamp brings it to max_scroll
        // (last line at the bottom row — the lowest scroll position). We
        // can't compute max_scroll here because the real terminal height
        // isn't known until the first run_loop iteration.
        let last = state.lines.len().saturating_sub(1);
        state.cursor = (last, 0);
        state.preferred_col = 0;
        state.scroll_y = usize::MAX;

        run(&mut state).map_err(|e| format!("terminal error: {e}"))
    })() {
        eprintln!("herdr-flash error: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    /// Verify the ANSI parser turns SGR escapes into per-char styles.
    /// These pin the core contract: `pane.read(format=ansi)` →
    /// `parse_ansi_lines` → styled cells, with `Color::Reset` treated as
    /// default and SGR state carried across newlines.
    #[test]
    fn parses_sgr_into_styled_cells() {
        let ansi = "\x1b[31mred\x1b[0m normal \x1b[32;1mbold green\x1b[0m";
        let lines = parse_ansi_lines(ansi);
        assert_eq!(lines.len(), 1);
        let cells = &lines[0];

        // "red" (3 chars) should be fg Red.
        for c in &cells[0..3] {
            assert_eq!(c.style.fg, Some(Color::Red), "red prefix not styled red");
        }
        // " normal " (8 chars) should be default/unstyled after the reset.
        // ansi-to-tui represents `\x1b[0m` as `Some(Color::Reset)` (a
        // sentinel), not `None` — a real spike finding: the parser emits a
        // Reset color rather than reverting to unset, so "no base style"
        // means fg is None *or* Some(Color::Reset).
        let is_default = |c: &StyledChar| matches!(c.style.fg, None | Some(Color::Reset));
        for c in &cells[3..11] {
            assert!(
                is_default(c),
                "text after reset should be default, got {:?}",
                c.style.fg
            );
        }
        // "bold green" (10 chars) should be fg Green + BOLD.
        for c in &cells[11..] {
            assert_eq!(
                c.style.fg,
                Some(Color::Green),
                "green suffix not styled green"
            );
            assert!(
                c.style.add_modifier.contains(Modifier::BOLD),
                "green suffix should be bold"
            );
        }
    }

    /// Multi-line ANSI with state carried across a line boundary (no reset
    /// before the newline) — the whole-block parse should keep the style.
    #[test]
    fn carries_style_across_newline() {
        let ansi = "\x1b[34mblue line one\nblue line two\x1b[0m";
        let lines = parse_ansi_lines(ansi);
        assert_eq!(lines.len(), 2);
        for line in &lines {
            for c in line {
                assert_eq!(
                    c.style.fg,
                    Some(Color::Blue),
                    "style should carry across newline"
                );
            }
        }
    }

    /// Plain text (no escapes) parses to unstyled cells, same char count.
    #[test]
    fn plain_text_is_unstyled() {
        let lines = parse_ansi_lines("hello world");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 11);
        for c in &lines[0] {
            assert_eq!(c.style, Style::default());
        }
    }

    // ── Word motion tests (Phase 3) ──────────────────────────────────────────
    //
    // These exercise the text-ops-read-ch contract from the ANSI-color
    // data model: motions run on plain-text fixtures and must land on the
    // right char regardless of styling. `motion_ignores_style` confirms
    // the `ch` field is the only input.

    fn state_from_text(text: &str) -> State {
        let mut state = State {
            lines: parse_ansi_lines(text),
            ..State::default()
        };
        if state.lines.is_empty() {
            state.lines.push(Vec::new());
        }
        state.content_rows = state.lines.len().max(1);
        state.content_cols = state
            .lines
            .iter()
            .map(|l| l.len())
            .max()
            .unwrap_or(0)
            .max(1);
        state
    }

    // Fixture: "foo bar.baz" — cols: f0 o1 o2 sp3 b4 a5 r6 .7 b8 a9 z10
    // word classes: Word(0-2) Space(3) Word(4-6) Other(7) Word(8-10)

    #[test]
    fn motion_w_lands_on_next_word_start() {
        let mut s = state_from_text("foo bar.baz");
        s.cursor = (0, 0);
        s.motion_w(false);
        assert_eq!(s.cursor, (0, 4));
        s.motion_w(false);
        assert_eq!(s.cursor, (0, 7));
        s.motion_w(false);
        assert_eq!(s.cursor, (0, 8));
    }

    #[test]
    fn motion_big_w_uses_word_semantics() {
        // WORD = non-whitespace run, so bar.baz is one WORD.
        let mut s = state_from_text("foo bar.baz");
        s.cursor = (0, 0);
        s.motion_w(true);
        assert_eq!(s.cursor, (0, 4));
        s.motion_w(true);
        // W from col 4 skips bar.baz (one WORD) and lands at EOL (col 11).
        assert_eq!(s.cursor, (0, 11));
    }

    #[test]
    fn motion_b_moves_backward_to_word_start() {
        let mut s = state_from_text("foo bar.baz");
        s.cursor = (0, 8);
        s.motion_b(false);
        assert_eq!(s.cursor, (0, 7));
        s.motion_b(false);
        assert_eq!(s.cursor, (0, 4));
        s.motion_b(false);
        assert_eq!(s.cursor, (0, 0));
    }

    #[test]
    fn motion_e_moves_to_word_end() {
        let mut s = state_from_text("foo bar.baz");
        s.cursor = (0, 0);
        s.motion_e(false);
        assert_eq!(s.cursor, (0, 2));
        s.motion_e(false);
        assert_eq!(s.cursor, (0, 6));
        s.motion_e(false);
        assert_eq!(s.cursor, (0, 7));
        s.motion_e(false);
        assert_eq!(s.cursor, (0, 10));
    }

    #[test]
    fn motion_wraps_across_lines() {
        let mut s = state_from_text("foo bar\nbaz qux");
        s.cursor = (0, 6); // on r, last char of line 0
        s.motion_w(false);
        assert_eq!(s.cursor, (1, 0));
        // b from (1,0) wraps back and lands on the START of bar (col 4),
        // not the end — b goes to word start, not word end.
        s.cursor = (1, 0);
        s.motion_b(false);
        assert_eq!(s.cursor, (0, 4));
    }

    #[test]
    fn motion_line_start_and_end() {
        let mut s = state_from_text("hello world");
        s.cursor = (0, 5);
        s.motion_line_end();
        assert_eq!(s.cursor, (0, 10));
        s.motion_line_start();
        assert_eq!(s.cursor, (0, 0));
    }

    #[test]
    fn motion_on_empty_line_clamps() {
        let mut s = state_from_text("\n\n");
        s.cursor = (1, 0);
        s.motion_line_end();
        assert_eq!(s.cursor, (1, 0));
        s.motion_w(false);
        assert!(s.cursor.0 >= 1);
    }

    /// Motions read ch, never Style — a styled fixture must produce the
    /// same cursor movement as its plain-text equivalent.
    #[test]
    fn motion_ignores_style() {
        let mut plain = state_from_text("foo bar.baz");
        let mut styled = state_from_text("\x1b[31mfoo\x1b[0m \x1b[1;32mbar.baz\x1b[0m");
        // Same underlying chars.
        assert_eq!(plain.lines[0].len(), styled.lines[0].len());
        plain.cursor = (0, 0);
        styled.cursor = (0, 0);
        plain.motion_w(false);
        styled.motion_w(false);
        assert_eq!(plain.cursor, styled.cursor);
        plain.motion_e(false);
        styled.motion_e(false);
        assert_eq!(plain.cursor, styled.cursor);
    }

    // ── Selection tests (Phase 4) ───────────────────────────────────────────

    #[test]
    fn selection_range_none_without_anchor() {
        let s = state_from_text("foo bar");
        assert!(s.selection_range().is_none());
        assert!(s.selection_info().is_none());
        assert!(s.selected_text().is_none());
    }

    #[test]
    fn selection_range_normalizes_to_stream_order() {
        let mut s = state_from_text("foo bar\nbaz qux");
        // anchor after cursor → range should normalize to (cursor, anchor).
        s.cursor = (0, 2);
        s.anchor = Some((1, 3));
        assert_eq!(s.selection_range(), Some(((0, 2), (1, 3))));
        // anchor before cursor → already in order.
        s.cursor = (1, 3);
        s.anchor = Some((0, 2));
        assert_eq!(s.selection_range(), Some(((0, 2), (1, 3))));
    }

    #[test]
    fn selection_info_single_line() {
        let mut s = state_from_text("hello world");
        s.cursor = (0, 0);
        s.anchor = Some((0, 4));
        assert_eq!(s.selection_info(), Some((1, 5))); // 1 line, 5 chars
    }

    #[test]
    fn selection_info_multi_line() {
        let mut s = state_from_text("foo bar\nbaz qux\nlast");
        s.cursor = (0, 4); // 'b' of bar
        s.anchor = Some((2, 2)); // 's' of last
        let (lines, chars) = s.selection_info().unwrap();
        assert_eq!(lines, 3);
        // first: line_len(0)=7 - 4 + 1(newline) = 4
        // mid:   line_len(1)=7 + 1 = 8
        // last:  2 + 1 = 3
        // total = 4 + 8 + 3 = 15
        assert_eq!(chars, 15);
    }

    #[test]
    fn selected_text_single_line() {
        let mut s = state_from_text("hello world");
        s.cursor = (0, 0);
        s.anchor = Some((0, 4));
        assert_eq!(s.selected_text().as_deref(), Some("hello"));
    }

    #[test]
    fn selected_text_multi_line() {
        let mut s = state_from_text("foo bar\nbaz qux\nlast");
        s.cursor = (0, 4); // 'b' of bar
        s.anchor = Some((2, 2)); // 's' of last
        assert_eq!(s.selected_text().as_deref(), Some("bar\nbaz qux\nlas"));
    }

    #[test]
    fn selected_text_strips_styles() {
        // The conversion contract: selected_text returns plain text, no
        // style bytes. A styled fixture must yield the same selected_text
        // as its plain-text equivalent.
        let mut plain = state_from_text("foo bar.baz");
        let mut styled = state_from_text("\x1b[31mfoo\x1b[0m \x1b[1;32mbar.baz\x1b[0m");
        plain.cursor = (0, 0);
        plain.anchor = Some((0, 6));
        styled.cursor = (0, 0);
        styled.anchor = Some((0, 6));
        assert_eq!(plain.selected_text(), styled.selected_text());
        assert_eq!(styled.selected_text().as_deref(), Some("foo bar"));
    }

    #[test]
    fn selection_extends_with_cursor_move() {
        // While anchor is set, moving the cursor extends the selection.
        let mut s = state_from_text("hello world");
        s.cursor = (0, 0);
        s.anchor = Some((0, 0));
        s.move_right(); // cursor now at 1
        assert_eq!(s.selection_range(), Some(((0, 0), (0, 1))));
        s.motion_w(false); // cursor jumps to 'w' of world at col 6
        assert_eq!(s.selection_range(), Some(((0, 0), (0, 6))));
    }
}
