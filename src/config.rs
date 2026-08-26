//! Config loader, adapted to Herdr's TOML model.
//!
//! Phase 9 scope: loads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` once per
//! launch. Three layers, mirroring the sister `herdr-zextract` port:
//!
//! - **Global** (top-level): `log_level`, `labels` (word-jump charset),
//!   `line_labels` (`directional`/`unified`), and the 16 `color_*` theme
//!   roles (hex `#rrggbb`, Catppuccin Macchiato defaults).
//! - **Per-keybind profiles** (`[profiles.<name>]`): `depths` — the
//!   scrollback-depth cycle list for `g`. Selected at launch by
//!   `FLASH_PROFILE=<name>`. A built-in `default` profile carries
//!   `["viewport", "200", "2000"]`.
//!
//! Missing/unset/parse-error → built-in defaults, never crash; parse errors
//! reported on stderr.

use crate::render::{parse_hex_color, Theme};

/// Initial-view scroll-follow mode: where to position the popup's
/// cursor/viewport when it opens, relative to the source pane's scroll
/// position.
///
/// - `Off` (default) — always open at the bottom of the captured text,
///   matching the original `zellij-flash` behavior. The source pane's
///   scroll state is ignored.
/// - `Offset` — read the source pane's `scroll.offset_from_bottom` (via
///   `pane.get`) and anchor the popup on the logical line that corresponds
///   to the source viewport's bottom screen-row. Exact when nothing in the
///   scrolled region wraps in the source; drifts upward when long lines
///   wrap (the popup renders one unwrapped line per row, the source
///   wraps — see PLANNING.md §11 "Data model").
/// - `Content` — additionally read the source's current viewport text
///   (`pane.read source=visible`) and locate it inside the capture by
///   fingerprint-matching distinctive short lines. Sidesteps the wrap
///   drift; falls back to `Offset` when no unique anchor is found. Most
///   faithful to "what's on screen", but content-dependent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollFollow {
    #[default]
    Off,
    Offset,
    Content,
}

/// Parse a `scroll_follow` string: "off"/"offset"/"content" (case-insensitive).
/// Unknown values fall back to `Off` (with a stderr warning at load time).
pub fn parse_scroll_follow(s: &str) -> Option<ScrollFollow> {
    match s.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "false" => Some(ScrollFollow::Off),
        "offset" => Some(ScrollFollow::Offset),
        "content" => Some(ScrollFollow::Content),
        _ => None,
    }
}

/// Scrollback depth for a single profile entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// `pane.read` with `source = "visible"` — just the viewport.
    /// Rarely useful (the whole point is to navigate scrollback), but
    /// available as a custom-profile option.
    Viewport,
    /// `pane.read` with `source = "recent_unwrapped"` + `lines = N`.
    Lines(u32),
    /// `pane.read` with `source = "recent_unwrapped"` and no `lines`
    /// cap — grabs everything the terminal has in its scrollback buffer.
    Unlimited,
}

impl Depth {
    /// Human-readable label for the footer (e.g. "200", "5000", "unlimited").
    pub fn label(&self) -> String {
        match self {
            Depth::Viewport => "viewport".to_string(),
            Depth::Lines(n) => n.to_string(),
            Depth::Unlimited => "unlimited".to_string(),
        }
    }
}

/// Parse a single depth string: "viewport" → Viewport, "N" → Lines(N).
fn parse_depth(s: &str) -> Option<Depth> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("viewport") {
        return Some(Depth::Viewport);
    }
    if s.eq_ignore_ascii_case("unlimited") || s.eq_ignore_ascii_case("all") {
        return Some(Depth::Unlimited);
    }
    s.parse::<u32>().ok().filter(|&n| n > 0).map(Depth::Lines)
}

/// A named profile with its depth cycle list.
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub depths: Vec<Depth>,
}

impl Profile {
    #[allow(dead_code)]
    pub fn label(&self) -> String {
        self.name.clone()
    }
}

/// Full runtime config.
#[derive(Debug, Clone)]
pub struct Config {
    pub log_level: String,
    /// Word-jump label charset (default: a-zA-Z, 52 chars).
    pub labels: Vec<char>,
    /// Line-jump scheme: false = directional (a-z below, A-Z above),
    /// true = unified (split `labels` in half).
    pub line_labels_unified: bool,
    /// Theme (16 color roles, Catppuccin Macchiato defaults).
    pub theme: Theme,
    /// Named profiles, selected at launch by `FLASH_PROFILE`.
    pub profiles: Vec<Profile>,
    /// Index of the active profile (selected by `FLASH_PROFILE` at launch).
    pub current_profile: usize,
    /// Initial-view scroll-follow mode (default `Off`). See [`ScrollFollow`].
    pub scroll_follow: ScrollFollow,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_level: "info".to_string(),
            labels: crate::flash::LABEL_CHARS.to_vec(),
            line_labels_unified: false,
            theme: Theme::default(),
            profiles: vec![Profile {
                name: "default".to_string(),
                depths: default_depths(),
            }],
            current_profile: 0,
            scroll_follow: ScrollFollow::Off,
        }
    }
}

/// Built-in default depth list: 200, 5000, unlimited, viewport.
/// `viewport` (just what's on screen) is last since it's rarely the
/// useful mode — the whole point is to navigate scrollback — but it's
/// available in the cycle for the case where you want to confirm what's
/// currently visible without scrollback noise.
fn default_depths() -> Vec<Depth> {
    vec![
        Depth::Lines(200),
        Depth::Lines(5000),
        Depth::Unlimited,
        Depth::Viewport,
    ]
}

/// Find a profile by name (case-insensitive). Returns its index.
fn find_profile(profiles: &[Profile], name: &str) -> Option<usize> {
    profiles
        .iter()
        .position(|p| p.name.eq_ignore_ascii_case(name))
}

/// Load config from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`, falling back
/// to built-in defaults on any error (missing file, unreadable, parse
/// error). Parse errors are reported on stderr. The `FLASH_PROFILE` env
/// var selects the active profile at launch (falls back to "default").
pub fn load() -> Config {
    let mut config = Config::default();

    let config_dir = match std::env::var("HERDR_PLUGIN_CONFIG_DIR") {
        Ok(dir) => dir,
        Err(_) => {
            // No config dir env — use defaults, select profile by FLASH_PROFILE.
            select_profile(&mut config);
            return config;
        }
    };

    let config_path = std::path::Path::new(&config_dir).join("config.toml");
    let text = match std::fs::read_to_string(&config_path) {
        Ok(t) => t,
        Err(e) => {
            // Missing/unreadable file — not an error, just use defaults.
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("herdr-flash: cannot read {}: {e}", config_path.display());
            }
            select_profile(&mut config);
            return config;
        }
    };

    // Parse the TOML. On error, warn and keep defaults.
    let toml_value: toml::Value = match toml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("herdr-flash: parse error in {}: {e}", config_path.display());
            select_profile(&mut config);
            return config;
        }
    };

    apply_toml(&mut config, &toml_value);
    select_profile(&mut config);
    config
}

/// Select the active profile by `FLASH_PROFILE` env (falls back to "default").
fn select_profile(config: &mut Config) {
    let name = std::env::var("FLASH_PROFILE").unwrap_or_else(|_| "default".to_string());
    if let Some(idx) = find_profile(&config.profiles, &name) {
        config.current_profile = idx;
    } else {
        // Unknown profile — warn and stay on default (index 0).
        eprintln!("herdr-flash: unknown FLASH_PROFILE '{name}', using default");
        config.current_profile = 0;
    }
}

/// Apply a parsed TOML value onto the config, overriding defaults.
fn apply_toml(config: &mut Config, root: &toml::Value) {
    let Some(table) = root.as_table() else {
        return;
    };

    // Global keys.
    if let Some(v) = table.get("log_level").and_then(toml::Value::as_str) {
        config.log_level = v.to_string();
    }
    if let Some(v) = table.get("labels").and_then(toml::Value::as_str) {
        let chars: Vec<char> = v.chars().collect();
        if !chars.is_empty() {
            config.labels = chars;
        }
    }
    if let Some(v) = table.get("line_labels").and_then(toml::Value::as_str) {
        config.line_labels_unified = matches!(v.trim(), "unified" | "custom" | "true" | "on");
    }
    if let Some(v) = table.get("scroll_follow").and_then(toml::Value::as_str) {
        match parse_scroll_follow(v) {
            Some(m) => config.scroll_follow = m,
            None => eprintln!(
                "herdr-flash: unknown scroll_follow '{v}', using off (expected off|offset|content)"
            ),
        }
    }

    // Theme: 16 color_* roles (hex #rrggbb).
    apply_color(table, "color_sel_bg", &mut config.theme.sel_bg);
    apply_color(table, "color_sel_fg", &mut config.theme.sel_fg);
    apply_color(table, "color_cursor_bg", &mut config.theme.cursor_bg);
    apply_color(table, "color_cursor_fg", &mut config.theme.cursor_fg);
    apply_color(table, "color_gutter_mark", &mut config.theme.gutter_cursor);
    apply_color(table, "color_gutter_dim", &mut config.theme.gutter_dim);
    apply_color(table, "color_sel_label", &mut config.theme.sel_indicator);
    apply_color(table, "color_footer_dim", &mut config.theme.footer_dim);
    apply_color(table, "color_footer_key", &mut config.theme.footer_key);
    apply_color(
        table,
        "color_jump_label_bg",
        &mut config.theme.jump_label_bg,
    );
    apply_color(
        table,
        "color_jump_label_fg",
        &mut config.theme.jump_label_fg,
    );
    apply_color(
        table,
        "color_jump_match_fg",
        &mut config.theme.jump_match_fg,
    );
    apply_color(
        table,
        "color_jump_partial_fg",
        &mut config.theme.jump_partial_fg,
    );
    apply_color(
        table,
        "color_search_match_bg",
        &mut config.theme.search_match_bg,
    );
    apply_color(
        table,
        "color_search_current_bg",
        &mut config.theme.search_current_bg,
    );
    apply_color(table, "color_search_fg", &mut config.theme.search_fg);

    // Profiles: [profiles.<name>] with depths = [...].
    if let Some(toml::Value::Table(profiles)) = table.get("profiles") {
        let mut parsed: Vec<Profile> = Vec::new();
        for (name, val) in profiles {
            if let Some(p) = parse_profile(name, val) {
                parsed.push(p);
            }
        }
        if !parsed.is_empty() {
            // Ensure a "default" profile always exists.
            if find_profile(&parsed, "default").is_none() {
                parsed.push(Profile {
                    name: "default".to_string(),
                    depths: default_depths(),
                });
            }
            config.profiles = parsed;
        }
    }
}

/// Parse a single `[profiles.<name>]` entry.
fn parse_profile(name: &str, val: &toml::Value) -> Option<Profile> {
    let table = val.as_table()?;
    let depths_val = table.get("depths")?;
    let depth_strs: Vec<&str> = depths_val
        .as_array()?
        .iter()
        .filter_map(toml::Value::as_str)
        .collect();
    let depths: Vec<Depth> = depth_strs.iter().filter_map(|s| parse_depth(s)).collect();
    if depths.is_empty() {
        return None;
    }
    Some(Profile {
        name: name.to_string(),
        depths,
    })
}

/// Apply a `color_*` key from the TOML table to a theme field.
fn apply_color(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    target: &mut ratatui::style::Color,
) {
    if let Some(v) = table.get(key).and_then(toml::Value::as_str) {
        if let Some(c) = parse_hex_color(v) {
            *target = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_four_depths() {
        let c = Config::default();
        assert_eq!(c.profiles.len(), 1);
        assert_eq!(c.profiles[0].name, "default");
        assert_eq!(c.profiles[0].depths.len(), 4);
        assert_eq!(c.profiles[0].depths[0], Depth::Lines(200));
        assert_eq!(c.profiles[0].depths[1], Depth::Lines(5000));
        assert_eq!(c.profiles[0].depths[2], Depth::Unlimited);
        assert_eq!(c.profiles[0].depths[3], Depth::Viewport);
    }

    #[test]
    fn parse_depth_viewport() {
        assert_eq!(parse_depth("viewport"), Some(Depth::Viewport));
        assert_eq!(parse_depth("VIEWPORT"), Some(Depth::Viewport));
    }

    #[test]
    fn parse_depth_lines() {
        assert_eq!(parse_depth("200"), Some(Depth::Lines(200)));
        assert_eq!(parse_depth("2000"), Some(Depth::Lines(2000)));
    }

    #[test]
    fn parse_depth_unlimited() {
        assert_eq!(parse_depth("unlimited"), Some(Depth::Unlimited));
        assert_eq!(parse_depth("UNLIMITED"), Some(Depth::Unlimited));
        assert_eq!(parse_depth("all"), Some(Depth::Unlimited));
    }

    #[test]
    fn parse_depth_rejects_zero_and_garbage() {
        assert_eq!(parse_depth("0"), None);
        assert_eq!(parse_depth("abc"), None);
        assert_eq!(parse_depth(""), None);
    }

    #[test]
    fn depth_label() {
        assert_eq!(Depth::Viewport.label(), "viewport");
        assert_eq!(Depth::Lines(200).label(), "200");
        assert_eq!(Depth::Unlimited.label(), "unlimited");
    }

    #[test]
    fn apply_toml_parses_profiles() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(
            r#"
[profiles.deep]
depths = ["2000", "5000"]

[profiles.quick]
depths = ["viewport", "100"]
"#,
        )
        .unwrap();
        apply_toml(&mut config, &toml);
        assert_eq!(config.profiles.len(), 3); // deep, quick, + auto-added default
        let deep = config.profiles.iter().find(|p| p.name == "deep").unwrap();
        assert_eq!(deep.depths, vec![Depth::Lines(2000), Depth::Lines(5000)]);
        let quick = config.profiles.iter().find(|p| p.name == "quick").unwrap();
        assert_eq!(quick.depths, vec![Depth::Viewport, Depth::Lines(100)]);
    }

    #[test]
    fn apply_toml_ensures_default_profile_exists() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(
            r#"
[profiles.custom]
depths = ["viewport"]
"#,
        )
        .unwrap();
        apply_toml(&mut config, &toml);
        // Should have "custom" AND "default" (auto-added).
        assert!(config.profiles.iter().any(|p| p.name == "custom"));
        assert!(config.profiles.iter().any(|p| p.name == "default"));
    }

    #[test]
    fn apply_toml_parses_labels() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(r#"labels = "asdfjkl;""#).unwrap();
        apply_toml(&mut config, &toml);
        assert_eq!(config.labels, vec!['a', 's', 'd', 'f', 'j', 'k', 'l', ';']);
    }

    #[test]
    fn apply_toml_parses_line_labels_unified() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(r#"line_labels = "unified""#).unwrap();
        apply_toml(&mut config, &toml);
        assert!(config.line_labels_unified);
    }

    #[test]
    fn apply_toml_parses_color_override() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(r##"color_sel_bg = "#ff0000""##).unwrap();
        apply_toml(&mut config, &toml);
        assert_eq!(config.theme.sel_bg, ratatui::style::Color::Rgb(255, 0, 0));
    }

    #[test]
    fn apply_toml_empty_depths_skips_profile() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(
            r#"
[profiles.empty]
depths = []
"#,
        )
        .unwrap();
        apply_toml(&mut config, &toml);
        // "empty" should NOT be added (empty depths); only "default" remains.
        assert!(config.profiles.iter().all(|p| p.name == "default"));
    }

    #[test]
    fn parse_scroll_follow_values() {
        assert_eq!(parse_scroll_follow("off"), Some(ScrollFollow::Off));
        assert_eq!(parse_scroll_follow("OFFSET"), Some(ScrollFollow::Offset));
        assert_eq!(parse_scroll_follow("content"), Some(ScrollFollow::Content));
        assert_eq!(parse_scroll_follow("none"), Some(ScrollFollow::Off));
        assert_eq!(parse_scroll_follow("garbage"), None);
        assert_eq!(parse_scroll_follow(""), None);
    }

    #[test]
    fn apply_toml_parses_scroll_follow() {
        let mut config = Config::default();
        let toml: toml::Value = toml::from_str(r#"scroll_follow = "content""#).unwrap();
        apply_toml(&mut config, &toml);
        assert_eq!(config.scroll_follow, ScrollFollow::Content);
    }

    #[test]
    fn default_scroll_follow_is_off() {
        assert_eq!(Config::default().scroll_follow, ScrollFollow::Off);
    }
}
