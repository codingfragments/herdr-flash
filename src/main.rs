//! `herdr-flash` — Phase 2: scrollback view with relative line numbers,
//! cursor, footer, arrow-key + half-page navigation.
//!
//! The popup opens a real PTY (Herdr popup placement). This binary reads
//! the source pane's scrollback via `pane.read`, renders it with `ratatui`
//! driving a `crossterm` backend directly, and runs an event loop until
//! `Esc` closes the popup.

mod render;
mod socket_client;

use std::io;

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
/// `source = "recent_unwrapped"`.
fn read_scrollback(socket_path: &str, pane_id: &str) -> Result<String, String> {
    let params = serde_json::json!({
        "pane_id": pane_id,
        "source": "recent_unwrapped",
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

// ── Mode ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Mode {
    Normal,
    // Jump, LineJump, Search, Confirm arrive in Phases 5–8.
}

// ── State ────────────────────────────────────────────────────────────────────

struct State {
    lines: Vec<String>,
    cursor: (usize, usize),
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
        self.lines.get(line).map(|l| l.chars().count()).unwrap_or(0)
    }

    fn move_up(&mut self) {
        if self.cursor.0 == 0 {
            return;
        }
        self.cursor.0 -= 1;
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
        self.scroll_cursor_into_view();
    }

    fn move_down(&mut self) {
        if self.cursor.0 + 1 >= self.lines.len() {
            return;
        }
        self.cursor.0 += 1;
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
        self.scroll_cursor_into_view();
    }

    fn move_left(&mut self) {
        if self.cursor.1 > 0 {
            self.cursor.1 -= 1;
            self.scroll_x_into_view();
        } else if self.cursor.0 > 0 {
            self.cursor.0 -= 1;
            self.cursor.1 = self.line_len(self.cursor.0);
            self.scroll_cursor_into_view();
        }
    }

    fn move_right(&mut self) {
        let len = self.line_len(self.cursor.0);
        if self.cursor.1 < len {
            self.cursor.1 += 1;
            self.scroll_x_into_view();
        } else if self.cursor.0 + 1 < self.lines.len() {
            self.cursor.0 += 1;
            self.cursor.1 = 0;
            self.scroll_cursor_into_view();
        }
    }

    fn page_up(&mut self) {
        let half = (self.content_rows / 2).max(1);
        self.cursor.0 = self.cursor.0.saturating_sub(half);
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
        self.recenter_scroll();
    }

    fn page_down(&mut self) {
        let half = (self.content_rows / 2).max(1);
        let last = self.lines.len().saturating_sub(1);
        self.cursor.0 = (self.cursor.0 + half).min(last);
        self.cursor.1 = self.cursor.1.min(self.line_len(self.cursor.0));
        self.recenter_scroll();
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

        let content_lines: Vec<Line<'static>> = visible
            .iter()
            .enumerate()
            .map(|(i, text)| {
                let abs = scroll_y + i;
                let is_cursor_line = abs == cursor_line;
                let dist = (abs as isize - cursor_line as isize).unsigned_abs();

                let (gutter_str, gutter_style) = (
                    format!(
                        "{:>w$}{}",
                        dist,
                        if is_cursor_line { "► " } else { "  " },
                        w = num_w
                    ),
                    if is_cursor_line {
                        gutter_cursor_style
                    } else {
                        gutter_dim
                    },
                );
                let gutter = Span::styled(gutter_str, gutter_style);

                let scroll_x = self.scroll_x;
                let logical_len = text.chars().count();
                let has_right_overflow = logical_len > scroll_x + avail_w;
                let has_left_overflow = scroll_x > 0;

                let visible_w = if has_right_overflow {
                    avail_w.saturating_sub(1)
                } else {
                    avail_w
                };
                let chars: Vec<char> = text.chars().skip(scroll_x).take(visible_w).collect();

                let cur_col = if is_cursor_line {
                    Some(cursor_col.saturating_sub(scroll_x))
                } else {
                    None
                };

                let mut spans = vec![gutter];
                if has_left_overflow {
                    spans.push(Span::styled(
                        "…",
                        Style::default().fg(self.theme.footer_dim),
                    ));
                }
                spans.extend(render::build_line_spans(&chars, cur_col, &self.theme));
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

        // Status line: profile label, line count, cursor pos.
        let line1_spans = vec![
            Span::raw(" "),
            Span::styled("[scrollback]", dim),
            Span::raw("  "),
            Span::styled(format!("{} lines", self.lines.len()), dim),
            Span::raw("  "),
            Span::styled(pos_str, dim),
        ];
        let line1 = Line::from(line1_spans);

        // Key-hint line (Phase 2: only basic nav).
        let mut line2_spans = vec![
            Span::raw(" "),
            Span::styled("↑↓←→", bold),
            Span::raw(":move  "),
            Span::styled("PgUp/PgDn", bold),
            Span::raw(":half-page  "),
            Span::styled("Shift-←/→", bold),
            Span::raw(":pan  "),
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
        let line2 = Line::from(line2_spans);

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
    loop {
        terminal.draw(|f| state.render_all(f.area(), f.buffer_mut()))?;
        // Content area = full height minus the 4-row footer
        // (Constraint::Length(4) in render_all). The scroll math uses
        // content_rows, so it must match the actual render area — not the
        // full terminal height — or the cursor ends up below the visible
        // content at the buffer end.
        state.content_rows = terminal.size()?.height.saturating_sub(4) as usize;
        state.content_cols = terminal.size()?.width as usize;

        // Clamp scroll_y now that we have the real viewport height.
        let max_scroll = state.lines.len().saturating_sub(state.content_rows);
        state.scroll_y = state.scroll_y.min(max_scroll);

        let CtEvent::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        // Any keypress clears the transient message.
        state.message = None;

        let only_shift = key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc => break,
            KeyCode::Up => state.move_up(),
            KeyCode::Down => state.move_down(),
            KeyCode::Left if only_shift => {
                state.scroll_x = state.scroll_x.saturating_sub(5);
            }
            KeyCode::Right if only_shift => {
                let max_x = state
                    .lines
                    .iter()
                    .map(|l| l.chars().count())
                    .max()
                    .unwrap_or(0)
                    .saturating_sub(state.avail_w().saturating_sub(1));
                state.scroll_x = (state.scroll_x + 5).min(max_x);
            }
            KeyCode::Left => state.move_left(),
            KeyCode::Right => state.move_right(),
            KeyCode::PageUp => state.page_up(),
            KeyCode::PageDown => state.page_down(),
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
            lines: text.lines().map(String::from).collect(),
            ..State::default()
        };
        if state.lines.is_empty() {
            state.lines.push(String::new());
        }

        // Start at the bottom of the captured text, matching the original.
        let last = state.lines.len().saturating_sub(1);
        state.cursor = (last, 0);
        state.scroll_y = last.saturating_sub(state.content_rows.saturating_sub(1));

        run(&mut state).map_err(|e| format!("terminal error: {e}"))
    })() {
        eprintln!("herdr-flash error: {message}");
    }
}
