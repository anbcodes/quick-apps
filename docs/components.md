# Built-in components

Every component is native code in the runtime (`crates/qa/src/ui/`). Apps put
them together and never draw them. A component's look comes only from **theme
tokens** ([theming.md](theming.md)), picked through its `variant`. Prop ids
start at 20 within each kind. The common props 1–19 are in
[ui-tree.md](ui-tree.md).

Tokens are listed as fallback chains: the first one found wins. `V` stands for
the node's `variant`.

## grid (1)

The only layout container ([layout.md](layout.md)).

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `cols` | tracks | `[fr(1)]` |
| 21 | `rows` | tracks | all `auto` |
| 22 | `gap` | f32 | token `space.gap` |
| 23 | `clickable` | i32 | 0. When 1: focusable, shows a hover/press tint, sends `Click`. Use it for list rows. |
| 24 | `selected` | i32 | 0. When 1: draws the selection background. |
| 25 | `flow` | i32 | 0. When 1: `cols`, `rows` and child positions are ignored, and children are placed left to right at their preferred width, wrapping onto new lines. `gap` spaces both ways. Each line is as tall as its tallest child; `align_y` places a child within it. Use it for chips, tags and toolbars. |

Tokens: bg `surface.V.bg`; border `surface.V.border`; radius
`surface.V.radius` → `radius`; hover `color.hover`; press `color.press`;
selected `color.selection`. With an empty variant, nothing is drawn.

## text (2)

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `text` | str | "" |
| 21 | `wrap` | i32 | 0 word-wrap · 1 one line + ellipsis · 2 one line, clipped |
| 22 | `style` | str | `"body"`; also `title`, `heading`, `caption`, `mono` |
| 23 | `text_align` | i32 | 0 start · 1 center · 2 end |
| 24 | `selectable` | i32 | 1: drag to select, double-click for a word, triple-click for all of it, Ctrl+C to copy. 0 turns it off. |
| 25 | `max_lines` | i32 | 0 (no limit). An ellipsis ends the last line. |
| 26 | `spans` | bytes | Styled ranges: repeated `u32 start, u32 end, u8 flags, str variant`, with byte offsets into `text`. |

Span flags: 1 bold (synthetic, so widths and line breaks don't change),
2 underline, 4 code (a `color.code_bg` → `color.surface` background),
8 strikethrough, 16 link
(`color.link` → `color.accent` and underlined; a click sends `Link`). Where
spans overlap, flags combine and the last non-empty `variant` wins. The
variant names a color token like the node's own (`accent` → `color.accent`).

Selection stays inside one text node and is drawn with `color.selection`.
A press on text inside a clickable grid or a link still clicks unless the
pointer moves more than 4 px first, in which case it selects instead. Ctrl+C
copies the selection unless an input has focus; the app doesn't see that key.

Tokens: size `font.<style>.size`; weight `font.<style>.weight`; family
`font.<style>.family` → `font.family`; color `color.V` (e.g. `muted`,
`danger`, `accent`) → `color.fg`.

## image (3)

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `src` | str | `"assets/x.png"` (in the bundle) or `"app:path"` (in the app folder) |
| 21 | `data` | bytes | PNG bytes. Used instead of `src`. |
| 22 | `fit` | i32 | 0 contain · 1 cover · 2 fill · 3 natural (1 image px = 1 logical px) |

v1 supports PNG only. Decoded pixels count toward the app's memory budget.

## canvas (4)

A pixel buffer the app fills with `canvas_put`. Use it for charts, games and
anything custom.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `px_w` | i32 | 0 (buffer width in pixels) |
| 21 | `px_h` | i32 | 0 |
| 22 | `fit` | i32 | as for image |
| 23 | `focusable` | i32 | 0. When 1: gets `Key` events while focused. |

Events: `Pointer`. The buffer (`px_w*px_h*4` bytes) counts toward the memory
budget.

## scroll (5)

Has exactly one child.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `axis` | i32 | 0 vertical · 1 horizontal · 2 both |
| 21 | `stick_end` | i32 | 0. When 1: stays pinned to the end while content grows (logs, chat). |

Events: `NearStart`, `NearEnd`. Tokens: scrollbar `color.scrollbar` →
`color.border`. The scrollbar is a thin overlay that shows only while
scrolling or on hover.

- **Anchoring.** When content changes size above what is on screen (rows
  inserted at the top, an image loading, a row growing), the view stays on
  the same rows: the runtime keeps the row a quarter of the way down the
  view at the same place. A chat can insert older messages at the top on
  `NearStart` without moving what the user is reading. `stick_end` wins
  while the view is at the end.
- **Wheel and touchpad.** A mouse wheel notch scrolls 56 px, animated over a
  few frames. Touchpad scrolling follows the fingers, and a flick keeps
  scrolling and slows down.

## button (10)

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `label` | str | "" |
| 21 | `disabled` | i32 | 0 |

Variants: `""`, `primary`, `danger`, `flat`. Events: `Click`. Tokens: bg
`button.V.bg` → (`primary`: `color.accent`, `danger`: `color.danger`, other:
`color.surface`); fg `button.V.fg` → (`color.accent_fg` / `color.danger_fg` /
`color.fg`); border `button.V.border` → `color.border`; radius `radius`;
padding `space.pad`.

## input (11)

A text field with readline keys, selection, undo and clipboard. Pasting into it
never prompts.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `value` | str | "". Replaces the text only if it differs. The cursor is clamped, or goes to the end if the input was empty. |
| 21 | `placeholder` | str | "" |
| 22 | `multiline` | i32 | 0 |
| 23 | `secret` | i32 | 0 (shows dots) |
| 24 | `disabled` | i32 | 0 |
| 25 | `lines` | i32 | 4 (visible lines when multiline; grows up to `max_h`) |

Events: `Change`, `Submit`, `FocusOut`. Tokens: bg `input.bg` →
`color.surface`; border `input.border` → `color.border`; focused border
`color.focus`; placeholder `color.placeholder` → `color.muted`.

## checkbox (12)

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `label` | str | "" |
| 21 | `checked` | i32 | 0 |
| 22 | `disabled` | i32 | 0 |
| 23 | `wrap` | i32 | 0 wraps the label onto more lines · 1 one line + ellipsis |

It flips its own state right away and sends `Toggle`. The app can `SET
checked` to override. Events: `Toggle`, `DoubleClick`. Tokens: box
`color.border`; checked fill `color.accent`; check mark `color.accent_fg`;
label `color.V` → `color.fg` (so `variant("muted")` dims a finished item).

## select (13)

A drop-down. The list opens in a runtime popup.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `options` | str | "" (one option per line, `\n`-separated) |
| 21 | `selected` | i32 | -1 (none) |
| 22 | `placeholder` | str | "" |
| 23 | `disabled` | i32 | 0 |

Events: `Select`. Tokens are the same as for input.

## tabs (14)

Draws only the tab strip. The app shows and hides the pages with `hidden`.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `labels` | str | "" (`\n`-separated) |
| 21 | `selected` | i32 | 0 |

Events: `Select`. Tokens: active `color.accent`; inactive text `color.muted`.

## progress (15)

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `value` | f32 | 0..1, or < 0 for indeterminate (animated) |

Tokens: track `color.surface`; bar `color.accent`. An indeterminate bar is the
only built-in animation, and it asks for frames only while it is visible.

## divider (16)

A 1-px line. Its direction follows its cell: wider than tall means horizontal.
Token: `color.border`.

## popup (17)

Shown over everything else while open, next to its anchor. A popup is never
placed in a container: create it detached and give it one child (a `menu`, or
any grid of nodes, such as a picker or a small form).

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `open` | i32 | 0. 1 shows it. The runtime sets it back to 0 when it closes the popup. |
| 21 | `anchor` | i32 (node id) | 0: at the pointer when it opened |
| 22 | `placement` | i32 | 0 below the anchor · 1 above · 2 right · 3 left. It flips to the other side when there is no room, and stays 6 px inside the window. |

The common `pad` prop defaults to 4. A popup placed below or above its
anchor is at least as wide as the anchor.

While a popup is open it takes the pointer and the keyboard: focus moves to
its first focusable node (and back when it closes), Tab stays inside it, and
nothing outside it hovers or scrolls. **Esc** or a press outside closes the
topmost popup and sends `Dismiss`. The press that closes it does nothing
else, except a right-click, which goes on to open the context menu under it.
A popup opened from inside another (a submenu) stacks on top; closing one
closes those above it.

Open one with `open` (the SDK's `ui.open(popup, anchor)`), or give any node a
`context_menu` ([ui-tree.md](ui-tree.md)): a right-click on that node or
inside it, or the Menu key or Shift+F10 while it has focus, opens the popup
at the pointer and sends `ContextMenu` naming the node, so the app can fill
in the popup for it.

Tokens: bg `popup.bg` → `color.bg`; border `popup.border` → `color.border`;
radius `popup.radius` → `radius`; shadow `popup.shadow`.

## menu (18)

A list of choices, usually the child of a popup.

| Id | Prop | Type | Default |
|---|---|---|---|
| 20 | `items` | str | One item per line. `-` is a separator. A leading `!` disables an item and a leading `*` checks it (✓). Text after a tab is a shortcut hint, drawn on the right. |

Choosing an item (click, or Enter/Space) sends `Select` with the item's line
index (separators count) and closes the popups it is in. The pointer and
Up/Down highlight items, Home/End go to the first and last, and a letter
jumps to the next item starting with it. Disabled items and separators are
skipped.

Tokens: highlight `menu.highlight` → `color.selection`; hint and disabled
text `color.muted`; check `color.accent`.

## Shared behavior

- **Focus.** Interactive components and `clickable` grids are focusable in
  tree order. The focus ring uses `color.focus`.
- **Disabled.** A disabled node is drawn at 50% alpha, gets no focus and sends
  no events. The common `disabled` prop ([ui-tree.md](ui-tree.md)) works on
  any node and covers everything inside it; the per-kind `disabled` props
  above still work.
- **Double-click.** Buttons, checkboxes and clickable grids send
  `DoubleClick` after the second of two quick clicks.
- **Cursor shapes.** I-beam over inputs and selectable text, pointer over
  buttons and clickable grids.

## Not in v1

These are list virtualization (use `NearEnd` and append rows), tables with
sortable headers, dialogs (build them from a popup, or grids and `hidden`), rich
text spans, icons, and sliders. Each one is a candidate for a later native
component.
