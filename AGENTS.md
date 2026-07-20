# Repository Guidelines

`herdr-leap` is a Herdr plugin with a jump/select-copy overlay over the focused pane's **visible**
buffer (`pane.read` `source = "visible"`), plus one-shot smart pane navigation:

1. **Leap (`open` / entrypoint `leap`)** — a schasse/tmux-jump-inspired character jump followed by
   select-to-copy of an arbitrary screen region (await-search → pick-start → pick-end).
2. **Smart nav (`smart-left` / `smart-down` / `smart-up` / `smart-right`)** — one-shot
   vim-tmux-navigator-style dispatch: if the focused pane's foreground process matches the
   editor/fzf predicate, `pane.send_keys` the matching Ctrl chord; otherwise
   `pane.focus_direction` in that direction. No overlay, no TUI.

Leap emits OSC 52 for Herdr to forward. Extract belongs to the separate public
`RooseveltAdvisors.herdr-extractor` plugin; do not restore an extract action or pane here.
Smart-nav must stay a one-shot path (no pane entrypoint).

## Project Shape

- `herdr-plugin.toml` is the Herdr plugin manifest. Keep the plugin id
  (`RooseveltAdvisors.herdr-leap`), action commands, pane commands, and binary name in sync.
  `open` → `scripts/open-leap` → entrypoint `leap`; `smart-*` →
  `./target/release/herdr-leap --mode smart-nav --direction …` (no pane open).
- `Cargo.toml` defines both the crate and the release binary as `herdr-leap`.
- `src/leap.rs` owns the pure leap logic: the `WrappedBuffer` coordinate model, character search
  (smartcase), and **region extraction** (`extract_region`) from anchor to extent. This is the
  load-bearing leap module — keep it covered by unit tests.
- `src/smart_nav.rs` owns pure smart-nav dispatch: direction vocabulary, editor/fzf predicate, and
  `decide()` → passthrough vs focus. Keep it covered by unit tests.
- `src/app.rs` owns the leap state machine (`Phase::AwaitSearch | PickStart | PickEnd`),
  hint input handling, and shared `Outcome` transitions (`Continue`, `Copy(String)`, `Cancel`).
- `src/hints.rs` generates unique, stable hint labels (`a`, `s`, … then fixed-width multi-char).
- `src/ui.rs` renders the dimmed leap pane, the hint labels for the current phase, and a status line.
- `src/theme.rs` owns the default TUI theme and user-configurable color parsing.
- `src/config.rs` loads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` (`search_chars`, `mode`,
  `copy_toast`, optional `[style]`).
- `scripts/open-leap` validates `HERDR_BIN_PATH` and falls back to `herdr` on `PATH`; this is
  required because a replaced running Linux server can expose a stale path ending in ` (deleted)`.
- `src/herdr_client.rs` is the Herdr Unix-socket JSON-RPC client (`pane.read`, `pane.layout`,
  `notification.show`, plus smart-nav `pane.process_info` / `pane.send_keys` /
  `pane.focus_direction`).
- `src/clipboard.rs` writes OSC 52.
- `src/main.rs` is the thin entry point (`--mode leap|smart-nav`); keep leap logic, smart-nav
  dispatch, theming, clipboard, and socket logic in the library modules. Smart-nav must not
  initialize ratatui.
- `scripts/lab-smart-nav.sh` is the guarded-lab integration proof (named non-default session only).

## Development Process

Use TDD for behavior changes. Add or update failing unit tests describing the intended behavior
first, then implement the smallest change that makes them pass. Keep tests on the pure modules
(`leap`, `smart_nav`, `hints`, `app`, `theme`, `config`) — no socket or TTY needed. Socket-client request shaping is covered with local UnixListener fixtures in
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
- Keep the jump lineage credit to `schasse/tmux-jump` in README, LICENSE notes, and manifest
  metadata; do not present the UX as an original invention.
- A multiplexer cannot move the inner program's cursor, so `mode = "jump"` is realized as
  set-anchor-then-select-and-copy. Document this honestly; do not pretend to reposition a cursor.
- Region rendering must keep the original visible pane lines and must not change line widths.
- Smart-nav personal `ctrl+h/j/k/l` bindings are documentation-only snippets; do not install them,
  commit captain dotfiles, or make them upstream Herdr defaults.
- Copy-mode always-navigate parity is a separate Herdr-core change; do not attempt it in this
  plugin repository.
- Do not commit `target/`, runtime logs, or local editor files.

## Maintaining this file

Update this file only for durable repository-wide guidance. Prefer pointers to authoritative files
and commands over duplicating implementation details, and remove stale guidance when behavior
moves to another repository.
