# herdr-leap

EasyMotion / leap-style **character jump + select-to-copy of an arbitrary screen region**, plus a
**visible-buffer token extractor**, for [Herdr](https://herdr.dev).

Two workflows share one plugin:

| Action | Intent |
|--------|--------|
| `RooseveltAdvisors.herdr-leap.open` | Hint **any character**, then copy the **arbitrary region** between two points |
| `RooseveltAdvisors.herdr-leap.extract` | List **copy-eligible tokens** (URLs, paths, quotes, words) from the **visible** pane and copy one |

Where copy plugins like `herdr-tiny-fingers` or `pluck` only hint detected tokens, **leap** still
covers any character span. **extract** is the extrakto-style companion for grabbing a token without
walking a region.

## How leap works (`open`)

1. **Await search** — the overlay shows the focused pane's visible content, dimmed. You type **one
   search character**.
2. **Pick start** — every occurrence of that character is labeled with a short hint (`a`, `s`, `d`,
   … then two-char labels). Matching is **smartcase**: a lowercase search char matches both cases,
   an uppercase search char matches only uppercase. Type a label to set the **anchor**.
3. **Pick end** — the matches are re-labeled from the anchor. Type a label to set the **extent**.
   The character region from anchor to extent (inclusive) is copied to your clipboard. `Backspace`
   returns to *pick start*; `Esc` / `Ctrl-C` cancels.
4. A `Copied: <preview>` toast is shown (when `copy_toast` is enabled).

## How extract works (`extract`)

1. The overlay reads **only** the focused pane's **visible** buffer (`pane.read` `source=visible`).
   Rows that fill the focused pane's wrap width are rejoined so tokens split by terminal soft wraps
   remain copyable; shorter rows preserve their hard line boundaries.
2. Tokens are collected with a bounded extrakto-parity set: **url**, **path**, double/single
   **quote**, and **word** (min length 5). Results are reversed (prefer lower/more-recent screen
   content) and deduped.
3. A typeahead list filters as you type. `↑`/`↓` (or `Ctrl-p`/`Ctrl-n`) move the selection.
   `Enter` copies the exact selected string; `Esc` / `Ctrl-C` cancels.

Both actions copy via an **OSC 52** clipboard write, which Herdr forwards from the plugin pane to
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

Herdr keybindings live in the user's Herdr config, not in the plugin manifest. Recommended bindings:

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.open"
description = "leap: jump + select-copy"

[[keys.command]]
key = "prefix+space"
type = "plugin_action"
command = "RooseveltAdvisors.herdr-leap.extract"
description = "extract: copy visible tokens"
```

Do not bind both keys to `open` — that collapses extract into the leap UI. Invoke extract directly
while developing with:

```bash
herdr plugin action invoke RooseveltAdvisors.herdr-leap.extract
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
