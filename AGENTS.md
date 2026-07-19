# Repository Guidelines

`herdr-leap` is a Herdr plugin with two overlay workflows over the focused pane's **visible**
buffer (`pane.read` `source = "visible"`):

1. **Leap (`open` / entrypoint `leap`)** — EasyMotion-style character jump + select-to-copy of an
   arbitrary screen region (await-search → pick-start → pick-end).
2. **Extract (`extract` / entrypoint `extract`)** — extrakto-style typeahead list of copy-eligible
   tokens (url/path/quote/word) from that same visible buffer; Enter copies, Esc cancels.

Both emit OSC 52 for Herdr to forward. Do not collapse the two actions onto one UI.

## Project Shape

- `herdr-plugin.toml` is the Herdr plugin manifest. Keep the plugin id
  (`RooseveltAdvisors.herdr-leap`), action commands, pane commands, and binary name in sync.
  `open` → entrypoint `leap`; `extract` → entrypoint `extract` (`herdr-leap --mode extract`).
- `Cargo.toml` defines both the crate and the release binary as `herdr-leap`.
- `src/leap.rs` owns the pure leap logic: the `WrappedBuffer` coordinate model, character search
  (smartcase), and **region extraction** (`extract_region`) from anchor to extent. This is the
  load-bearing leap module — keep it covered by unit tests.
- `src/extract.rs` owns pure visible-buffer token extraction. This is load-bearing for the extract
  action — keep it covered by unit tests.
- `src/extract_app.rs` owns the extract typeahead state machine; `src/extract_ui.rs` renders it.
- `src/app.rs` owns the leap state machine (`Phase::AwaitSearch | PickStart | PickEnd`),
  hint input handling, and shared `Outcome` transitions (`Continue`, `Copy(String)`, `Cancel`).
- `src/hints.rs` generates unique, stable hint labels (`a`, `s`, … then fixed-width multi-char).
- `src/ui.rs` renders the dimmed leap pane, the hint labels for the current phase, and a status line.
- `src/theme.rs` owns the default TUI theme and user-configurable color parsing.
- `src/config.rs` loads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` (`search_chars`, `mode`,
  `copy_toast`, optional `[style]`).
- `src/herdr_client.rs` is the Herdr Unix-socket JSON-RPC client. `src/clipboard.rs` writes OSC 52.
- `src/main.rs` is the thin TUI entry point (`--mode leap|extract`); keep leap/extract logic,
  theming, clipboard, and socket logic in the library modules.

## Development Process

Use TDD for behavior changes. Add or update failing unit tests describing the intended behavior
first, then implement the smallest change that makes them pass. Keep tests on the pure modules
(`leap`, `extract`, `extract_app`, `hints`, `app`, `theme`, `config`) — no socket or TTY needed.

## Development Commands

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

For local Herdr testing:

```bash
cargo build --release --locked
herdr plugin link .
herdr server reload-config
herdr plugin action invoke RooseveltAdvisors.herdr-leap.open
herdr plugin action invoke RooseveltAdvisors.herdr-leap.extract
herdr plugin config-dir RooseveltAdvisors.herdr-leap
```

## Implementation Notes

- Prefer Herdr's socket API over shelling out to `herdr` from the running TUI.
- Clipboard writes use OSC 52, not platform clipboard commands. Herdr forwards OSC 52 writes from
  plugin panes to the foreground client.
- `mode = "jump"` places Herdr's copy-mode cursor via `pane.copy_mode_jump` (not the child PTY
  caret). Do not fake jump with OSC 52 region copy or synthetic PTY keys.
  Leap pane placement must stay **popup** so the source viewport is not resized before capture.
- Region rendering must keep the original visible pane lines and must not change line widths.
- Do not commit `target/`, runtime logs, or local editor files.
