//! Word-jump label algorithm, ported from the original `zellij-flash`.
//!
//! Phase 5 scope: `s` / `S` word-jump with nvim-flash-style labels. The
//! algorithm operates on the `ch` field of `StyledChar` cells (plain-text
//! extraction) — never on the `Style` — per the "Data model" note in
//! PLANNING.md §11. Rendering of the resulting labels/matches/partial
//! highlights happens in `render::build_line_spans` using the replace
//! overlay policy.
//!
//! The label charset is hardcoded to the 52-char `a-zA-Z` pool for now;
//! Phase 9 makes it config-driven.

use crate::StyledChar;

/// Default label charset (Phase 9 makes this config-driven).
pub const LABEL_CHARS: &[char] = &[
    'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's',
    't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L',
    'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
];

/// A labeled jump match: `(line, jump_col, label_col, label_char)`.
/// `jump_col` is where the cursor lands; `label_col` is where the label
/// glyph renders (last char of the prefix, clamped to the last real char).
pub type JumpLabel = (usize, usize, usize, char);

/// A partial (unlabeled) match: `(line, jump_col, label_col)`.
pub type PartialMatch = (usize, usize, usize);

/// Compute jump labels for the given typed prefix.
///
/// Returns `(labels, partial_matches)`:
/// - `labels = (line, jump_col, label_col, label_char)` — sorted by distance
///   from the cursor (nearest first). `jump_col` is the match start (where
///   the cursor lands); `label_col` is where the label glyph renders (last
///   char of the prefix, clamped to the last real character so EOL matches
///   don't render past the line end).
/// - `partial_matches = (line, jump_col, label_col)` — all matches when
///   there are too many to label; rendered as partial-highlight (no labels).
///
/// Algorithm (ported verbatim from the original, adapted only to read
/// `StyledChar.ch` instead of indexing a `String`):
/// 1. Case-insensitive substring match across **visible lines**, with
///    trailing-space virtual matching so `"foo "` matches `"foo"` at line end.
/// 2. Exclude typed chars from the label pool.
/// 3. Continuation-aware exclusion: a label char equal to a continuation
///    of 2+ matches is excluded entirely (typing it narrows, never jumps).
///    A label char equal to the unique continuation of exactly one match
///    is pre-assigned to that match.
/// 4. When non-pre-labeled matches exceed the pool, fall back to
///    partial-match highlighting (no labels).
pub fn compute_jump_labels(
    lines: &[Vec<StyledChar>],
    scroll_y: usize,
    content_rows: usize,
    cursor: (usize, usize),
    typed: &str,
    label_chars: &[char],
) -> (Vec<JumpLabel>, Vec<PartialMatch>) {
    if typed.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let typed_lower: Vec<char> = typed.to_lowercase().chars().collect();
    let tlen = typed_lower.len();
    // How many trailing spaces the user typed. Each line gets that many
    // virtual spaces appended so "x " also matches "x" at the line end.
    let trailing_spaces = typed_lower.iter().rev().take_while(|&&c| c == ' ').count();

    let vis_start = scroll_y;
    let vis_end = (scroll_y + content_rows).min(lines.len());

    let mut raw: Vec<PartialMatch> = Vec::new(); // (line, jump_col, label_col)
                                                 // Indexing lines by line_idx is intentional — we need the absolute line
                                                 // number for the match coordinates, not just the line content.
    #[allow(clippy::needless_range_loop)]
    for line_idx in vis_start..vis_end {
        // Extract plain chars from styled cells (lowercased for matching).
        let plain: Vec<char> = lines[line_idx].iter().map(|c| c.ch).collect();
        let mut chars_lower: Vec<char> = plain.iter().map(|c| c.to_ascii_lowercase()).collect();
        let orig_n = chars_lower.len();
        // Append virtual spaces so trailing-space patterns reach EOL.
        if orig_n > 0 {
            chars_lower.extend(std::iter::repeat_n(' ', trailing_spaces));
        }
        let n = chars_lower.len();
        if tlen > n {
            continue;
        }
        for col in 0..=(n - tlen) {
            if chars_lower[col..col + tlen] == typed_lower[..] {
                // Label sits on the last char of the prefix, clamped to
                // the last real character (never on a virtual space).
                let label_col = (col + tlen - 1).min(orig_n.saturating_sub(1));
                raw.push((line_idx, col, label_col));
            }
        }
    }

    // Dedup: a line ending with a real space could produce the same
    // (line, jump_col, label_col) from both the natural and virtual match.
    raw.sort_unstable();
    raw.dedup();

    if raw.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Build exclude set: all cases of typed characters.
    let exclude: std::collections::HashSet<char> = typed
        .chars()
        .flat_map(|c| [c.to_ascii_lowercase(), c.to_ascii_uppercase()])
        .collect();

    // Compute the next character (continuation) after the typed prefix for
    // each match. We use this to enforce two invariants:
    //   1. A label char that equals a continuation of 2+ matches is excluded
    //      entirely — typing it must narrow the search, never commit a jump.
    //   2. A label char that equals the unique continuation of exactly one
    //      match is pre-assigned to that specific match — it must not label
    //      any other position.
    let mut continuation_counts: std::collections::HashMap<char, usize> =
        std::collections::HashMap::new();
    // (line, jump_col) → lowercase continuation char
    let mut match_continuation: std::collections::HashMap<(usize, usize), char> =
        std::collections::HashMap::new();
    for &(line_idx, jump_col, _) in &raw {
        let line_chars: Vec<char> = lines[line_idx].iter().map(|c| c.ch).collect();
        if let Some(&ch) = line_chars.get(jump_col + tlen) {
            let c = ch.to_ascii_lowercase();
            *continuation_counts.entry(c).or_insert(0) += 1;
            match_continuation.insert((line_idx, jump_col), c);
        }
    }

    // Pool excludes typed chars AND every continuation char (both cases).
    // Continuations with count == 1 are pre-assigned directly; excluding
    // them from the pool prevents them from landing on a different match.
    let all_continuations: std::collections::HashSet<char> = continuation_counts
        .keys()
        .flat_map(|&c| [c.to_ascii_lowercase(), c.to_ascii_uppercase()])
        .collect();
    let pool: Vec<char> = label_chars
        .iter()
        .filter(|&&c| !exclude.contains(&c) && !all_continuations.contains(&c))
        .copied()
        .collect();

    // Separate matches into pre-labeled (unique continuation → that char is
    // the label) and to-label (no continuation or ambiguous → pool label).
    let mut pre_labeled: Vec<JumpLabel> = Vec::new();
    let mut to_label: Vec<PartialMatch> = Vec::new();
    for &(line_idx, jump_col, label_col) in &raw {
        let cont = match_continuation.get(&(line_idx, jump_col)).copied();
        let pre = cont.and_then(|c| {
            if *continuation_counts.get(&c).unwrap_or(&0) == 1 {
                // Prefer lowercase label; fall back to uppercase.
                if label_chars.contains(&c) && !exclude.contains(&c) {
                    Some(c)
                } else {
                    let cu = c.to_ascii_uppercase();
                    if label_chars.contains(&cu) && !exclude.contains(&cu) {
                        Some(cu)
                    } else {
                        None
                    }
                }
            } else {
                None
            }
        });
        match pre {
            Some(lc) => pre_labeled.push((line_idx, jump_col, label_col, lc)),
            None => to_label.push((line_idx, jump_col, label_col)),
        }
    }

    // Too many non-pre-labeled matches to cover with the pool: fall back to
    // partial-match highlighting (no labels at all).
    if to_label.len() > pool.len() {
        return (Vec::new(), raw);
    }

    // Sort non-pre-labeled by distance from cursor; assign pool labels.
    let (cline, ccol) = cursor;
    to_label.sort_by_key(|&(line, col, _)| {
        let dl = (line as isize - cline as isize).unsigned_abs();
        let dc = (col as isize - ccol as isize).unsigned_abs();
        dl * 10_000 + dc
    });
    let mut labels: Vec<JumpLabel> = pre_labeled;
    labels.extend(
        to_label
            .into_iter()
            .zip(pool)
            .map(|((line, jump_col, label_col), lc)| (line, jump_col, label_col, lc)),
    );
    (labels, Vec::new())
}

/// Compute line-jump labels for the visible viewport.
///
/// Returns `(line, label_char)` pairs — one per visible line except the
/// cursor line (which has no label). Default **directional** scheme:
/// `a`-`z` for lines below the cursor (`a` = nearest), `A`-`Z` for lines
/// above (`A` = nearest). The `unified` scheme (splitting the label
/// charset in half) is config-driven and lands in Phase 9; ship the
/// directional scheme now.
///
/// Unlike `compute_jump_labels`, this is instant — no prefix typing,
/// no partial fallback. Every visible line (except the cursor line) gets
/// a label the moment `l` is pressed.
pub fn compute_line_labels(
    lines: &[Vec<StyledChar>],
    scroll_y: usize,
    content_rows: usize,
    cursor: (usize, usize),
    label_chars: &[char],
    unified: bool,
) -> Vec<(usize, char)> {
    let vis_start = scroll_y;
    let vis_end = (scroll_y + content_rows).min(lines.len());
    let cline = cursor.0;

    let below = (vis_start..vis_end).filter(|&l| l > cline);
    let above = (vis_start..cline.min(vis_end)).rev();

    let mut labels: Vec<(usize, char)> = Vec::new();

    if unified {
        // Split label_chars in half: first half → below, second half → above.
        let n = label_chars.len();
        let mid = n.div_ceil(2); // first half gets the extra char if odd
        let below_pool = &label_chars[..mid];
        let above_pool = &label_chars[mid..];
        for (line, &lc) in below.zip(below_pool.iter()) {
            labels.push((line, lc));
        }
        for (line, &lc) in above.zip(above_pool.iter()) {
            labels.push((line, lc));
        }
    } else {
        // Directional scheme: a-z below (nearest = a), A-Z above (nearest = A).
        for (line, lc) in below.zip('a'..='z') {
            labels.push((line, lc));
        }
        for (line, lc) in above.zip('A'..='Z') {
            labels.push((line, lc));
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn styled_line(s: &str) -> Vec<StyledChar> {
        s.chars()
            .map(|ch| StyledChar {
                ch,
                style: Style::default(),
            })
            .collect()
    }

    fn styled_lines(text: &str) -> Vec<Vec<StyledChar>> {
        text.lines().map(styled_line).collect()
    }

    /// Empty prefix → no labels, no partials.
    #[test]
    fn empty_prefix_yields_nothing() {
        let lines = styled_lines("foo bar baz");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "", LABEL_CHARS);
        assert!(labels.is_empty());
        assert!(partials.is_empty());
    }

    /// A prefix with few matches → labels assigned, nearest first.
    #[test]
    fn few_matches_get_labeled() {
        // "foo bar baz" — matches for "ba" at cols 4 and 8.
        let lines = styled_lines("foo bar baz");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "ba", LABEL_CHARS);
        assert!(partials.is_empty(), "should not be partial");
        assert_eq!(labels.len(), 2, "two matches should get labels");
        // Nearest to cursor (0,0) is col 4 (bar), then col 8 (baz).
        assert_eq!(labels[0].1, 4, "nearest match first");
        assert_eq!(labels[1].1, 8);
        // Labels should be distinct chars from the pool.
        assert_ne!(labels[0].3, labels[1].3);
    }

    /// Too many matches → partial fallback (no labels, all matches returned).
    #[test]
    fn too_many_matches_fall_back_to_partial() {
        // 60 matches for "x" — exceeds the 52-char pool.
        let line = styled_line(&"x ".repeat(60));
        let lines = vec![line];
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "x", LABEL_CHARS);
        assert!(labels.is_empty(), "too many matches → no labels");
        assert_eq!(partials.len(), 60, "all matches in partial");
    }

    /// Typed chars are excluded from the label pool — typing one narrows.
    #[test]
    fn typed_chars_excluded_from_pool() {
        // "aba" — matches for "a" at cols 0 and 2. 'a' is typed, so it
        // can't be a label. The labels should be non-'a' pool chars.
        let lines = styled_lines("aba");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "a", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(labels.len(), 2);
        for &(_, _, _, lc) in &labels {
            assert_ne!(lc, 'a', "typed char must not be a label");
            assert_ne!(lc, 'A', "typed char (upper) must not be a label");
        }
    }

    /// Trailing-space virtual matching: "foo " matches "foo" at line end.
    #[test]
    fn trailing_space_matches_at_eol() {
        // "foo" at end of line — "foo " (with trailing space) should match.
        let lines = styled_lines("x foo");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "foo ", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(
            labels.len(),
            1,
            "trailing-space pattern should match at EOL"
        );
        assert_eq!(labels[0].1, 2, "match starts at col 2");
    }

    /// Case-insensitive matching.
    #[test]
    fn case_insensitive_matching() {
        let lines = styled_lines("Foo BAR baz");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "ba", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(labels.len(), 2, "BAR and baz both match 'ba'");
        // BAR at col 4, baz at col 8.
        let jump_cols: Vec<usize> = labels.iter().map(|&(_, jc, _, _)| jc).collect();
        assert!(jump_cols.contains(&4));
        assert!(jump_cols.contains(&8));
    }

    /// Matches only within visible lines (scroll window).
    #[test]
    fn only_visible_lines_matched() {
        let lines = styled_lines("foo\nbar\nbaz");
        // scroll_y=1, content_rows=1 → only line 1 ("bar") is visible.
        let (labels, partials) = compute_jump_labels(&lines, 1, 1, (1, 0), "ba", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(labels.len(), 1, "only the visible line matches");
        assert_eq!(labels[0].0, 1, "match is on visible line 1");
    }

    /// Unique continuation → pre-assigned label.
    #[test]
    fn unique_continuation_preassigned() {
        // "abc abd" — matches for "ab" at cols 0 and 4.
        // Continuations: 'c' (col 2, unique) and 'd' (col 6, unique).
        // Each should be pre-assigned as its own label.
        let lines = styled_lines("abc abd");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "ab", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(labels.len(), 2);
        let label_chars: Vec<char> = labels.iter().map(|&(_, _, _, lc)| lc).collect();
        assert!(
            label_chars.contains(&'c'),
            "unique continuation 'c' pre-assigned"
        );
        assert!(
            label_chars.contains(&'d'),
            "unique continuation 'd' pre-assigned"
        );
    }

    /// Ambiguous continuation → excluded from the pool (typing it narrows).
    #[test]
    fn ambiguous_continuation_excluded() {
        // "abx abx" — matches for "ab" at cols 0 and 4.
        // Continuation is 'x' for both (ambiguous, count=2) → 'x' excluded
        // from the pool. Labels must be non-'x' pool chars.
        let lines = styled_lines("abx abx");
        let (labels, partials) = compute_jump_labels(&lines, 0, 10, (0, 0), "ab", LABEL_CHARS);
        assert!(partials.is_empty());
        assert_eq!(labels.len(), 2);
        for &(_, _, _, lc) in &labels {
            assert_ne!(lc, 'x', "ambiguous continuation must not be a label");
            assert_ne!(lc, 'X');
        }
    }

    // ── Line-jump tests (Phase 6) ───────────────────────────────────────────

    #[test]
    fn line_jump_labels_all_visible_except_cursor() {
        // 5 lines, cursor on line 2. Lines 0,1 above; lines 3,4 below.
        let lines = styled_lines("a\nb\nc\nd\ne");
        let labels = compute_line_labels(&lines, 0, 10, (2, 0), LABEL_CHARS, false);
        // 4 labels (all except cursor line 2).
        assert_eq!(labels.len(), 4);
        // Below: lines 3,4 → a,b
        // Above (reversed): lines 1,0 → A,B
        let by_line: std::collections::HashMap<usize, char> = labels.iter().cloned().collect();
        assert_eq!(by_line[&3], 'a', "nearest below = a");
        assert_eq!(by_line[&4], 'b');
        assert_eq!(by_line[&1], 'A', "nearest above = A");
        assert_eq!(by_line[&0], 'B');
        assert!(!by_line.contains_key(&2), "cursor line has no label");
    }

    #[test]
    fn line_jump_cursor_at_top() {
        // Cursor on line 0 → all labels are below (a-z).
        let lines = styled_lines("a\nb\nc");
        let labels = compute_line_labels(&lines, 0, 10, (0, 0), LABEL_CHARS, false);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0], (1, 'a'));
        assert_eq!(labels[1], (2, 'b'));
    }

    #[test]
    fn line_jump_cursor_at_bottom() {
        // Cursor on last line → all labels are above (A-Z).
        let lines = styled_lines("a\nb\nc");
        let labels = compute_line_labels(&lines, 0, 10, (2, 0), LABEL_CHARS, false);
        assert_eq!(labels.len(), 2);
        // Above reversed: line 1 = A, line 0 = B
        assert_eq!(labels[0], (1, 'A'));
        assert_eq!(labels[1], (0, 'B'));
    }

    #[test]
    fn line_jump_respects_scroll_window() {
        // 5 lines, scroll_y=1, content_rows=2 → only lines 1,2 visible.
        let lines = styled_lines("a\nb\nc\nd\ne");
        let labels = compute_line_labels(&lines, 1, 2, (1, 0), LABEL_CHARS, false);
        // Only line 2 is visible and below cursor → one label 'a'.
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0], (2, 'a'));
    }

    #[test]
    fn line_jump_more_than_26_lines() {
        // 30 lines below cursor → only 26 get labels (a-z runs out).
        let text: String = (0..31).map(|i| format!("line{i}\n")).collect();
        let lines = styled_lines(text.trim_end());
        let labels = compute_line_labels(&lines, 0, 31, (0, 0), LABEL_CHARS, false);
        // 30 lines below, but only 26 letters → 26 labels.
        assert_eq!(labels.len(), 26);
        assert_eq!(labels[0], (1, 'a'));
        assert_eq!(labels[25], (26, 'z'));
    }
}
