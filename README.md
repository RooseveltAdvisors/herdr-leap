# herdr-leap

EasyMotion / leap-style **character jump + select-to-copy of an arbitrary screen region** for
[Herdr](https://herdr.dev).

Where copy plugins like `herdr-tiny-fingers` or `pluck` hint only *detected tokens* (URLs, paths,
SHAs), **herdr-leap hints any character you type** and lets you copy an **arbitrary region** of the
visible screen — the span between two hinted points.

## How it works

1. **Await search** — the overlay shows the focused pane's visible content, dimmed. You type **one
   search character**.
2. **Pick start** — every occurrence of that character is labeled with a short hint (`a`, `s`, `d`,
   … then two-char labels). Matching is **smartcase**: a lowercase search char matches both cases,
   an uppercase search char matches only uppercase. Type a label to set the **anchor**.
3. **Pick end** — the matches are re-labeled from the anchor. Type a label to set the **extent**.
   The character region from anchor to extent (inclusive) is copied to your clipboard. `Backspace`
   returns to *pick start*; `Esc` / `Ctrl-C` cancels.
4. A `Copied: <preview>` toast is shown (when `copy_toast` is enabled).

The region is copied via an **OSC 52** clipboard write, which Herdr forwards from the plugin pane to
the foreground client (same mechanism `herdr-tiny-fingers` uses — no `pbcopy`/`wl-copy`/`xclip`
shelling required).

### Region semantics (the load-bearing behavior)

The visible buffer is modeled as **wrapped rows** at the pane width (the same coordinate model
`herdr-tiny-fingers` uses). Anchor and extent are `(visual_row, col)` positions in that buffer.

- **Single row:** the inclusive column span of that row.
- **Multiple rows:** the tail of the anchor row, the full intervening rows, and the head of the
  extent row up to and including the extent column — reconstructed as the **real text**. Rows that
  were **soft-wrapped** from one logical line are re-joined with no separator; rows separated by a
  **hard line break** are joined with `\n`.
- **Reversed selection:** if you label the extent above/before the anchor, the region is normalized
  automatically.

### On "jump" mode

A terminal multiplexer cannot move the *inner* program's cursor. So `mode = "jump"` is realized
honestly as: set the anchor and immediately proceed to select-and-copy (identical to `mode =
"select"`, the default). herdr-leap copies a region; it does not reposition the underlying app's
cursor.

## Install

```bash
herdr plugin install RooseveltAdvisors/herdr-leap
herdr server reload-config
```

Or link a local checkout for development:

```bash
cargo build --release --locked
herdr plugin link .
herdr server reload-config
herdr plugin action invoke RooseveltAdvisors.herdr-leap.open
```

## Keybinding

Herdr keybindings live in the user's Herdr config, not in the plugin manifest. Recommended binding
(e.g. `prefix+f`):

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.open"
description = "leap: jump + select-copy"
```

You can bind any key you like — `prefix+space` is another common choice for a leap-style motion:

```toml
[[keys.command]]
key = "prefix+space"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.open"
description = "leap: jump + select-copy"
```

## Configuration

Optional `config.toml` in the plugin config directory
(`herdr plugin config-dir RooseveltAdvisors.herdr-leap`):

```toml
# Number of search characters to type before labeling (MVP supports 1).
search_chars = 1
# "select" (default) or "jump" (see "On jump mode" above — both select-and-copy).
mode = "select"
# Show a "Copied: <preview>" toast after copying.
copy_toast = true

# Optional label colors (named colors or #RRGGBB).
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

## License

MIT — see [LICENSE](LICENSE).
