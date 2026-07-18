# Repository Guidelines

`herdr-leap` is a Herdr plugin that provides EasyMotion/leap-style **character jump + select-to-copy
of an arbitrary screen region**. It opens an overlay pane, reads the previously focused Herdr pane
via `pane.read` with `source = "visible"`, and drives a three-state machine
(await-search → pick-start → pick-end) that labels character matches and copies the region between
two chosen points by emitting an OSC 52 clipboard sequence for Herdr to forward.

## Project Shape

- `herdr-plugin.toml` is the Herdr plugin manifest. Keep the plugin id
  (`RooseveltAdvisors.herdr-leap`), action command, pane command, and binary name in sync.
- `Cargo.toml` defines both the crate and the release binary as `herdr-leap`.
- `src/leap.rs` owns the pure leap logic: the `WrappedBuffer` coordinate model, character search
  (smartcase), and **region extraction** (`extract_region`) from anchor to extent. This is the
  load-bearing module — keep it covered by unit tests.
- `src/app.rs` owns the interactive state machine (`Phase::AwaitSearch | PickStart | PickEnd`),
  hint input handling, and `Outcome` transitions (`Continue`, `Copy(String)`, `Cancel`).
- `src/hints.rs` generates unique, stable hint labels (`a`, `s`, … then fixed-width multi-char).
- `src/ui.rs` renders the dimmed pane, the hint labels for the current phase, and a status line.
- `src/theme.rs` owns the default TUI theme and user-configurable color parsing.
- `src/config.rs` loads `$HERDR_PLUGIN_CONFIG_DIR/config.toml` (`search_chars`, `mode`,
  `copy_toast`, optional `[style]`).
- `src/herdr_client.rs` is the Herdr Unix-socket JSON-RPC client. `src/clipboard.rs` writes OSC 52.
- `src/main.rs` is the thin TUI entry point; keep leap logic, theming, clipboard, and socket logic
  in the library modules.

## Development Process

Use TDD for behavior changes. Add or update failing unit tests describing the intended behavior
first, then implement the smallest change that makes them pass. Keep tests on the pure modules
(`leap`, `hints`, `app`, `theme`, `config`) — no socket or TTY needed.

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
herdr plugin config-dir RooseveltAdvisors.herdr-leap
```

## Implementation Notes

- Prefer Herdr's socket API over shelling out to `herdr` from the running TUI.
- Clipboard writes use OSC 52, not platform clipboard commands. Herdr forwards OSC 52 writes from
  plugin panes to the foreground client.
- A multiplexer cannot move the inner program's cursor, so `mode = "jump"` is realized as
  set-anchor-then-select-and-copy. Document this honestly; do not pretend to reposition a cursor.
- Region rendering must keep the original visible pane lines and must not change line widths.
- Do not commit `target/`, runtime logs, or local editor files.
