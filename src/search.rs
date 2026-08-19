//! Incremental search, ported from the original `zellij-flash`.
//!
//! Phase 7 scope: `/` enters search mode (only when no anchor is set).
//! The algorithm operates on the `ch` field of `StyledChar` cells
//! (plain-text extraction) — never on the `Style` — per the "Data model"
//! note in PLANNING.md §11. Highlight rendering of the matches happens in
//! `render::build_line_spans` using the replace overlay policy.

use crate::StyledChar;

/// Compute all case-insensitive substring matches for `query` across
/// **all captured lines** (not just visible). Returns `(line, col)` pairs
/// for each match start, in stream order. Empty query → no matches.
pub fn compute_search_matches(lines: &[Vec<StyledChar>], query: &str) -> Vec<(usize, usize)> {
    if query.is_empty() {
        return Vec::new();
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let qlen = q.len();
    let mut out = Vec::new();
    for (li, line) in lines.iter().enumerate() {
        let lc: Vec<char> = line.iter().map(|c| c.ch.to_ascii_lowercase()).collect();
        if lc.len() < qlen {
            continue;
        }
        for col in 0..=(lc.len() - qlen) {
            if lc[col..col + qlen] == q[..] {
                out.push((li, col));
            }
        }
    }
    out
}

/// Index of the first match at or after the cursor, wrapping to 0.
pub fn search_current_from_cursor(matches: &[(usize, usize)], cursor: (usize, usize)) -> usize {
    let (cl, cc) = cursor;
    matches
        .iter()
        .position(|&(ml, mc)| ml > cl || (ml == cl && mc >= cc))
        .unwrap_or(0)
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

    #[test]
    fn empty_query_no_matches() {
        let lines = styled_lines("foo bar");
        assert!(compute_search_matches(&lines, "").is_empty());
    }

    #[test]
    fn single_line_matches() {
        let lines = styled_lines("foo bar foo baz");
        let matches = compute_search_matches(&lines, "foo");
        assert_eq!(matches, vec![(0, 0), (0, 8)]);
    }

    #[test]
    fn multi_line_matches() {
        let lines = styled_lines("foo\nbar\nfoo bar");
        let matches = compute_search_matches(&lines, "foo");
        assert_eq!(matches, vec![(0, 0), (2, 0)]);
    }

    #[test]
    fn case_insensitive() {
        let lines = styled_lines("Foo BAR baz");
        let matches = compute_search_matches(&lines, "ba");
        assert_eq!(matches, vec![(0, 4), (0, 8)]); // BAR and baz
    }

    #[test]
    fn no_matches() {
        let lines = styled_lines("foo bar");
        let matches = compute_search_matches(&lines, "xyz");
        assert!(matches.is_empty());
    }

    #[test]
    fn current_from_cursor_at_or_after() {
        // matches at (0,0), (0,8), (2,0)
        let lines = styled_lines("foo bar foo\nbaz\nfoo bar");
        let matches = compute_search_matches(&lines, "foo");
        // cursor at (0,5) → first match at or after is (0,8)
        assert_eq!(search_current_from_cursor(&matches, (0, 5)), 1);
        // cursor at (1,0) → first match at or after is (2,0) = index 2
        assert_eq!(search_current_from_cursor(&matches, (1, 0)), 2);
        // cursor at (2,5) → no match after → wraps to 0
        assert_eq!(search_current_from_cursor(&matches, (2, 5)), 0);
    }

    #[test]
    fn matches_all_lines_not_just_visible() {
        // Search matches across ALL captured lines, not just the visible window.
        let lines = styled_lines("foo\nbar\nfoo\nbaz\nfoo");
        let matches = compute_search_matches(&lines, "foo");
        assert_eq!(matches, vec![(0, 0), (2, 0), (4, 0)]);
    }

    /// Search operates on plain chars, never on styles — a styled fixture
    /// must produce the same matches as its plain-text equivalent.
    #[test]
    fn search_ignores_style() {
        use crate::parse_ansi_lines;
        let plain = styled_lines("foo bar foo");
        let styled_text = "\x1b[31mfoo\x1b[0m bar \x1b[1;32mfoo\x1b[0m";
        let styled = parse_ansi_lines(styled_text);
        // Same underlying chars: "foo bar foo"
        assert_eq!(plain[0].len(), styled[0].len());
        let plain_matches = compute_search_matches(&plain, "foo");
        let styled_matches = compute_search_matches(&styled, "foo");
        assert_eq!(plain_matches, styled_matches);
        assert_eq!(styled_matches, vec![(0, 0), (0, 8)]);
    }
}
