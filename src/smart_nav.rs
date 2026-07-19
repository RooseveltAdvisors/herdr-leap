//! Vim-aware smart pane navigation (tmux `vim-tmux-navigator` parity for terminal mode).
//!
//! Pure dispatch: if the focused pane's foreground process matches the editor/fzf predicate,
//! forward the same Ctrl chord into the pane; otherwise focus the geometric neighbor in the
//! same tab. No-neighbor is a quiet no-op at the API layer.

use std::path::Path;

/// Geometric navigation direction (same vocabulary as Herdr `pane.focus_direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

impl Direction {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "left" => Some(Self::Left),
            "down" => Some(Self::Down),
            "up" => Some(Self::Up),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Down => "down",
            Self::Up => "up",
            Self::Right => "right",
        }
    }

    /// Herdr key-combo string for `pane.send_keys` when passthrough is required.
    pub fn ctrl_key(self) -> &'static str {
        match self {
            Self::Left => "ctrl+h",
            Self::Down => "ctrl+j",
            Self::Up => "ctrl+k",
            Self::Right => "ctrl+l",
        }
    }
}

/// What the smart-nav one-shot should do after inspecting the focused pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Forward `key` into the focused pane (editor / fzf family).
    PassThrough { key: &'static str },
    /// Focus the geometric neighbor in `direction` (shell / ordinary process).
    Focus { direction: Direction },
}

/// One foreground process entry from `pane.process_info`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub name: String,
    pub argv0: Option<String>,
}

/// Decide passthrough vs focus from foreground process names/argv0 basenames.
///
/// Matches the captain tmux `is_vim` process-name contract (and harmless upstream extras:
/// optional leading `.`, optional trailing `-wrapped`):
/// `g?(view|l?n?vim?x?|fzf)(diff)?`
pub fn decide(direction: Direction, processes: &[ForegroundProcess]) -> Decision {
    if processes.iter().any(process_is_passthrough) {
        Decision::PassThrough {
            key: direction.ctrl_key(),
        }
    } else {
        Decision::Focus { direction }
    }
}

fn process_is_passthrough(process: &ForegroundProcess) -> bool {
    if is_passthrough_name(&process.name) {
        return true;
    }
    if let Some(argv0) = process.argv0.as_deref() {
        let base = Path::new(argv0)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(argv0);
        if is_passthrough_name(base) {
            return true;
        }
    }
    false
}

/// Process-name form of captain tmux.conf `is_vim` + optional upstream `.` / `-wrapped`.
pub fn is_passthrough_name(name: &str) -> bool {
    // Case-insensitive. Anchored full match on the basename.
    // g? \.? (view | l? n? vi m? x? | fzf) (diff)? (-wrapped)?
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }

    // Lowercase into a small stack buffer (process names are short).
    let mut lower = [0u8; 64];
    if bytes.len() > lower.len() {
        return match_passthrough_owned(&name.to_ascii_lowercase());
    }
    for (i, b) in bytes.iter().enumerate() {
        lower[i] = b.to_ascii_lowercase();
    }
    match_passthrough_bytes(&lower[..bytes.len()])
}

fn match_passthrough_owned(lower: &str) -> bool {
    match_passthrough_bytes(lower.as_bytes())
}

fn match_passthrough_bytes(s: &[u8]) -> bool {
    let mut i = 0;
    // optional leading g
    if s.get(i) == Some(&b'g') {
        i += 1;
    }
    // optional leading .
    if s.get(i) == Some(&b'.') {
        i += 1;
    }

    let rest = &s[i..];
    let (matched_core, after_core) = if rest.starts_with(b"view") {
        (true, 4)
    } else if rest.starts_with(b"fzf") {
        (true, 3)
    } else if let Some(n) = match_vim_family(rest) {
        (true, n)
    } else {
        (false, 0)
    };
    if !matched_core {
        return false;
    }
    let mut j = after_core;
    if rest[j..].starts_with(b"diff") {
        j += 4;
    }
    if rest[j..].starts_with(b"-wrapped") {
        j += 8;
    }
    j == rest.len()
}

/// `l?n?vim?x?` → number of bytes consumed, or None.
fn match_vim_family(s: &[u8]) -> Option<usize> {
    let mut i = 0;
    if s.get(i) == Some(&b'l') {
        i += 1;
    }
    if s.get(i) == Some(&b'n') {
        i += 1;
    }
    if s.get(i) != Some(&b'v') || s.get(i + 1) != Some(&b'i') {
        return None;
    }
    i += 2;
    if s.get(i) == Some(&b'm') {
        i += 1;
    }
    if s.get(i) == Some(&b'x') {
        i += 1;
    }
    Some(i)
}

/// Result of performing a smart-nav decision against Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartNavResult {
    PassedThrough {
        pane_id: String,
        key: &'static str,
    },
    Focused {
        pane_id: String,
        direction: Direction,
        changed: bool,
        reason: Option<String>,
        focused_pane_id: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn direction_parses_and_maps_ctrl_keys() {
        assert_eq!(Direction::parse("left"), Some(Direction::Left));
        assert_eq!(Direction::parse("down"), Some(Direction::Down));
        assert_eq!(Direction::parse("up"), Some(Direction::Up));
        assert_eq!(Direction::parse("right"), Some(Direction::Right));
        assert_eq!(Direction::parse("sideways"), None);
        assert_eq!(Direction::Left.ctrl_key(), "ctrl+h");
        assert_eq!(Direction::Down.ctrl_key(), "ctrl+j");
        assert_eq!(Direction::Up.ctrl_key(), "ctrl+k");
        assert_eq!(Direction::Right.ctrl_key(), "ctrl+l");
        assert_eq!(Direction::Left.as_str(), "left");
    }

    #[test]
    fn predicate_accepts_vim_family_and_fzf() {
        for name in [
            "nvim",
            "vim",
            "vi",
            "view",
            "gview",
            "gvim",
            "lvim",
            "vimdiff",
            "nvimdiff",
            "fzf",
            ".nvim",
            "nvim-wrapped",
            "VIM",
            "Nvim",
        ] {
            assert!(
                is_passthrough_name(name),
                "expected passthrough for {name:?}"
            );
        }
    }

    #[test]
    fn predicate_rejects_ordinary_processes() {
        for name in [
            "zsh", "bash", "python", "helix", "emacs", "node", "cargo", "",
        ] {
            assert!(!is_passthrough_name(name), "expected reject for {name:?}");
        }
    }

    #[test]
    fn decide_passthrough_for_nvim_name() {
        let procs = [ForegroundProcess {
            name: "nvim".into(),
            argv0: Some("nvim".into()),
        }];
        assert_eq!(
            decide(Direction::Left, &procs),
            Decision::PassThrough { key: "ctrl+h" }
        );
    }

    #[test]
    fn decide_passthrough_for_path_qualified_argv0() {
        let procs = [ForegroundProcess {
            name: "nvim".into(),
            argv0: Some("/usr/bin/nvim".into()),
        }];
        assert_eq!(
            decide(Direction::Right, &procs),
            Decision::PassThrough { key: "ctrl+l" }
        );
    }

    #[test]
    fn decide_passthrough_uses_argv0_basename_when_name_is_generic() {
        // Some platforms may report a generic name; argv0 still wins.
        let procs = [ForegroundProcess {
            name: "main".into(),
            argv0: Some("/opt/homebrew/bin/fzf".into()),
        }];
        assert_eq!(
            decide(Direction::Up, &procs),
            Decision::PassThrough { key: "ctrl+k" }
        );
    }

    #[test]
    fn decide_focus_for_shell() {
        let procs = [ForegroundProcess {
            name: "zsh".into(),
            argv0: Some("zsh".into()),
        }];
        assert_eq!(
            decide(Direction::Down, &procs),
            Decision::Focus {
                direction: Direction::Down
            }
        );
    }

    #[test]
    fn decide_focus_when_no_foreground_processes() {
        assert_eq!(
            decide(Direction::Left, &[]),
            Decision::Focus {
                direction: Direction::Left
            }
        );
    }

    #[test]
    fn decide_passthrough_if_any_process_matches() {
        let procs = [
            ForegroundProcess {
                name: "zsh".into(),
                argv0: None,
            },
            ForegroundProcess {
                name: "fzf".into(),
                argv0: None,
            },
        ];
        assert_eq!(
            decide(Direction::Left, &procs),
            Decision::PassThrough { key: "ctrl+h" }
        );
    }
}
