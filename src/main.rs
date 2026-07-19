use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use herdr_leap::app::{App, Outcome};
use herdr_leap::clipboard::copy_to_clipboard;
use herdr_leap::config::load_leap_settings;
use herdr_leap::extract_app::{ExtractApp, ExtractInput};
use herdr_leap::herdr_client::{context_focused_pane_id, SocketClient};
use herdr_leap::leap::WrappedBuffer;
use herdr_leap::smart_nav::{decide, Decision, Direction};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log_state(&format!("error: {err:#}"));
            eprintln!("herdr-leap: {err:#}");
            ExitCode::from(1)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunMode {
    Leap,
    Extract,
    SmartNav { direction: Direction },
}

fn run() -> Result<()> {
    let mode = parse_run_mode(std::env::args().skip(1))?;
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .context("HERDR_SOCKET_PATH is not set; open this through the Herdr plugin action")?;
    let pane_id = context_focused_pane_id()
        .context("HERDR_PLUGIN_CONTEXT_JSON did not include focused_pane_id")?;
    let mut client = SocketClient::connect(Path::new(&socket_path))?;

    if let RunMode::SmartNav { direction } = mode {
        return run_smart_nav(&mut client, &pane_id, direction);
    }

    let text = client.read_visible_pane(&pane_id)?;
    let wrap_width = match client.visible_pane_width(&pane_id) {
        Ok(width) => Some(visible_wrap_width(width)),
        Err(err) => {
            log_state(&format!("pane_width_unavailable: {err:#}"));
            None
        }
    };

    let config_dir = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR");
    let settings = load_leap_settings(config_dir.as_deref().map(Path::new))?;
    let copy_toast = settings.copy_toast;

    let outcome = match mode {
        RunMode::Leap => run_leap(&text, wrap_width, &settings)?,
        RunMode::Extract => run_extract(&text, wrap_width, &settings)?,
        RunMode::SmartNav { .. } => unreachable!("smart-nav handled above"),
    };
    log_state(&format!("outcome={outcome:?}"));

    if let Outcome::Copy(text) = outcome {
        copy_to_clipboard(&text)?;
        if copy_toast {
            match client.show_notification(&copy_notification_title(&text)) {
                Ok(result) if !result.shown => {
                    log_state(&format!("notification_not_shown reason={}", result.reason));
                }
                Ok(_) => {}
                Err(err) => {
                    log_state(&format!("notification_error: {err:#}"));
                }
            }
        }
    }
    Ok(())
}

/// One-shot smart pane navigation. No TUI / overlay startup.
fn run_smart_nav(client: &mut SocketClient, pane_id: &str, direction: Direction) -> Result<()> {
    log_state(&format!(
        "start mode=smart-nav direction={} pane={pane_id}",
        direction.as_str()
    ));
    let info = client
        .process_info(pane_id)
        .with_context(|| format!("process_info failed for pane {pane_id}"))?;
    match decide(direction, &info.foreground_processes) {
        Decision::PassThrough { key } => {
            client
                .send_keys(pane_id, &[key])
                .with_context(|| format!("send_keys {key} failed for pane {pane_id}"))?;
            let line = format!("smart-nav: passthrough {key} -> {pane_id}");
            log_state(&line);
            println!("{line}");
        }
        Decision::Focus { direction } => {
            let result = client
                .focus_direction(pane_id, direction)
                .with_context(|| {
                    format!(
                        "focus_direction {} failed for pane {pane_id}",
                        direction.as_str()
                    )
                })?;
            let line = format!(
                "smart-nav: focus {} from={pane_id} changed={} reason={} focused={}",
                direction.as_str(),
                result.changed,
                result.reason.as_deref().unwrap_or("none"),
                result.focused_pane_id.as_deref().unwrap_or("none")
            );
            log_state(&line);
            println!("{line}");
        }
    }
    Ok(())
}

fn run_leap(
    text: &str,
    wrap_width: Option<usize>,
    settings: &herdr_leap::config::LeapSettings,
) -> Result<Outcome> {
    let buffer = WrappedBuffer::from_text(text, wrap_width);
    log_state(&format!(
        "start mode=leap rows={} wrap_width={wrap_width:?} leap_mode={} copy_toast={}",
        buffer.row_count(),
        settings.mode.label(),
        settings.copy_toast
    ));
    let mut app = App::new(buffer, settings.theme.clone(), settings.mode);

    let _restore = TerminalRestore;
    let mut terminal = ratatui::init();
    loop {
        terminal.draw(|frame| herdr_leap::ui::draw(frame, &app))?;
        match event::read()? {
            Event::Key(key) => {
                if let Some(ch) = leap_key_to_char(key) {
                    match app.handle_char(ch) {
                        Outcome::Continue => {}
                        other => return Ok(other),
                    }
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn run_extract(
    text: &str,
    wrap_width: Option<usize>,
    settings: &herdr_leap::config::LeapSettings,
) -> Result<Outcome> {
    let mut app =
        ExtractApp::from_visible_text_with_wrap_width(text, wrap_width, settings.theme.clone());
    log_state(&format!(
        "start mode=extract items={} wrap_width={wrap_width:?} copy_toast={}",
        app.total_count(),
        settings.copy_toast
    ));

    let _restore = TerminalRestore;
    let mut terminal = ratatui::init();
    loop {
        terminal.draw(|frame| herdr_leap::extract_ui::draw(frame, &app))?;
        match event::read()? {
            Event::Key(key) => {
                if let Some(input) = extract_key_to_input(key) {
                    match app.handle_input(input) {
                        Outcome::Continue => {}
                        other => return Ok(other),
                    }
                }
            }
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn parse_run_mode<I, S>(args: I) -> Result<RunMode>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut mode = RunMode::Leap;
    let mut direction: Option<Direction> = None;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_ref();
        match arg {
            "--mode" => {
                let value = iter
                    .next()
                    .map(|s| s.as_ref().to_string())
                    .context("--mode requires leap|extract|smart-nav")?;
                mode = parse_mode_value(&value)?;
            }
            flag if flag.starts_with("--mode=") => {
                mode = parse_mode_value(&flag[7..])?;
            }
            "--direction" => {
                let value = iter
                    .next()
                    .map(|s| s.as_ref().to_string())
                    .context("--direction requires left|down|up|right")?;
                direction = Some(parse_direction_value(&value)?);
            }
            flag if flag.starts_with("--direction=") => {
                direction = Some(parse_direction_value(&flag["--direction=".len()..])?);
            }
            "--help" | "-h" => {
                // Not reached in normal plugin open; keep for local invocation.
                bail!(
                    "usage: herdr-leap [--mode leap|extract|smart-nav] [--direction left|down|up|right]"
                );
            }
            other => bail!("unrecognized argument: {other}"),
        }
    }
    match mode {
        RunMode::SmartNav { .. } => {
            let direction =
                direction.context("--mode smart-nav requires --direction left|down|up|right")?;
            Ok(RunMode::SmartNav { direction })
        }
        _ if direction.is_some() => {
            bail!("--direction is only valid with --mode smart-nav")
        }
        other => Ok(other),
    }
}

fn parse_mode_value(value: &str) -> Result<RunMode> {
    match value {
        "leap" => Ok(RunMode::Leap),
        "extract" => Ok(RunMode::Extract),
        "smart-nav" => Ok(RunMode::SmartNav {
            // Placeholder; parse_run_mode fills the real direction.
            direction: Direction::Left,
        }),
        other => bail!("unknown --mode {other:?} (expected leap|extract|smart-nav)"),
    }
}

fn parse_direction_value(value: &str) -> Result<Direction> {
    Direction::parse(value)
        .with_context(|| format!("unknown --direction {value:?} (expected left|down|up|right)"))
}

fn copy_notification_title(text: &str) -> String {
    let mut chars = text.chars();
    let mut preview = chars.by_ref().take(15).collect::<String>();
    if chars.next().is_some() {
        preview.push_str("...");
    }
    format!("Copied: {preview}")
}

fn visible_wrap_width(layout_width: usize) -> usize {
    // Herdr's pane rectangle includes the terminal's wrap-pending right edge, while `pane.read`
    // moves that character to the following visible row. Mirror the reference plugin's adjustment.
    if layout_width > 1 {
        layout_width - 1
    } else {
        layout_width
    }
}

fn log_state(message: &str) {
    let Some(dir) = std::env::var_os("HERDR_PLUGIN_STATE_DIR") else {
        return;
    };
    let path = Path::new(&dir).join("herdr-leap.log");
    let line = format!("{message}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
}

struct TerminalRestore;

impl Drop for TerminalRestore {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

fn leap_key_to_char(key: KeyEvent) -> Option<char> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => return Some('\u{3}'),
            _ => return None,
        }
    }
    match key.code {
        KeyCode::Esc => Some('\u{1b}'),
        KeyCode::Backspace => Some('\u{7f}'),
        KeyCode::Char(ch) => Some(ch),
        _ => None,
    }
}

fn extract_key_to_input(key: KeyEvent) -> Option<ExtractInput> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') | KeyCode::Char('C') => Some(ExtractInput::CtrlC),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(ExtractInput::Down),
            KeyCode::Char('p') | KeyCode::Char('P') => Some(ExtractInput::Up),
            _ => None,
        };
    }
    match key.code {
        KeyCode::Esc => Some(ExtractInput::Esc),
        KeyCode::Backspace => Some(ExtractInput::Backspace),
        KeyCode::Enter => Some(ExtractInput::Enter),
        KeyCode::Up => Some(ExtractInput::Up),
        KeyCode::Down => Some(ExtractInput::Down),
        KeyCode::Char(ch) => Some(ExtractInput::Char(ch)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn copy_notification_title_includes_short_text() {
        assert_eq!(copy_notification_title("hello"), "Copied: hello");
    }

    #[test]
    fn copy_notification_title_truncates_after_fifteen_characters() {
        assert_eq!(
            copy_notification_title("1234567890123456"),
            "Copied: 123456789012345..."
        );
    }

    #[test]
    fn copy_notification_title_truncates_by_characters() {
        assert_eq!(
            copy_notification_title("あいうえおかきくけこさしすせそた"),
            "Copied: あいうえおかきくけこさしすせそ..."
        );
    }

    #[test]
    fn visible_wrap_width_excludes_the_terminal_right_edge() {
        assert_eq!(visible_wrap_width(118), 117);
        assert_eq!(visible_wrap_width(1), 1);
    }

    #[test]
    fn maps_escape_and_backspace_keys() {
        assert_eq!(
            leap_key_to_char(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some('\u{1b}')
        );
        assert_eq!(
            leap_key_to_char(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            Some('\u{7f}')
        );
    }

    #[test]
    fn maps_control_c_to_cancel_char() {
        assert_eq!(
            leap_key_to_char(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some('\u{3}')
        );
    }

    #[test]
    fn passes_through_plain_search_characters() {
        assert_eq!(
            leap_key_to_char(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some('t')
        );
    }

    #[test]
    fn parse_run_mode_defaults_to_leap() {
        assert_eq!(parse_run_mode(Vec::<&str>::new()).unwrap(), RunMode::Leap);
    }

    #[test]
    fn parse_run_mode_accepts_extract() {
        assert_eq!(
            parse_run_mode(["--mode", "extract"]).unwrap(),
            RunMode::Extract
        );
        assert_eq!(
            parse_run_mode(["--mode=extract"]).unwrap(),
            RunMode::Extract
        );
    }

    #[test]
    fn parse_run_mode_accepts_smart_nav_with_direction() {
        assert_eq!(
            parse_run_mode(["--mode", "smart-nav", "--direction", "left"]).unwrap(),
            RunMode::SmartNav {
                direction: Direction::Left
            }
        );
        assert_eq!(
            parse_run_mode(["--mode=smart-nav", "--direction=right"]).unwrap(),
            RunMode::SmartNav {
                direction: Direction::Right
            }
        );
    }

    #[test]
    fn parse_run_mode_requires_direction_for_smart_nav() {
        assert!(parse_run_mode(["--mode", "smart-nav"]).is_err());
    }

    #[test]
    fn parse_run_mode_rejects_direction_without_smart_nav() {
        assert!(parse_run_mode(["--direction", "left"]).is_err());
        assert!(parse_run_mode(["--mode", "extract", "--direction", "up"]).is_err());
    }

    #[test]
    fn parse_run_mode_rejects_unknown() {
        assert!(parse_run_mode(["--mode", "teleport"]).is_err());
        assert!(parse_run_mode(["--wat"]).is_err());
        assert!(parse_run_mode(["--mode", "smart-nav", "--direction", "sideways"]).is_err());
    }

    #[test]
    fn extract_key_maps_enter_arrows_and_ctrl_nav() {
        assert_eq!(
            extract_key_to_input(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(ExtractInput::Enter)
        );
        assert_eq!(
            extract_key_to_input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(ExtractInput::Up)
        );
        assert_eq!(
            extract_key_to_input(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            Some(ExtractInput::Down)
        );
    }

    #[test]
    fn manifest_declares_open_extract_and_smart_nav_actions() {
        let manifest = include_str!("../herdr-plugin.toml");
        let value: toml::Value = toml::from_str(manifest).expect("manifest parses");
        let actions = value
            .get("actions")
            .and_then(|v| v.as_array())
            .expect("actions array");
        let ids: Vec<&str> = actions
            .iter()
            .filter_map(|a| a.get("id").and_then(|id| id.as_str()))
            .collect();
        assert!(ids.contains(&"open"), "open action missing: {ids:?}");
        assert!(ids.contains(&"extract"), "extract action missing: {ids:?}");
        for smart in ["smart-left", "smart-down", "smart-up", "smart-right"] {
            assert!(ids.contains(&smart), "{smart} action missing: {ids:?}");
        }

        let open = actions
            .iter()
            .find(|a| a.get("id").and_then(|id| id.as_str()) == Some("open"))
            .unwrap();
        let open_cmd = open
            .get("command")
            .and_then(|c| c.as_array())
            .expect("open command");
        let open_joined = open_cmd
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            open_joined.contains("--entrypoint leap"),
            "open must keep leap entrypoint: {open_joined}"
        );

        let extract = actions
            .iter()
            .find(|a| a.get("id").and_then(|id| id.as_str()) == Some("extract"))
            .unwrap();
        let extract_cmd = extract
            .get("command")
            .and_then(|c| c.as_array())
            .expect("extract command");
        let extract_joined = extract_cmd
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            extract_joined.contains("--entrypoint extract"),
            "extract must open extract entrypoint: {extract_joined}"
        );

        for (id, dir) in [
            ("smart-left", "left"),
            ("smart-down", "down"),
            ("smart-up", "up"),
            ("smart-right", "right"),
        ] {
            let action = actions
                .iter()
                .find(|a| a.get("id").and_then(|x| x.as_str()) == Some(id))
                .unwrap_or_else(|| panic!("{id} missing"));
            let cmd = action
                .get("command")
                .and_then(|c| c.as_array())
                .unwrap_or_else(|| panic!("{id} command"));
            let joined = cmd
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                joined.contains("smart-nav") && joined.contains(dir),
                "{id} must invoke one-shot smart-nav {dir}: {joined}"
            );
            assert!(
                !joined.contains("plugin pane open"),
                "{id} must not open an overlay pane: {joined}"
            );
        }

        let panes = value
            .get("panes")
            .and_then(|v| v.as_array())
            .expect("panes array");
        let pane_ids: Vec<&str> = panes
            .iter()
            .filter_map(|p| p.get("id").and_then(|id| id.as_str()))
            .collect();
        assert!(
            pane_ids.contains(&"leap"),
            "leap pane missing: {pane_ids:?}"
        );
        assert!(
            pane_ids.contains(&"extract"),
            "extract pane missing: {pane_ids:?}"
        );
        // Smart-nav is one-shot; it must not add an overlay pane entrypoint.
        assert!(
            !pane_ids.iter().any(|id| id.contains("smart")),
            "smart-nav must not declare overlay panes: {pane_ids:?}"
        );

        let extract_pane = panes
            .iter()
            .find(|p| p.get("id").and_then(|id| id.as_str()) == Some("extract"))
            .unwrap();
        let pane_cmd = extract_pane
            .get("command")
            .and_then(|c| c.as_array())
            .expect("extract pane command");
        let pane_joined = pane_cmd
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            pane_joined.contains("--mode") && pane_joined.contains("extract"),
            "extract pane must pass --mode extract: {pane_joined}"
        );
    }
}
