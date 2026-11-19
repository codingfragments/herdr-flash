//! User config for herdr-flash, loaded once at startup from the first
//! config file that exists (see [`locate`]): `$FLASH_CONFIG`, else
//! `$XDG_CONFIG_HOME/flash/config.toml` (`~/.config/flash/config.toml`),
//! else the legacy herdr location `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//! Optional `[host.<name>]` keys override `scroll_follow` /
//! `key_debounce_ms` for one multiplexer host (see
//! [`apply_host_overrides`]).

//! Layering (Phase 9 scope, extended by the Rex port): global keys
//! (`log_level`, `labels`, `line_labels`, the 17 `color_*` theme roles),
//! per-keybind profiles (`[profiles.<name>]` -> `depths`, selected at
//! launch by `FLASH_PROFILE`), and `[host.<name>]` overrides for one
//! multiplexer (`scroll_follow`, `key_debounce_ms`).
//!
//! Missing/unset/parse-error -> built-in defaults, never crash; parse
//! errors reported on stderr.

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
    /// Theme (17 color roles, Catppuccin Macchiato defaults).
    pub theme: Theme,
    /// Named profiles, selected at launch by `FLASH_PROFILE`.
    pub profiles: Vec<Profile>,
    /// Index of the active profile (selected by `FLASH_PROFILE` at launch).
    pub current_profile: usize,
    /// Initial-view scroll-follow mode (default `Off`). See [`ScrollFollow`].
    pub scroll_follow: ScrollFollow,
    /// Debounce window (ms) for identical consecutive key events — a
    /// workaround for popup keyboard quirks. `None` = not configured:
    /// `main` then picks the host default (`Caps::legacy_keyboard` →
    /// 40, else 0); an explicit root or `[host.<name>]` value wins.
    pub key_debounce_ms: Option<u64>,
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
            key_debounce_ms: None,
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

/// Load the first config file [`locate`] finds, with the running
/// host's `[host.<host>]` overrides applied. `host` is the multiplexer
/// the tool runs under (`"herdr"` / `"rex"`). Missing file, unreadable
/// file or a parse error all fall back to built-in defaults — a broken
/// config must never crash the popup. The `FLASH_PROFILE` env var
/// selects the active profile at launch (falls back to "default").
pub fn load(host: &str) -> Config {
    let mut config = Config::default();

    let Some(config_path) = location().path else {
        select_profile(&mut config);
        return config;
    };
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

    let toml_value = apply_host_overrides(toml_value, host);
    apply_toml(&mut config, &toml_value);
    select_profile(&mut config);
    config
}

/// `[host.<name>]` can override, for one multiplexer only, the keys
/// listed here — everything else under `[host.<name>]` is ignored with
/// a warning.
const HOST_OVERRIDE_KEYS: &[&str] = &["scroll_follow", "key_debounce_ms"];

/// Take the `[host]` table out of `root` and overlay this host's
/// `[host.<host>]` keys over the root keys (a root-level scalar is
/// replaced; anything but a scalar in the override is a warning and
/// ignored). Other hosts' sections are dropped silently. Returns the
/// merged value.
fn apply_host_overrides(mut root: toml::Value, host: &str) -> toml::Value {
    let Some(table) = root.as_table_mut() else {
        return root;
    };
    let Some(hosts) = table.remove("host") else {
        return root;
    };
    let toml::Value::Table(mut hosts) = hosts else {
        eprintln!("herdr-flash: ignored `host`: expected a table like [host.rex]");
        return root;
    };
    let Some(ours) = hosts.remove(host) else {
        return root;
    };
    let toml::Value::Table(ours) = ours else {
        eprintln!("herdr-flash: ignored [host.{host}]: expected a table");
        return root;
    };
    for (key, value) in ours {
        if !HOST_OVERRIDE_KEYS.contains(&key.as_str()) {
            eprintln!(
                "herdr-flash: ignored [host.{host}.{key}]: host overrides support only {}",
                HOST_OVERRIDE_KEYS.join(", ")
            );
            continue;
        }
        if !matches!(value, toml::Value::Table(_) | toml::Value::Array(_)) {
            table.insert(key, value);
        } else {
            eprintln!("herdr-flash: ignored [host.{host}.{key}]: expected a scalar value");
        }
    }
    root
}

/// The environment [`locate`] works from — plain values, so the lookup
/// rules are unit-testable without touching the process environment.
#[derive(Debug, Clone, Default)]
pub struct ConfigEnv {
    /// `$FLASH_CONFIG`: an explicit config file.
    pub explicit: Option<std::path::PathBuf>,
    pub xdg_config_home: Option<std::path::PathBuf>,
    pub home: Option<std::path::PathBuf>,
    /// `$HERDR_PLUGIN_CONFIG_DIR` (legacy herdr location).
    pub herdr_plugin_dir: Option<std::path::PathBuf>,
}

impl ConfigEnv {
    fn from_process_env() -> Self {
        let path_var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
        };
        Self {
            explicit: path_var("FLASH_CONFIG"),
            xdg_config_home: path_var("XDG_CONFIG_HOME"),
            home: path_var("HOME"),
            herdr_plugin_dir: path_var("HERDR_PLUGIN_CONFIG_DIR"),
        }
    }
}

/// The config file in use, if one exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    pub path: Option<std::path::PathBuf>,
}

/// Config lookup:
/// 1. `explicit` (`$FLASH_CONFIG`) — used as given, no fallback.
/// 2. `$XDG_CONFIG_HOME/flash/config.toml` (absolute only, per the XDG
///    spec), else `~/.config/flash/config.toml` — the normal location,
///    shared by every multiplexer the tool runs under.
/// 3. `$HERDR_PLUGIN_CONFIG_DIR/config.toml` — the legacy herdr
///    location, read as a fallback so existing setups keep working.
///
/// The first file that exists wins.
pub fn locate(env: &ConfigEnv, exists: impl Fn(&std::path::Path) -> bool) -> ConfigLocation {
    if let Some(explicit) = &env.explicit {
        return ConfigLocation {
            path: exists(explicit).then(|| explicit.clone()),
        };
    }
    let neutral = env
        .xdg_config_home
        .as_ref()
        .filter(|p| p.is_absolute())
        .map(|p| p.join("flash/config.toml"))
        .or_else(|| {
            env.home
                .as_ref()
                .map(|h| h.join(".config/flash/config.toml"))
        });
    let legacy = env.herdr_plugin_dir.as_ref().map(|d| d.join("config.toml"));
    let candidates: Vec<std::path::PathBuf> = [neutral, legacy].into_iter().flatten().collect();
    ConfigLocation {
        path: candidates.into_iter().find(|p| exists(p)),
    }
}

/// [`locate`] against the real environment and filesystem.
pub fn location() -> ConfigLocation {
    locate(&ConfigEnv::from_process_env(), |p| p.is_file())
}

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
    if let Some(v) = table
        .get("key_debounce_ms")
        .and_then(toml::Value::as_integer)
    {
        if v >= 0 {
            config.key_debounce_ms = Some(v as u64);
        }
    }
    if let Some(v) = table.get("scroll_follow").and_then(toml::Value::as_str) {
        match parse_scroll_follow(v) {
            Some(m) => config.scroll_follow = m,
            None => eprintln!(
                "herdr-flash: unknown scroll_follow '{v}', using off (expected off|offset|content)"
            ),
        }
    }

    // Theme: 17 color_* roles (hex #rrggbb).
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
    apply_color(
        table,
        "color_decoration_dim",
        &mut config.theme.decoration_dim,
    );

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

    #[test]
    fn default_key_debounce_is_40ms() {
        assert_eq!(Config::default().key_debounce_ms, None);
    }

    // ── Config location (Rex port Phase 2) ─────────────────────────

    fn env(
        explicit: Option<&str>,
        xdg: Option<&str>,
        home: Option<&str>,
        herdr: Option<&str>,
    ) -> ConfigEnv {
        ConfigEnv {
            explicit: explicit.map(std::path::PathBuf::from),
            xdg_config_home: xdg.map(std::path::PathBuf::from),
            home: home.map(std::path::PathBuf::from),
            herdr_plugin_dir: herdr.map(std::path::PathBuf::from),
        }
    }

    fn existing<'a>(files: &'a [&'a str]) -> impl Fn(&std::path::Path) -> bool + 'a {
        move |p: &std::path::Path| files.iter().any(|f| std::path::Path::new(f) == p)
    }

    #[test]
    fn locate_prefers_the_neutral_location_over_legacy() {
        let e = env(None, None, Some("/h"), Some("/herdr/cfg"));
        let loc = locate(
            &e,
            existing(&["/h/.config/flash/config.toml", "/herdr/cfg/config.toml"]),
        );
        assert_eq!(
            loc.path,
            Some(std::path::PathBuf::from("/h/.config/flash/config.toml"))
        );
    }

    #[test]
    fn locate_falls_back_to_the_legacy_herdr_location() {
        let e = env(None, None, Some("/h"), Some("/herdr/cfg"));
        let loc = locate(&e, existing(&["/herdr/cfg/config.toml"]));
        assert_eq!(
            loc.path,
            Some(std::path::PathBuf::from("/herdr/cfg/config.toml"))
        );
    }

    #[test]
    fn locate_uses_absolute_xdg_config_home_only() {
        let e = env(None, Some("/x"), Some("/h"), None);
        assert_eq!(
            locate(&e, existing(&["/x/flash/config.toml"])).path,
            Some(std::path::PathBuf::from("/x/flash/config.toml"))
        );
        let e = env(None, Some("rel"), Some("/h"), None);
        assert_eq!(
            locate(&e, existing(&["/h/.config/flash/config.toml"])).path,
            Some(std::path::PathBuf::from("/h/.config/flash/config.toml"))
        );
    }

    #[test]
    fn locate_explicit_wins_and_does_not_fall_back() {
        let e = env(Some("/my/flash.toml"), None, Some("/h"), Some("/herdr/cfg"));
        let loc = locate(
            &e,
            existing(&["/h/.config/flash/config.toml", "/herdr/cfg/config.toml"]),
        );
        assert_eq!(loc.path, None);
        let loc = locate(&e, existing(&["/my/flash.toml"]));
        assert_eq!(loc.path, Some(std::path::PathBuf::from("/my/flash.toml")));
    }

    // ── [host.<name>] overrides ────────────────────────────────────

    #[test]
    fn host_overrides_apply_for_the_running_host_only() {
        let root: toml::Value = toml::from_str(
            r#"
scroll_follow = "offset"
key_debounce_ms = 40

[host.rex]
scroll_follow = "off"
key_debounce_ms = 0

[host.herdr]
key_debounce_ms = 60

[host.rex.labels]
"#,
        )
        .unwrap();
        let mut config = Config::default();
        let merged = apply_host_overrides(root, "rex");
        apply_toml(&mut config, &merged);
        assert_eq!(config.scroll_follow, ScrollFollow::Off);
        assert_eq!(config.key_debounce_ms, Some(0));
        // The other host's values never apply.
        let merged = apply_host_overrides(
            toml::from_str::<toml::Value>(
                r#"
scroll_follow = "offset"
[host.herdr]
key_debounce_ms = 60
[host.rex]
scroll_follow = "off"
"#,
            )
            .unwrap(),
            "herdr",
        );
        let mut config = Config::default();
        apply_toml(&mut config, &merged);
        assert_eq!(config.scroll_follow, ScrollFollow::Offset);
        assert_eq!(config.key_debounce_ms, Some(60));
    }

    #[test]
    fn key_debounce_ms_parses_from_the_root() {
        let mut config = Config::default();
        let root: toml::Value = toml::from_str("key_debounce_ms = 25").unwrap();
        apply_toml(&mut config, &root);
        assert_eq!(config.key_debounce_ms, Some(25));
        // and a host override beats the root
        let root: toml::Value =
            toml::from_str("key_debounce_ms = 25\n[host.rex]\nkey_debounce_ms = 0\n").unwrap();
        let mut config = Config::default();
        apply_toml(&mut config, &apply_host_overrides(root, "rex"));
        assert_eq!(config.key_debounce_ms, Some(0));
    }
}
