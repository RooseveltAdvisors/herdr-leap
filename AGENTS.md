# Repository Guidelines

`herdr-leap` is a Herdr plugin with two overlay workflows over the focused pane's **visible**
buffer (`pane.read` `source = "visible"`), plus one-shot smart pane navigation:

1. **Leap (`open` / entrypoint `leap`)** — EasyMotion-style character jump + select-to-copy of an
   arbitrary screen region (await-search → pick-start → pick-end).
2. **Extract (`extract` / entrypoint `extract`)** — extrakto-style typeahead list of copy-eligible
   tokens (url/path/quote/word) from that same visible buffer; Enter copies, Esc cancels.
3. **Smart nav (`smart-left` / `smart-down` / `smart-up` / `smart-right`)** — one-shot
   vim-tmux-navigator-style dispatch: if the focused pane's foreground process matches the
   editor/fzf predicate, `pane.send_keys` the matching Ctrl chord; otherwise
   `pane.focus_direction` in that direction. No overlay, no TUI.

Leap and extract emit OSC 52 for Herdr to forward. Do not collapse leap and extract onto one UI.
Smart-nav must stay a one-shot path (no pane entrypoint).

## Project Shape

- `herdr-plugin.toml` is the Herdr plugin manifest. Keep the plugin id
  (`RooseveltAdvisors.herdr-leap`), action commands, pane commands, and binary name in sync.
  `open` → entrypoint `leap`; `extract` → entrypoint `extract` (`herdr-leap --mode extract`);
  `smart-*` → `./target/release/herdr-leap --mode smart-nav --direction …` (no pane open).
- `Cargo.toml` defines both the crate and the release binary as `herdr-leap`.
- `src/leap.rs` owns the pure leap logic: the `WrappedBuffer` coordinate model, character search
  (smartcase), and **region extraction** (`extract_region`) from anchor to extent. This is the
  load-bearing leap module — keep it covered by unit tests.
- `src/extract.rs` owns pure visible-buffer token extraction. This is load-bearing for the extract
  action — keep it covered by unit tests.
- `src/extract_app.rs` owns the extract typeahead state machine; `src/extract_ui.rs` renders it.
- `src/smart_nav.rs` owns pure smart-nav dispatch: direction vocabulary, editor/fzf predicate, and
  `decide()` → passthrough vs focus. Keep it covered by unit tests.
- `src/app.rs` owns the leap state machine (`Phase::AwaitSearch | PickStart | PickEnd`),
  hint input handling, and shared `Outcome` transitions (`Continue`, `Copy(String)`, `Cancel`).
- `src/hints.rs` generates unique, stable hint labels (`a`, `s`, … then fixed-width multi-char).
- `src/ui.rs` renders the dimmed leap pane, the hint labels for the current phase, and a status line.
- `src/theme.rs` owns the default TUI theme and user-configurable color parsing.
- `src/config.rs` loads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` (`search_chars`, `mode`,
  `copy_toast`, optional `[style]`).
- `src/herdr_client.rs` is the Herdr Unix-socket JSON-RPC client (`pane.read`, `pane.layout`,
  `notification.show`, plus smart-nav `pane.process_info` / `pane.send_keys` /
  `pane.focus_direction`).
- `src/clipboard.rs` writes OSC 52.
- `src/main.rs` is the thin entry point (`--mode leap|extract|smart-nav`); keep leap/extract logic,
  smart-nav dispatch, theming, clipboard, and socket logic in the library modules. Smart-nav must
  not initialize ratatui.
- `scripts/lab-smart-nav.sh` is the guarded-lab integration proof (named non-default session only).

## Development Process

Use TDD for behavior changes. Add or update failing unit tests describing the intended behavior
first, then implement the smallest change that makes them pass. Keep tests on the pure modules
(`leap`, `extract`, `extract_app`, `smart_nav`, `hints`, `app`, `theme`, `config`) — no socket or
TTY needed. Socket-client request shaping is covered with local UnixListener fixtures in
`herdr_client` tests.

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
herdr plugin action invoke RooseveltAdvisors.herdr-leap.smart-right
herdr plugin config-dir RooseveltAdvisors.herdr-leap
```

Guarded lab (never default session):

```bash
HERDR_LAB_HELPER=/opt/ra/firstmate/bin/fm-herdr-lab.sh ./scripts/lab-smart-nav.sh
```

## Implementation Notes

- Prefer Herdr's socket API over shelling out to `herdr` from the running binary.
- Clipboard writes use OSC 52, not platform clipboard commands. Herdr forwards OSC 52 writes from
  plugin panes to the foreground client.
- A multiplexer cannot move the inner program's cursor, so `mode = "jump"` is realized as
  set-anchor-then-select-and-copy. Document this honestly; do not pretend to reposition a cursor.
- Region rendering must keep the original visible pane lines and must not change line widths.
- Smart-nav personal `ctrl+h/j/k/l` bindings are documentation-only snippets; do not install them,
  commit captain dotfiles, or make them upstream Herdr defaults.
- Copy-mode always-navigate parity is a separate Herdr-core change; do not attempt it in this
  plugin repository.
- Do not commit `target/`, runtime logs, or local editor files.
