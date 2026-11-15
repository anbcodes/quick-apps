# Events

The runtime delivers one event per `qa_event(ptr, len)` call. The buffer is a
`u8` type followed by its fields, encoded as in [app-abi.md](app-abi.md). Events
are queued and delivered in order. While the app is busy, `Resize` and
`Pointer` move events coalesce (only the latest is kept).

| Type | Name | Fields | Sent when |
|---|---|---|---|
| 1 | `Resize` | `f32 w, f32 h, f32 scale` | Window size or scale changed. The tree relayouts by itself, so this is only needed for layouts that switch on size. |
| 2 | `Click` | `u32 node` | A button is activated by mouse, Enter or Space |
| 3 | `Change` | `u32 node, str value` | An input's text changed (on every edit) |
| 4 | `Submit` | `u32 node, str value` | Enter in a single-line input, or Ctrl+Enter in a multiline one |
| 5 | `Toggle` | `u32 node, u8 checked` | A checkbox was toggled |
| 6 | `Select` | `u32 node, i32 index` | A `select` or `tabs` choice changed, or a `menu` item was chosen (its line index) |
| 7 | `Key` | `u32 focused_node, u32 keysym, u8 mods, str text` | A key no built-in used (see below) |
| 8 | `Pointer` | `u32 node, u8 kind, f32 x, f32 y, u8 buttons, f32 dx, f32 dy` | Canvas only. Kind: 0 down, 1 up, 2 move, 3 wheel, 4 leave. x/y are canvas pixels. |
| 9 | `Timer` | `u32 id` | |
| 10 | `Http` | `u32 req, u16 status, str headers, bytes body` | `status` 0 means a failure, and the body is the error text |
| 11 | `Ws` | `u32 ws, u8 kind, bytes data` | Kind: 0 open, 1 text, 2 binary, 3 closed, 4 error |
| 12 | `Message` | `str from, bytes data` | Another app sent a message. `from` is its manifest URL. |
| 13 | `RequestDone` | `u32 req, i32 result, str info` | See [permissions.md](permissions.md) |
| 14 | `Close` | — | The user closed the window. The app has 2 s of CPU to save, then exits. |
| 15 | `Activate` | — | `qa run` was called again for this app, or the app was launched from its `.desktop` file |
| 16 | `FocusOut` | `u32 node` | An input lost focus (handy for "save on blur") |
| 17 | `NearEnd` | `u32 node` | A `scroll` came within one viewport of its end, so the app can load more rows. It fires again after the content grows or the view moves back toward the start. |
| 18 | `Link` | `u32 node, u32 span` | A link span in a text node was clicked. `span` is the span's index in the node's `spans`. |
| 19 | `DoubleClick` | `u32 node` | The second of two clicks within 400 ms on the same button, checkbox or clickable grid. It follows that click's own `Click` or `Toggle`, so a checkbox has toggled twice by then. |
| 20 | `NearStart` | `u32 node` | A `scroll` came within one viewport of its start, so the app can insert older rows at the top (the view stays put). It fires again after the content grows or the view moves away from the start. |
| 21 | `ContextMenu` | `u32 node, f32 x, f32 y` | A right-click (or the Menu key) on a node with a `context_menu`; `x`/`y` are window coordinates. The popup is already opening: change its content now. |
| 22 | `Dismiss` | `u32 popup` | The runtime closed a popup: Esc, a press outside it, or a menu choice. Not sent when the app closes it. |

Every event with a `node` field ends with one more field, `i32 tag`: the
node's `tag` prop (0 if unset). Decoders that stop reading early can ignore
it. `open_url` is allowed while handling `Link`.

`mods` bits: 1 Shift, 2 Ctrl, 4 Alt, 8 Super.

## Keys

The runtime handles keys in this order:

1. **Runtime keys**: an open prompt or overlay takes all input. `Ctrl+Q`
   closes the window. Esc closes the topmost open popup, and the Menu key or
   Shift+F10 opens the focused node's `context_menu`.
2. **The focused built-in.** An input uses text keys, arrows and editing
   shortcuts; a button uses Enter and Space; a menu uses arrows, Enter,
   Space and letters; a scroll uses PageUp, PageDown, Home and End.
3. **Focus traversal**: Tab and Shift+Tab move between focusable nodes in tree
   order.
4. **Everything else** goes to the app as `Key`, with `focused_node` (or 0).
   This is how apps add shortcuts, such as `j`/`k` in a list.

Keysyms are X11/xkb keysyms (`0xff0d` Return, `0xff1b` Escape, `'a'` = 0x61 …).
`qa-sdk` names the common ones.
