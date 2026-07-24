//! Loads optional leap settings from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//!
//! Every field is optional; a missing file or directory yields defaults. Style colors reuse the
//! reference plugin's `parse_color` logic so named colors and `#RRGGBB` both work.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::app::Mode;
use crate::theme::{parse_color, Theme};

/// Fully-resolved settings the entry point consumes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeapSettings {
    /// Number of search characters typed before labeling. The MVP supports exactly 1.
    pub search_chars: usize,
    pub mode: Mode,
    pub copy_toast: bool,
    pub theme: Theme,
}

impl Default for LeapSettings {
    fn default() -> Self {
        Self {
            search_chars: 1,
            mode: Mode::Jump,
            copy_toast: false,
            theme: Theme::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawConfig {
    search_chars: Option<usize>,
    mode: Option<String>,
    #[serde(default)]
    copy_toast: bool,
    style: Option<StyleConfig>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct StyleConfig {
    hint_fg: Option<String>,
    hint_bg: Option<String>,
    match_fg: Option<String>,
    match_bg: Option<String>,
    selected_hint_fg: Option<String>,
    selected_hint_bg: Option<String>,
    selected_match_fg: Option<String>,
    selected_match_bg: Option<String>,
    status_fg: Option<String>,
    status_bg: Option<String>,
    empty_fg: Option<String>,
}

fn parse_config(input: &str) -> Result<RawConfig, toml::de::Error> {
    toml::from_str(input)
}

fn parse_mode(raw: Option<&str>) -> Mode {
    match raw {
        Some("select") => Mode::Select,
        _ => Mode::Jump,
    }
}

fn compile_settings(raw: &RawConfig) -> Result<LeapSettings> {
    Ok(LeapSettings {
        // Only single-character search is supported in the MVP; clamp any other value to 1 rather
        // than erroring so a forward-looking config still loads.
        search_chars: raw.search_chars.unwrap_or(1).clamp(1, 1),
        mode: parse_mode(raw.mode.as_deref()),
        copy_toast: raw.copy_toast,
        theme: compile_theme(raw.style.as_ref())?,
    })
}

fn compile_theme(style: Option<&StyleConfig>) -> Result<Theme> {
    let mut theme = Theme::default();
    let Some(style) = style else {
        return Ok(theme);
    };
    if let Some(value) = &style.hint_fg {
        theme.hint_fg = parse_color(value).context("invalid style.hint_fg")?;
    }
    if let Some(value) = &style.hint_bg {
        theme.hint_bg = parse_color(value).context("invalid style.hint_bg")?;
    }
    if let Some(value) = &style.match_fg {
        theme.match_fg = parse_color(value).context("invalid style.match_fg")?;
    }
    if let Some(value) = &style.match_bg {
        theme.match_bg = Some(parse_color(value).context("invalid style.match_bg")?);
    }
    if let Some(value) = &style.selected_hint_fg {
        theme.selected_hint_fg = parse_color(value).context("invalid style.selected_hint_fg")?;
    }
    if let Some(value) = &style.selected_hint_bg {
        theme.selected_hint_bg = parse_color(value).context("invalid style.selected_hint_bg")?;
    }
    if let Some(value) = &style.selected_match_fg {
        theme.selected_match_fg = parse_color(value).context("invalid style.selected_match_fg")?;
    }
    if let Some(value) = &style.selected_match_bg {
        theme.selected_match_bg = parse_color(value).context("invalid style.selected_match_bg")?;
    }
    if let Some(value) = &style.status_fg {
        theme.status_fg = parse_color(value).context("invalid style.status_fg")?;
    }
    if let Some(value) = &style.status_bg {
        theme.status_bg = parse_color(value).context("invalid style.status_bg")?;
    }
    if let Some(value) = &style.empty_fg {
        theme.empty_fg = parse_color(value).context("invalid style.empty_fg")?;
    }
    Ok(theme)
}

/// Load settings from `<config_dir>/config.toml`, or defaults when the dir/file is absent.
pub fn load_leap_settings(config_dir: Option<&Path>) -> Result<LeapSettings> {
    let Some(config_dir) = config_dir else {
        return Ok(LeapSettings::default());
    };
    let config_path = config_dir.join("config.toml");
    let input = match std::fs::read_to_string(&config_path) {
        Ok(input) => input,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LeapSettings::default());
        }
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let raw = parse_config(&input)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    compile_settings(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use ratatui::style::Color;

    #[test]
    fn empty_config_yields_defaults() {
        let settings = compile_settings(&parse_config("").unwrap()).unwrap();
        assert_eq!(settings, LeapSettings::default());
        assert_eq!(settings.search_chars, 1);
        assert_eq!(settings.mode, Mode::Jump);
        assert!(!settings.copy_toast);
    }

    #[test]
    fn parses_jump_mode() {
        let settings = compile_settings(&parse_config("mode = \"jump\"").unwrap()).unwrap();
        assert_eq!(settings.mode, Mode::Jump);
    }

    #[test]
    fn unknown_mode_falls_back_to_jump() {
        let settings = compile_settings(&parse_config("mode = \"teleport\"").unwrap()).unwrap();
        assert_eq!(settings.mode, Mode::Jump);
    }

    #[test]
    fn parses_select_mode() {
        let settings = compile_settings(&parse_config("mode = \"select\"").unwrap()).unwrap();
        assert_eq!(settings.mode, Mode::Select);
    }

    #[test]
    fn parses_copy_toast_true() {
        let settings = compile_settings(&parse_config("copy_toast = true").unwrap()).unwrap();
        assert!(settings.copy_toast);
    }

    #[test]
    fn other_search_chars_are_clamped_to_one() {
        let settings = compile_settings(&parse_config("search_chars = 3").unwrap()).unwrap();
        assert_eq!(settings.search_chars, 1);
    }

    #[test]
    fn compiles_custom_style_colors() {
        let config = parse_config(
            r##"
[style]
hint_fg = "black"
hint_bg = "light-yellow"
match_fg = "#ffd75f"
status_bg = "blue"
"##,
        )
        .unwrap();
        let settings = compile_settings(&config).unwrap();
        assert_eq!(settings.theme.hint_bg, Color::LightYellow);
        assert_eq!(settings.theme.match_fg, Color::Rgb(255, 215, 95));
        assert_eq!(settings.theme.status_bg, Color::Blue);
    }

    #[test]
    fn rejects_invalid_style_color() {
        let config = parse_config("[style]\nhint_bg = \"not-a-color\"").unwrap();
        let err = compile_settings(&config).unwrap_err();
        assert!(err.to_string().contains("style.hint_bg"));
    }

    #[test]
    fn missing_config_dir_yields_defaults() {
        assert_eq!(load_leap_settings(None).unwrap(), LeapSettings::default());
    }

    #[test]
    fn missing_config_file_yields_defaults() {
        let dir = std::env::temp_dir().join(format!(
            "herdr-leap-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = load_leap_settings(Some(&dir)).unwrap();
        assert_eq!(settings, LeapSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
