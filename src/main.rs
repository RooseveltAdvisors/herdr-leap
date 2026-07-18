use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use herdr_leap::app::{App, Outcome};
use herdr_leap::clipboard::copy_to_clipboard;
use herdr_leap::config::load_leap_settings;
use herdr_leap::herdr_client::{context_focused_pane_id, SocketClient};
use herdr_leap::leap::WrappedBuffer;

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

fn run() -> Result<()> {
    let socket_path = std::env::var_os("HERDR_SOCKET_PATH")
        .context("HERDR_SOCKET_PATH is not set; open this through the Herdr plugin action")?;
    let pane_id = context_focused_pane_id()
        .context("HERDR_PLUGIN_CONTEXT_JSON did not include focused_pane_id")?;
    let mut client = SocketClient::connect(Path::new(&socket_path))?;
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

    let buffer = WrappedBuffer::from_text(&text, wrap_width);
    log_state(&format!(
        "start pane_id={pane_id} rows={} wrap_width={wrap_width:?} mode={} copy_toast={copy_toast}",
        buffer.row_count(),
        settings.mode.label()
    ));
    let mut app = App::new(buffer, settings.theme, settings.mode);

    let outcome = {
        let _restore = TerminalRestore;
        let mut terminal = ratatui::init();
        loop {
            terminal.draw(|frame| herdr_leap::ui::draw(frame, &app))?;
            match event::read()? {
                Event::Key(key) => {
                    if let Some(ch) = key_to_char(key) {
                        match app.handle_char(ch) {
                            Outcome::Continue => {}
                            other => break other,
                        }
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
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

fn key_to_char(key: KeyEvent) -> Option<char> {
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
            key_to_char(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some('\u{1b}')
        );
        assert_eq!(
            key_to_char(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)),
            Some('\u{7f}')
        );
    }

    #[test]
    fn maps_control_c_to_cancel_char() {
        assert_eq!(
            key_to_char(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some('\u{3}')
        );
    }

    #[test]
    fn passes_through_plain_search_characters() {
        assert_eq!(
            key_to_char(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some('t')
        );
    }
}
