use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use herdr_leap::app::{App, Outcome};
use herdr_leap::clipboard::copy_to_clipboard;
use herdr_leap::config::load_leap_settings;
use herdr_leap::herdr_client::{
    context_focused_pane_id, CopyModeJumpRequest, PaneScrollSnapshot, SocketClient,
    VisiblePaneSnapshot,
};
use herdr_leap::leap::{Pos, WrappedBuffer};
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

    // Keep the source viewport identity from before the visible read. The popup placement prevents
    // the picker itself from resizing this pane.
    let scroll = match client.pane_scroll(&pane_id) {
        Ok(scroll) => Some(scroll),
        Err(err) => {
            log_state(&format!("pane_scroll_unavailable: {err:#}"));
            None
        }
    };
    let snapshot = client.read_visible_pane(&pane_id)?;
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

    let outcome = run_leap(&snapshot.text, wrap_width, &settings)?;
    log_state(&format!("outcome={outcome:?}"));

    match outcome {
        Outcome::Jump(pos) => {
            apply_jump(
                &mut client,
                &pane_id,
                &snapshot,
                scroll.as_ref(),
                pos,
                wrap_width,
            )?;
        }
        Outcome::Copy(text) => {
            copy_to_clipboard(&text)?;
            if copy_toast {
                match client.show_notification(&copy_notification_title(&text)) {
                    Ok(result) if !result.shown => {
                        log_state(&format!("notification_not_shown reason={}", result.reason));
                    }
                    Ok(_) => {}
                    Err(err) => log_state(&format!("notification_error: {err:#}")),
                }
            }
        }
        Outcome::Continue | Outcome::Cancel => {}
    }
    Ok(())
}

fn apply_jump(
    client: &mut SocketClient,
    pane_id: &str,
    snapshot: &VisiblePaneSnapshot,
    scroll: Option<&PaneScrollSnapshot>,
    pos: Pos,
    wrap_width: Option<usize>,
) -> Result<()> {
    let scroll = scroll.context("jump requires pane scroll metrics from pane.get")?;
    let buffer = WrappedBuffer::from_text(&snapshot.text, wrap_width);
    let mut request = CopyModeJumpRequest {
        pane_id: pane_id.to_string(),
        viewport_row: buffer.viewport_row(pos),
        viewport_col: buffer.viewport_col(pos),
        revision: snapshot.revision,
        offset_from_bottom: scroll.offset_from_bottom,
    };

    for attempt in 1..=2 {
        log_state(&format!(
            "jump attempt={attempt} pane_id={pane_id} row={} col={} revision={} offset={}",
            request.viewport_row,
            request.viewport_col,
            request.revision,
            request.offset_from_bottom
        ));
        match client.copy_mode_jump(&request) {
            Ok(result) => {
                log_state(&format!(
                    "jump_ok pane_id={} row={} col={}",
                    result.pane_id, result.viewport_row, result.viewport_col
                ));
                return Ok(());
            }
            Err(err) => {
                let message = err.to_string();
                if message.contains("unknown_method") || message.contains("method_not_found") {
                    bail!("jump unavailable: this Herdr build lacks pane.copy_mode_jump");
                }
                if attempt == 1 && message.contains("stale_pane_viewport") {
                    let refreshed_scroll = client.pane_scroll(pane_id)?;
                    let refreshed_snapshot = client.read_visible_pane(pane_id)?;
                    // Re-issue with the fresh revision when the SELECTED row still holds, even if
                    // an unrelated row (a live status/agent line) churned. Refuse only when the
                    // jump target could have moved: the scroll offset, viewport height, wrapped row
                    // count, or the selected row's own content changed. wrap_width is stable here —
                    // the popup is full-size, so capture never resizes the source viewport.
                    let fresh = WrappedBuffer::from_text(&refreshed_snapshot.text, wrap_width);
                    if refreshed_scroll.offset_from_bottom != scroll.offset_from_bottom
                        || refreshed_scroll.viewport_rows != scroll.viewport_rows
                        || fresh.row_count() != buffer.row_count()
                        || fresh.rows().get(pos.row) != buffer.rows().get(pos.row)
                    {
                        bail!("selected viewport row changed; refusing a stale jump");
                    }
                    request.revision = refreshed_snapshot.revision;
                    request.offset_from_bottom = refreshed_scroll.offset_from_bottom;
                    continue;
                }
                return Err(err).context("pane.copy_mode_jump failed");
            }
        }
    }
    unreachable!("two-attempt jump loop always returns")
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
                    .context("--mode requires leap|smart-nav")?;
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
                bail!("usage: herdr-leap [--mode leap|smart-nav] [--direction left|down|up|right]");
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
        "smart-nav" => Ok(RunMode::SmartNav {
            // Placeholder; parse_run_mode fills the real direction.
            direction: Direction::Left,
        }),
        other => bail!("unknown --mode {other:?} (expected leap|smart-nav)"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn parse_run_mode_rejects_migrated_extract_mode() {
        assert!(parse_run_mode(["--mode", "extract"]).is_err());
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
        assert!(parse_run_mode(["--mode", "leap", "--direction", "up"]).is_err());
    }

    #[test]
    fn parse_run_mode_rejects_unknown() {
        assert!(parse_run_mode(["--mode", "teleport"]).is_err());
        assert!(parse_run_mode(["--wat"]).is_err());
        assert!(parse_run_mode(["--mode", "smart-nav", "--direction", "sideways"]).is_err());
    }

    #[test]
    fn stale_jump_retries_when_only_a_non_target_row_changed() {
        // A live status/agent row above the target churns between capture and jump, so the full
        // viewport text differs, but the selected row is byte-identical. The retry must re-issue
        // the same row/cell with the fresh revision instead of refusing.
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::env::temp_dir().join(format!("herdr-leap-jump-safe-{unique}.sock"));
        let listener = UnixListener::bind(&socket_path).unwrap();
        let original_text = "history\n aあtarget";
        let fresh_text = "changed\n aあtarget";

        let handle = std::thread::spawn(move || {
            let (_probe, _) = listener.accept().unwrap();
            for step in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let response = match step {
                    0 => {
                        assert_eq!(request["method"], "pane.copy_mode_jump");
                        assert_eq!(request["params"]["revision"], 41);
                        r#"{"id":"1","error":{"code":"stale_pane_viewport","message":"stale"}}"#
                            .to_string()
                    }
                    1 => {
                        assert_eq!(request["method"], "pane.get");
                        r#"{"id":"2","result":{"type":"pane_info","pane":{"revision":42,"scroll":{"offset_from_bottom":3,"max_offset_from_bottom":9,"viewport_rows":2}}}}"#.to_string()
                    }
                    2 => {
                        assert_eq!(request["method"], "pane.read");
                        format!(
                            r#"{{"id":"3","result":{{"type":"pane_read","read":{{"text":{fresh_text:?},"revision":42}}}}}}"#
                        )
                    }
                    _ => {
                        assert_eq!(request["method"], "pane.copy_mode_jump");
                        assert_eq!(request["params"]["viewport_row"], 1);
                        assert_eq!(request["params"]["viewport_col"], 4);
                        assert_eq!(request["params"]["revision"], 42);
                        assert_eq!(request["params"]["offset_from_bottom"], 3);
                        r#"{"id":"4","result":{"type":"pane_copy_mode_jump","pane_id":"w1:p1","viewport_row":1,"viewport_col":4,"revision":42,"offset_from_bottom":3}}"#.to_string()
                    }
                };
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        apply_jump(
            &mut client,
            "w1:p1",
            &VisiblePaneSnapshot {
                text: original_text.into(),
                revision: 41,
            },
            Some(&PaneScrollSnapshot {
                offset_from_bottom: 3,
                max_offset_from_bottom: 9,
                viewport_rows: 2,
                revision: 40,
            }),
            Pos::new(1, 3),
            None,
        )
        .unwrap();

        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn stale_jump_refuses_when_the_target_row_shifts() {
        // The selected row itself changed under the cursor (reflow/edit), so re-issuing the same
        // row/cell would land on the wrong content. The retry must refuse and never send a second
        // jump.
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::env::temp_dir().join(format!("herdr-leap-jump-shift-{unique}.sock"));
        let listener = UnixListener::bind(&socket_path).unwrap();
        let original_text = "history\n aあtarget";
        let fresh_text = "history\n aあmoved!";

        let handle = std::thread::spawn(move || {
            let (_probe, _) = listener.accept().unwrap();
            for step in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                let request: serde_json::Value = serde_json::from_str(&line).unwrap();
                let response = match step {
                    0 => {
                        assert_eq!(request["method"], "pane.copy_mode_jump");
                        r#"{"id":"1","error":{"code":"stale_pane_viewport","message":"stale"}}"#
                            .to_string()
                    }
                    1 => {
                        assert_eq!(request["method"], "pane.get");
                        r#"{"id":"2","result":{"type":"pane_info","pane":{"revision":42,"scroll":{"offset_from_bottom":3,"max_offset_from_bottom":9,"viewport_rows":2}}}}"#.to_string()
                    }
                    _ => {
                        assert_eq!(request["method"], "pane.read");
                        format!(
                            r#"{{"id":"3","result":{{"type":"pane_read","read":{{"text":{fresh_text:?},"revision":42}}}}}}"#
                        )
                    }
                };
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });

        let mut client = SocketClient::connect(&socket_path).unwrap();
        let err = apply_jump(
            &mut client,
            "w1:p1",
            &VisiblePaneSnapshot {
                text: original_text.into(),
                revision: 41,
            },
            Some(&PaneScrollSnapshot {
                offset_from_bottom: 3,
                max_offset_from_bottom: 9,
                viewport_rows: 2,
                revision: 40,
            }),
            Pos::new(1, 3),
            None,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("refusing a stale jump"),
            "expected a refusal, got: {err:#}"
        );

        handle.join().unwrap();
        let _ = std::fs::remove_file(socket_path);
    }

    #[test]
    fn manifest_keeps_extractor_in_its_own_plugin() {
        let manifest = include_str!("../herdr-plugin.toml");
        let value: toml::Value = toml::from_str(manifest).expect("manifest parses");
        let actions = value
            .get("actions")
            .and_then(|v| v.as_array())
            .expect("actions array");
        let action_ids: Vec<&str> = actions
            .iter()
            .filter_map(|action| action.get("id").and_then(|id| id.as_str()))
            .collect();
        assert!(
            !action_ids.contains(&"extract"),
            "extract action moved to RooseveltAdvisors.herdr-extractor: {action_ids:?}"
        );

        let panes = value
            .get("panes")
            .and_then(|v| v.as_array())
            .expect("panes array");
        let pane_ids: Vec<&str> = panes
            .iter()
            .filter_map(|pane| pane.get("id").and_then(|id| id.as_str()))
            .collect();
        assert!(
            !pane_ids.contains(&"extract"),
            "extract pane moved to RooseveltAdvisors.herdr-extractor: {pane_ids:?}"
        );
    }

    #[test]
    fn manifest_declares_open_and_one_shot_smart_nav_actions() {
        let manifest = include_str!("../herdr-plugin.toml");
        let value: toml::Value = toml::from_str(manifest).expect("manifest parses");
        let actions = value
            .get("actions")
            .and_then(|v| v.as_array())
            .expect("actions array");
        let ids: Vec<&str> = actions
            .iter()
            .filter_map(|action| action.get("id").and_then(|id| id.as_str()))
            .collect();
        assert!(ids.contains(&"open"), "open action missing: {ids:?}");
        for smart in ["smart-left", "smart-down", "smart-up", "smart-right"] {
            assert!(ids.contains(&smart), "{smart} action missing: {ids:?}");
        }

        let open = actions
            .iter()
            .find(|action| action.get("id").and_then(|id| id.as_str()) == Some("open"))
            .expect("open action");
        assert_eq!(
            open.get("command")
                .and_then(|command| command.as_array())
                .and_then(|command| command.first())
                .and_then(|part| part.as_str()),
            Some("./scripts/open-leap")
        );

        for (id, direction) in [
            ("smart-left", "left"),
            ("smart-down", "down"),
            ("smart-up", "up"),
            ("smart-right", "right"),
        ] {
            let action = actions
                .iter()
                .find(|action| action.get("id").and_then(|value| value.as_str()) == Some(id))
                .unwrap_or_else(|| panic!("{id} missing"));
            let command = action
                .get("command")
                .and_then(|command| command.as_array())
                .expect("smart-nav command")
                .iter()
                .filter_map(|part| part.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert!(command.contains("smart-nav") && command.contains(direction));
            assert!(!command.contains("plugin pane open"));
        }
    }

    #[test]
    fn open_script_falls_back_when_herdr_bin_path_is_stale() {
        let script = include_str!("../scripts/open-leap");
        assert!(script.contains("[ -x \"$HERDR_BIN_PATH\" ]"));
        assert!(script.contains("command -v herdr"));
        assert!(script.contains("--entrypoint leap"));
        assert!(script.contains("--placement popup"));
        assert!(script.contains("--width 100%"));
        assert!(script.contains("--height 100%"));
    }
}
