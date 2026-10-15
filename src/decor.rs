//! `decor` — decoration detection for the copy/insert action boundary.
//!
//! Scrollback content is frequently *decorated*: terminal UIs wrap code
//! blocks in box frames (pi's rounded `╭· fish ···╮` blocks, chat TUIs,
//! spinner frames), and common tools prefix every line with a
//! line-number gutter (`bat`'s `  1 │ `, `cat -n`, `grep -n`/`rg`'s
//! `12:` / `file.rs:12:`). A selection that spans such content copies
//! the decoration verbatim, which is almost never what the user wants
//! — they wanted the *text*, not the frame around it.
//!
//! This module is deliberately pure: it takes the exact lines a
//! selection will copy (stream-extracted or block-extracted — the same
//! text `selected_text()` produces) and reports, per line, which
//! character columns are decoration. No UI, no `State` dependency, so
//! everything here is unit-testable and the render/actions integration
//! (live dimming, footer hint, Enter-copy, `p` raw-copy) stays thin.
//!
//! Design decisions (grilled & settled, see PLANNING.md §13 feature
//! note):
//! - **Detection is per-selection**: only characters inside the selected
//!   range are ever considered decoration. Selecting one line of a box
//!   never rewrites other lines.
//! - **Boxes**: a full frame (top rule row … bottom rule row, side bars
//!   on every content line) or, failing that, side bars only (the same
//!   Unicode bar char starting every non-empty line). ASCII frames
//!   (`+ - |`) are **excluded** — they collide with markdown tables and
//!   ASCII art, which are content far more often than frames.
//! - **Gutters**: an optional identical file prefix (`rg` multi-file),
//!   spaces, a number, and a separator (`:`, `|`, `│`, tab for
//!   `cat -n`), with the counter **strictly increasing** across the
//!   non-empty selected lines. The monotonic requirement is what keeps
//!   real code lines that happen to start with `12:` from being
//!   misread as a gutter.
//! - **Strip is conservative**: rule rows drop, bar + exactly one
//!   padding space + trailing spaces/right bar drop, gutter prefix
//!   drops. Content indentation is *kept* (no de-indent — that's a
//!   recorded follow-up idea, not v1).
//! - Escape hatch: `p` copies the selection verbatim (see main.rs),
//!   so any detection mistake is always recoverable.

// Wired in from the live-feedback/actions integration steps; kept
// `pub` + module-level allow so the detector can land and be tested
// on its own first.
#![allow(dead_code)]

// ── Decoration character sets ─────────────────────────────────────────────────
//
// Box styles seen in the wild (rounded = pi/AI CLIs, light/heavy/double
// = TUI frameworks, dashed = spinner frames). We intentionally do NOT
// build style families (corner ↔ bar ↔ rule correspondences): the
// requirements below are strong enough together (a rule row *and*
// matching bars on *every* content line) that checking each glyph
// against the union sets is sufficient and much harder to get wrong.

/// Top-row left corners.
const TOP_LEFT: [char; 5] = ['╭', '┌', '┏', '╔', '╒'];
/// Top-row right corners.
const TOP_RIGHT: [char; 5] = ['╮', '┐', '┓', '╗', '╕'];
/// Bottom-row left corners.
const BOTTOM_LEFT: [char; 5] = ['╰', '└', '┗', '╚', '╘'];
/// Bottom-row right corners.
const BOTTOM_RIGHT: [char; 5] = ['╯', '┘', '┛', '╝', '╛'];
/// Vertical bar glyphs (rounded/light `│`, heavy `┃`, double `║`, dashed `╎`).
const BARS: [char; 4] = ['│', '┃', '║', '╎'];
/// Horizontal rule glyphs (light `─`, heavy `━`, double `═`, dashed `╌`).
const H_RULES: [char; 4] = ['─', '━', '═', '╌'];
/// Filler glyphs used inside rule rows (pi pads frames with `·`).
const FILLERS: [char; 1] = ['·'];

// ── Public API ────────────────────────────────────────────────────────────────

/// Per-line decoration verdict for one analyzed selection.
///
/// All column counts are in characters (not bytes) and refer to the
/// line **as it will be copied** (post stream/block extraction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineDecor {
    /// The line is a box rule row (top/bottom frame line) — dropped
    /// entirely by [`strip`].
    pub rule_row: bool,
    /// Decoration columns at the start of the line: uniform frame
    /// margin + box bar + one padding space + gutter prefix.
    pub prefix_cols: usize,
    /// Decoration columns at the end of the line: trailing spaces +
    /// right box bar (only when a right bar is actually present).
    pub suffix_cols: usize,
}

/// What [`analyze`] found in a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecorAnalysis {
    /// Parallel to the input lines.
    pub lines: Vec<LineDecor>,
    /// An enclosing box frame (full frame or side bars) was detected.
    pub has_box: bool,
    /// A line-number gutter (possibly *inside* a detected box) was
    /// detected.
    pub has_gutter: bool,
}

impl DecorAnalysis {
    /// Short footer label naming what will be stripped.
    pub fn label(&self) -> &'static str {
        match (self.has_box, self.has_gutter) {
            (true, true) => "box+numbers",
            (true, false) => "box",
            (false, true) => "numbers",
            (false, false) => "",
        }
    }

    /// How many lines survive [`strip`] (non-rule-row lines).
    pub fn stripped_line_count(&self) -> usize {
        self.lines.iter().filter(|d| !d.rule_row).count()
    }
}

/// Analyze the lines a selection will copy and detect decoration.
///
/// Input is exactly the selection's extracted text, split on `\n` —
/// the same lines `selected_text()`/`selected_text_block()` yield, so
/// detection and stripping can never disagree about what's selected.
/// Returns `None` when nothing decoration-like was found with high
/// confidence (the selection should then be copied verbatim).
pub fn analyze(lines: &[&str]) -> Option<DecorAnalysis> {
    let n = lines.len();
    if n < 2 {
        return None; // No decoration pattern is meaningful on one line.
    }

    // Uniform leading margin shared by every non-empty line (boxes are
    // frequently indented in scrollback, e.g. inside a padded message).
    let indent = common_indent(lines);

    // ── Box detection ────────────────────────────────────────────────────────
    // A frame is recognized from *fragments*: rule rows can appear anywhere
    // in the range (a selection may clip the top or bottom of a box), and
    // the content lines are the rest. Requires every non-empty content
    // line to open with the same Unicode bar. Without any rule row in
    // range, ≥2 non-empty content lines are needed (a lone framed line is
    // too little evidence); a rule row anywhere is frame context enough
    // for a single content line.
    let rule_rows: Vec<bool> = lines
        .iter()
        .map(|l| {
            is_rule_row(skip_indent(l, indent), Side::Top)
                || is_rule_row(skip_indent(l, indent), Side::Bottom)
        })
        .collect();
    let content: Vec<&str> = lines
        .iter()
        .zip(&rule_rows)
        .filter(|(_, rule)| !**rule)
        .map(|(l, _)| skip_indent(l, indent))
        .collect();
    let bar = common_bar(&content);
    let has_box = match bar {
        Some(_) => {
            let nonempty = content.iter().filter(|l| !l.trim().is_empty()).count();
            nonempty >= 1 && (nonempty >= 2 || rule_rows.iter().any(|r| *r))
        }
        None => false,
    };
    let box_lines: Vec<usize> = (0..n).filter(|i| !rule_rows[*i]).collect();

    // ── Per-line box prefix/suffix ───────────────────────────────────────────
    // prefix = margin + bar + one padding space (pi-style frames pad the
    // content by exactly one space after the bar; deeper indentation is
    // content and is kept). suffix = trailing spaces + right bar, and only
    // when the right bar is actually present (content longer than the
    // frame has no right bar and keeps everything after the prefix).
    let mut prefix: Vec<usize> = vec![0; n];
    let mut suffix: Vec<usize> = vec![0; n];
    if has_box {
        let bar = bar.expect("has_box implies bar");
        for &i in &box_lines {
            let chars: Vec<char> = lines[i].chars().collect();
            // Margin + bar.
            let mut p = indent + 1;
            // One padding space after the bar, if present.
            if chars.get(p) == Some(&' ') {
                p += 1;
            }
            prefix[i] = p;
            // Right bar: last non-whitespace char is the bar → consume
            // it plus the frame-padding whitespace run before it.
            if let Some(pos) = chars.iter().rposition(|c| !c.is_whitespace()) {
                if pos >= p && chars[pos] == bar {
                    let mut s = pos;
                    while s > p && chars[s - 1].is_whitespace() {
                        s -= 1;
                    }
                    suffix[i] = chars.len() - s;
                }
            }
        }
    }

    // ── Gutter detection ─────────────────────────────────────────────────────
    // Run on the *remainder* after box decoration so boxed+numbered
    // content (a pi block with bat-style line numbers inside) strips
    // both. Without a box, the remainder is the whole line.
    let mut has_gutter = false;
    let items: Vec<(usize, String)> = if has_box {
        box_lines
            .iter()
            .map(|&i| (i, remainder(lines[i], prefix[i], suffix[i])))
            .collect()
    } else {
        // Without a box, every line is a candidate (no rule rows were
        // found — that's why there's no box).
        (0..n).map(|i| (i, lines[i].to_string())).collect()
    };
    if let Some(lens) = detect_gutter(&items) {
        has_gutter = true;
        for (i, len) in lens {
            prefix[i] += len;
        }
    }

    if !has_box && !has_gutter {
        return None;
    }

    Some(DecorAnalysis {
        lines: (0..n)
            .map(|i| LineDecor {
                rule_row: rule_rows[i],
                prefix_cols: prefix[i],
                suffix_cols: suffix[i],
            })
            .collect(),
        has_box,
        has_gutter,
    })
}

/// Apply an analysis to the same lines it was computed from: drop rule
/// rows, remove per-line decoration prefix/suffix columns, join with
/// `\n`. Content indentation inside the frame is preserved.
pub fn strip(analysis: &DecorAnalysis, lines: &[&str]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (line, d) in lines.iter().zip(&analysis.lines) {
        if d.rule_row {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let start = d.prefix_cols.min(chars.len());
        let end = (chars.len().saturating_sub(d.suffix_cols)).max(start);
        out.push(chars[start..end].iter().collect());
    }
    out.join("\n")
}

// ── Internals ─────────────────────────────────────────────────────────────────

/// Which frame edge a rule row is checked against.
enum Side {
    Top,
    Bottom,
}

/// Smallest count of leading spaces shared by every non-empty line.
/// (Only spaces count — tabs are content as far as margins go.)
fn common_indent(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|&c| c == ' ').count())
        .min()
        .unwrap_or(0)
}

/// Drop `indent` leading spaces (safe: `indent` counts ASCII spaces).
fn skip_indent(line: &str, indent: usize) -> &str {
    let actual = line.chars().take_while(|&c| c == ' ').count();
    &line[actual.min(indent)..]
}

/// Rule-row check: opens with a corner of the given edge, closes with
/// the matching corner (after trailing spaces), is at least 3 chars,
/// and contains at least one rule/filler glyph between the corners —
/// the title text in `╭· fish ···╮` sits *between* fillers, so the
/// middle may contain arbitrary title characters.
fn is_rule_row(line: &str, side: Side) -> bool {
    let t = line.trim_end();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() < 3 {
        return false;
    }
    let (lefts, rights) = match side {
        Side::Top => (&TOP_LEFT[..], &TOP_RIGHT[..]),
        Side::Bottom => (&BOTTOM_LEFT[..], &BOTTOM_RIGHT[..]),
    };
    if !lefts.contains(&chars[0]) || !rights.contains(&chars[chars.len() - 1]) {
        return false;
    }
    chars[1..chars.len() - 1]
        .iter()
        .any(|c| H_RULES.contains(c) || FILLERS.contains(c))
}

/// The bar char every non-empty content line opens with, if they all
/// open with the same Unicode bar. `None` on empty input, a non-bar
/// first char, or mixed bars.
fn common_bar(content: &[&str]) -> Option<char> {
    let nonempty: Vec<&str> = content
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if nonempty.is_empty() {
        return None;
    }
    let bar = nonempty[0].chars().next()?;
    if !BARS.contains(&bar) {
        return None;
    }
    if nonempty.iter().all(|l| l.starts_with(bar)) {
        Some(bar)
    } else {
        None
    }
}

/// Text between the decoration columns of a line.
fn remainder(line: &str, prefix: usize, suffix: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let start = prefix.min(chars.len());
    let end = (chars.len().saturating_sub(suffix)).max(start);
    chars[start..end].iter().collect()
}

/// Parsed numeric gutter prefix of a single line: `[spaces][digits]`
/// then an optional single space, then a separator (`:` `|` `│` or tab
/// for `cat -n`), then an optional single space/tab. Returns the total
/// gutter length in chars plus the parsed counter value.
fn parse_gutter_simple(s: &str) -> Option<(usize, u64)> {
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() && chars[i] == ' ' {
        i += 1;
    }
    let num_start = i;
    while i < chars.len() && chars[i].is_ascii_digit() {
        i += 1;
    }
    if i == num_start {
        return None; // No number at the start.
    }
    let num: u64 = chars[num_start..i]
        .iter()
        .collect::<String>()
        .parse()
        .ok()?;
    let mut j = i;
    // bat-style: one space between number and `│`.
    if j < chars.len() && chars[j] == ' ' {
        j += 1;
    }
    if j < chars.len() && matches!(chars[j], ':' | '|' | '│' | '\t') {
        j += 1;
    } else {
        return None;
    }
    // The single space after bat's `│` separator is gutter padding — but
    // only there. After `:`/`|` (grep/rg) a following space belongs to the
    // content (often code indentation) and must survive the strip.
    if j < chars.len() && chars[j - 1] == '│' && (chars[j] == ' ' || chars[j] == '\t') {
        j += 1;
    }
    Some((j, num))
}

fn strictly_increasing(nums: &[u64]) -> bool {
    nums.windows(2).all(|w| w[0] < w[1])
}

/// Detect a line-number gutter across `(line index, text)` items.
///
/// Two passes, most common first:
/// 1. **Bare counter** — `  12 │ text` / `12:text` / `12\ttext`: every
///    non-empty item parses as `[spaces]digits[sep]`, counters strictly
///    increasing (`grep` may skip lines, so jumps are allowed; strict
///    increase is still required).
/// 2. **Identical file prefix** — `src/main.rs:12:text` (rg multi-file,
///    git grep): every non-empty item shares a literal prefix ending in
///    `:` (required to look like a path, i.e. contain `/` or `.` — this
///    is what keeps `host:8080:`-style output from being eaten), and
///    the remainder parses as pass 1.
///
/// Empty items are skipped (contribute gutter length 0). Returns
/// per-item gutter lengths, or `None` when no consistent gutter exists.
fn detect_gutter(items: &[(usize, String)]) -> Option<Vec<(usize, usize)>> {
    let nonempty: Vec<(usize, &str)> = items
        .iter()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(i, t)| (*i, t.as_str()))
        .collect();
    if nonempty.len() < 2 {
        return None;
    }

    // Pass 1: bare counter at line start.
    let parsed: Vec<Option<(usize, u64)>> = nonempty
        .iter()
        .map(|(_, t)| parse_gutter_simple(t))
        .collect();
    if parsed.iter().all(|p| p.is_some()) {
        let nums: Vec<u64> = parsed.iter().map(|p| p.unwrap().1).collect();
        if strictly_increasing(&nums) {
            return Some(
                items
                    .iter()
                    .map(|(i, t)| {
                        let len = if t.trim().is_empty() {
                            0
                        } else {
                            parse_gutter_simple(t).expect("just validated").0
                        };
                        (*i, len)
                    })
                    .collect(),
            );
        }
    }

    // Pass 2: identical literal prefix ending in ':' before the counter.
    // Candidates come from the first item; the longest candidate that
    // validates across every item wins.
    let first = nonempty[0].1;
    let mut best: Option<(String, usize)> = None; // (prefix, prefix char len)
    for (byte_idx, c) in first.char_indices() {
        if c != ':' {
            continue;
        }
        let prefix = &first[..byte_idx + 1]; // ':' is 1 byte — safe cut.
        if !prefix.contains('/') && !prefix.contains('.') {
            continue; // Must look like a path, not `name:port`.
        }
        let prefix_len = prefix.chars().count();
        if nonempty.iter().all(|(_, t)| {
            t.starts_with(prefix) && parse_gutter_simple(&t[prefix.len()..]).is_some()
        }) {
            let nums: Vec<u64> = nonempty
                .iter()
                .map(|(_, t)| parse_gutter_simple(&t[prefix.len()..]).unwrap().1)
                .collect();
            if strictly_increasing(&nums) {
                let p = prefix.to_string();
                if best
                    .as_ref()
                    .is_none_or(|(b, _)| b.chars().count() < prefix_len)
                {
                    best = Some((p, prefix_len));
                }
            }
        }
    }
    if let Some((prefix, prefix_len)) = best {
        return Some(
            items
                .iter()
                .map(|(i, t)| {
                    let len = if t.trim().is_empty() {
                        0
                    } else {
                        prefix_len + parse_gutter_simple(&t[prefix.len()..]).unwrap().0
                    };
                    (*i, len)
                })
                .collect(),
        );
    }

    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// The fixtures below are the coverage contract for v1: each `covered_*`
// test is a real-world decoration shape that must strip; each
// `not_covered_*` test is a look-alike that must be left verbatim.

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Option<String> {
        let lines: Vec<&str> = text.split('\n').collect();
        let a = analyze(&lines)?;
        Some(strip(&a, &lines))
    }

    // ── COVERED: boxes ───────────────────────────────────────────────────────

    /// Partial frame: the selection clips the box — top rule row and
    /// content, but not the bottom. Fragment detection still fires:
    /// rule rows drop, bars strip.
    #[test]
    fn covered_partial_frame_top_rule_only() {
        let text = "\
╭────────╮
│ clipped │
│ frame   │";
        assert_eq!(run(text).as_deref(), Some("clipped\nframe"));

        let text = "\
│ clipped │
│ frame   │
╰────────╯";
        assert_eq!(run(text).as_deref(), Some("clipped\nframe"));
    }

    /// The pi fish block from the wild — rounded corners, `·`-filler
    /// title row, `·`-filled bottom rule, `│` sides, one padding space,
    /// deeper content indentation kept, blank content line kept.
    #[test]
    fn covered_pi_fish_rounded_box_with_title() {
        let text = "\
╭· fish ·····································╮
│   cd ~/.dotfiles/dotSrc/app_coding_agents  │
│   git checkout feat/demo-s5                │
│                                            │
│   # 1. Temp workspace                      │
╰············································╯";
        let lines: Vec<&str> = text.split('\n').collect();
        let a = analyze(&lines).expect("fish box must be detected");
        assert!(a.has_box, "fish box: has_box");
        assert!(!a.has_gutter, "fish box has no numbers: no gutter");
        assert_eq!(a.label(), "box");
        assert_eq!(a.stripped_line_count(), 4);
        let stripped = strip(&a, &lines);
        assert_eq!(
            stripped,
            "  cd ~/.dotfiles/dotSrc/app_coding_agents\n  \
             git checkout feat/demo-s5\n\n  # 1. Temp workspace"
        );
    }

    /// The same block indented in the scrollback (rendered inside a
    /// padded context) — the uniform margin is decoration, the inner
    /// content indentation is kept.
    #[test]
    fn covered_indented_box_strips_margin() {
        // Vec, not a `\`-continued string: string continuations strip
        // leading whitespace and would eat the fixture's margin.
        let lines: Vec<&str> = vec!["   ╭──────────╮", "   │ hello    │", "   ╰──────────╯"];
        let a = analyze(&lines).expect("indented box must be detected");
        assert_eq!(strip(&a, &lines), "hello");
    }

    /// Light, heavy, double frames — different glyph sets, same shape.
    #[test]
    fn covered_light_heavy_double_frames() {
        let light = "\
┌────────┐
│ light  │
└────────┘";
        assert_eq!(run(light).as_deref(), Some("light"));

        let heavy = "\
┏━━━━━━┓
┃ heavy┃
┗━━━━━━┛";
        assert_eq!(run(heavy).as_deref(), Some("heavy"));

        let double = "\
╔═══════╗
║ double║
╚═══════╝";
        assert_eq!(run(double).as_deref(), Some("double"));
    }

    /// Content longer than the frame has no right bar — it keeps
    /// everything after the left bar + padding space.
    #[test]
    fn covered_box_with_overflowing_content_line() {
        let text = "\
╭──────╮
│ short│
│ this line is far longer than the frame and has no closing bar
╰──────╯";
        assert_eq!(
            run(text).as_deref(),
            Some("short\nthis line is far longer than the frame and has no closing bar")
        );
    }

    /// Selecting only the framed rows (no top/bottom rules in range)
    /// still strips the side bars when every non-empty line opens with
    /// the same Unicode bar.
    #[test]
    fn covered_side_bars_only_selection() {
        let text = "\
│ alpha │
│ beta  │
│ gamma │";
        assert_eq!(run(text).as_deref(), Some("alpha\nbeta\ngamma"));
    }

    // ── COVERED: line-number gutters ─────────────────────────────────────────

    /// bat-style right-aligned gutter with `│` separator.
    #[test]
    fn covered_bat_gutter() {
        let text = "\
   1 │ cd ~/somewhere
   2 │ git status
  10 │ cargo build";
        assert_eq!(
            run(text).as_deref(),
            Some("cd ~/somewhere\ngit status\ncargo build")
        );
    }

    /// `cat -n`: right-aligned number + tab (and the space-padded form
    /// terminals render tabs as).
    #[test]
    fn covered_cat_n_gutter() {
        let text = "\
     1\talpha
     2\tbeta";
        assert_eq!(run(text).as_deref(), Some("alpha\nbeta"));
    }

    /// `grep -n` / `rg` single-file: bare `N:` prefixes (jumps in the
    /// counter are fine — grep skips non-matching lines).
    #[test]
    fn covered_grep_n_gutter() {
        let text = "\
12:first match
13:second match
40:third match";
        assert_eq!(
            run(text).as_deref(),
            Some("first match\nsecond match\nthird match")
        );
    }

    /// `rg`/`git grep` multi-file form: identical path prefix + `N:`.
    #[test]
    fn covered_rg_multifile_gutter() {
        let text = "\
src/main.rs:12:fn main() {
src/main.rs:13:    println!(\"hi\");
src/main.rs:14:}";
        assert_eq!(
            run(text).as_deref(),
            Some("fn main() {\n    println!(\"hi\");\n}")
        );
    }

    // ── COVERED: combined ────────────────────────────────────────────────────

    /// A boxed block whose content is itself numbered (pi block with
    /// line numbers inside, boxed `bat` output) — both strip, in the
    /// right order.
    #[test]
    fn covered_boxed_numbered_content_strips_both() {
        let text = "\
╭─────────────────╮
│   1 │ cd ~/x    │
│   2 │ git pull  │
╰─────────────────╯";
        let lines: Vec<&str> = text.split('\n').collect();
        let a = analyze(&lines).expect("boxed+numbered must be detected");
        assert!(a.has_box && a.has_gutter);
        assert_eq!(a.label(), "box+numbers");
        assert_eq!(strip(&a, &lines), "cd ~/x\ngit pull");
    }

    // ── NOT COVERED: look-alikes stay verbatim ───────────────────────────────

    /// Markdown tables: ASCII `|` is never a box bar; ASCII frames are
    /// excluded on purpose.
    #[test]
    fn not_covered_markdown_table() {
        let text = "\
| key | value |
|-----|-------|
| a   | 1     |";
        assert!(run(text).is_none());
    }

    /// ASCII-art frame: `+--+`/`| |` — content, not decoration.
    #[test]
    fn not_covered_ascii_art_box() {
        let text = "\
+--------+
|  art   |
|   :)   |
+--------+";
        assert!(run(text).is_none());
    }

    /// Code lines that happen to start `N:` — not monotonic, so no
    /// gutter. (Sequence errors are the main code-vs-gutter tiebreaker.)
    #[test]
    fn not_covered_non_monotonic_numbered_code() {
        let text = "\
12: this is code
5: more code
99: more code";
        assert!(run(text).is_none());
    }

    /// Counter monotonic but separator missing (`12 foo`) or host:port
    /// shapes (`host:8080:`) — no path-like prefix, no strip.
    #[test]
    fn not_covered_host_port_and_plain_numbers() {
        let text = "\
host:8080:/a
host:8081:/b";
        assert!(run(text).is_none());

        let plain = "\
3 apples
4 pears";
        assert!(run(text).is_none());
        assert!(run(plain).is_none());
    }

    /// Mixed bar glyphs across lines — not one coherent frame.
    #[test]
    fn not_covered_mixed_bars() {
        let text = "\
│ one │
┃ two ┃";
        assert!(run(text).is_none());
    }

    /// A stream selection that starts mid-line (first copied line has
    /// no bar at its start) — box detection requires the frame, so the
    /// selection copies verbatim. Extend the selection to the whole
    /// box to get the clean copy.
    #[test]
    fn not_covered_partial_mid_line_selection() {
        let text = "\
cd ~/x │
│ two │";
        assert!(run(text).is_none());
    }

    /// Rule rows without content between them, or a lone framed line —
    /// too little evidence.
    #[test]
    fn not_covered_too_small_fragments() {
        assert!(run("╭────╮\n╰────╯").is_none());
        assert!(run("│ only one line │").is_none());
    }

    /// Mermaid/Unicode art can *legitimately* be a box — the detector
    /// will offer to strip it (it is one); `p` copies it verbatim.
    /// This test documents the known false-positive territory rather
    /// than fighting it.
    #[test]
    fn known_false_positive_mermaid_box_strips_use_p_for_raw() {
        let text = "\
╭───────╮
│ Start │
╰───┬───╯";
        assert_eq!(run(text).as_deref(), Some("Start"));
    }
}
