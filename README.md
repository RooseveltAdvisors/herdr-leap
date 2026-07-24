# herdr-leap

A [Herdr](https://herdr.dev) plugin for a **one-pick tmux-jump-style word-start jump** into the
invoking pane's copy mode, plus Vim/fzf-aware smart pane navigation.

## Lineage and credit

The jump workflow is based on the UX established by
[schasse/tmux-jump](https://github.com/schasse/tmux-jump), described by its authors as
“Vimium/Easymotion like navigation for tmux.” This Herdr port is not presented as an original UX
invention. It adapts that lineage to Herdr's visible-buffer and plugin-pane APIs.

Herdr cannot move a cursor inside an arbitrary program. Leap instead places Herdr's copy-mode
cursor through `pane.copy_mode_jump`; it does not inject keys into the child program.

## Actions

| Action | Behavior |
|---|---|
| `RooseveltAdvisors.herdr-leap.open` | Open the one-pick word-start copy-mode jump popup |
| `RooseveltAdvisors.herdr-leap.smart-{left,down,up,right}` | One-shot Vim/fzf-aware pane navigation |

Token extraction has moved to the separate
[`RooseveltAdvisors/herdr-extractor`](https://github.com/RooseveltAdvisors/herdr-extractor)
plugin. `RooseveltAdvisors.herdr-leap.extract` and the `extract` pane entrypoint were removed in
v0.2.0. Bind `prefix+space` to `RooseveltAdvisors.herdr-extractor.extract`.

## How Leap works

1. A full-size popup reads the focused pane's unchanged **visible** buffer.
2. Type one search character. Smartcase word starts receive hints.
3. Type one hint. Herdr enters copy mode with its cursor at that captured cell.

The jump carries the visible read's revision and scroll offset. A stale viewport is refreshed only
when its text and scroll position are unchanged; otherwise Leap refuses to drift. `Esc` or `Ctrl-C`
cancels without touching the clipboard. Optional `mode = "select"` retains the two-pick OSC 52
region-copy flow.

## Install

```bash
herdr plugin install RooseveltAdvisors/herdr-leap
herdr server reload-config
```

Recommended jump binding:

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.open"
description = "tmux-jump-style copy-mode cursor"
```

The action launcher validates `HERDR_BIN_PATH` before using it and falls back to `herdr` from
`PATH`. This keeps actions working when Linux reports a replaced running server executable as a
path ending in ` (deleted)`.

## Optional smart pane navigation

Smart navigation stays in this plugin, but it is independent of the `prefix+f` jump workflow. Each
action reads `pane.process_info`: Vim/Neovim/fzf receives the matching `Ctrl-h/j/k/l`; other
processes trigger `pane.focus_direction`. These direct bindings are personal configuration, not
plugin-installed or Herdr defaults:

```toml
[[keys.command]]
key = "ctrl+h"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.smart-left"
description = "smart pane left"

[[keys.command]]
key = "ctrl+j"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.smart-down"
description = "smart pane down"

[[keys.command]]
key = "ctrl+k"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.smart-up"
description = "smart pane up"

[[keys.command]]
key = "ctrl+l"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.smart-right"
description = "smart pane right"
```

Copy-mode navigation parity is a Herdr-core concern and is not implemented by this plugin.

## Configuration

Create `config.toml` under `herdr plugin config-dir RooseveltAdvisors.herdr-leap`:

```toml
search_chars = 1
mode = "jump" # default; "select" enables two-pick region copy
copy_toast = true

[style]
hint_bg = "yellow"
hint_fg = "black"
```

## Development

```bash
cargo fmt -- --check
cargo test
cargo build --release --locked
cargo clippy --all-targets -- -D warnings
```

The guarded smart-nav lab requires a generated, named non-default Herdr session:

```bash
HERDR_LAB_HELPER=/opt/ra/firstmate/bin/fm-herdr-lab.sh ./scripts/lab-smart-nav.sh
```

## License

MIT — see [LICENSE](LICENSE). The license file also records the tmux-jump lineage acknowledgement.
