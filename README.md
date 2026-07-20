# herdr-leap

A [Herdr](https://herdr.dev) plugin for **tmux-jump-style character hints** followed by
select-to-copy of an arbitrary visible screen region.

## Lineage and credit

The jump workflow is based on the UX established by
[schasse/tmux-jump](https://github.com/schasse/tmux-jump), described by its authors as
“Vimium/Easymotion like navigation for tmux.” This Herdr port is not presented as an original UX
invention. It adapts that lineage to Herdr's visible-buffer and plugin-pane APIs.

Herdr cannot move the cursor inside an arbitrary program. After a hint chooses the anchor,
herdr-leap therefore asks for an extent and copies the inclusive region via OSC 52. It does not
pretend to reposition the underlying application's cursor.

## Actions

| Action | Behavior |
|---|---|
| `RooseveltAdvisors.herdr-leap.open` | Open the jump/select-copy overlay |
| `RooseveltAdvisors.herdr-leap.smart-{left,down,up,right}` | One-shot Vim/fzf-aware pane navigation |

Token extraction has moved to the separate
[`RooseveltAdvisors/herdr-extractor`](https://github.com/RooseveltAdvisors/herdr-extractor)
plugin. `RooseveltAdvisors.herdr-leap.extract` and the `extract` pane entrypoint were removed in
v0.2.0. Bind `prefix+space` to `RooseveltAdvisors.herdr-extractor.extract`.

## How jump/select-copy works

1. The overlay reads only the focused pane's **visible** buffer and dims it.
2. Type one search character. Lowercase searches match either case; uppercase is exact-case.
3. Type a displayed hint to set the anchor.
4. Type a second hint to set the extent. The inclusive region is copied with OSC 52.
5. `Backspace` returns from extent selection to anchor selection. `Esc` or `Ctrl-C` cancels.

Rows that filled the visible pane width are treated as soft wraps and rejoined without a newline.
Hard line boundaries remain newlines. Reversed selections are normalized automatically.

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
description = "tmux-jump-style select-copy"
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
mode = "select" # or "jump"; both select and copy because Herdr cannot move an inner cursor
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
