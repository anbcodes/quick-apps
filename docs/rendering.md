# Rendering

```
tree ──layout──▶ rects ──paint──▶ display list ──rasterizer──▶ pixel buffer ──platform──▶ screen
```

## Display list

Painting walks the tree, skips subtrees that miss the damage region, and
appends commands. Coordinates are in physical pixels, already multiplied by
the scale.

| Command | Fields |
|---|---|
| `Fill` | rect, color, corner radius |
| `Stroke` | rect, color, corner radius, width |
| `Glyphs` | font, size px, color, baseline origin, `[(glyph_id, x)]` |
| `Image` | image id, source rect, destination rect (bilinear) |
| `Pixels` | canvas id, destination rect |
| `Clip` / `Unclip` | rect, corner radius |

This list is the **seam between the UI and the backend**. Anything that can
draw these six commands can render Quick Apps.

## Rasterizer (CPU)

- The target is premultiplied ARGB8888, the `wl_shm` format, blended with
  source-over in sRGB.
- Rounded rects use analytic coverage, from the distance to the corner circle.
- Glyphs are rasterized from their outlines (quadratic and cubic) with a
  signed-area accumulation scanline rasterizer. The A8 bitmaps go into an LRU
  cache (about 4 MB) keyed by `(font, glyph, size_px, x_subpixel/4)`.
- Every command is clipped to the current damage rect, so pixels outside the
  damage are never touched.

## Damage

- A node whose paint changed adds its **old and new** rects to the damage.
- If there are more than 8 rects, or their bounding box is under twice their
  total area, they merge into that bounding box.
- The window uses two buffers. The buffer being drawn into is two frames old,
  so the runtime redraws `damage(this frame) ∪ damage(previous frame)`.

## Text

- Shaping is simple: `cmap` → glyph, `hmtx` advances, and `kern`-table pair
  kerning (GPOS kerning is not read yet). There are no ligatures, no bidi and
  no complex scripts in v1.
- A glyph missing from the style's font comes from the first fallback font
  that has it. `font.fallback` lists them, comma-separated (file names or
  paths); the default list tries DejaVu Sans, Noto Sans, the Noto symbol and
  math fonts, Noto Emoji (monochrome), Symbola, Noto CJK, Droid Sans Fallback
  and Unifont. Missing fonts and fonts with only color bitmaps are skipped.
- Text spans ([components.md](components.md)) split a line into runs of
  glyphs, one per style. Bold is synthetic (thicker outlines), so it never
  changes widths or line breaks; underlines and code backgrounds are fills
  drawn with the run.
- Lines break at spaces and after hyphens, and anywhere in CJK text. A word
  longer than the line breaks at the last character that fits.
- Measurements are cached per `(text node, width)`.

## Wayland platform (v1)

The Wayland client is hand-written over `libc`, speaking the wire protocol
directly. It uses these protocols:

| Protocol | Use |
|---|---|
| `wl_compositor`, `wl_shm` (memfd) | Surfaces and pixel buffers |
| `xdg_wm_base` / `xdg_toplevel` | The window, min size, move and resize |
| `zxdg_decoration_manager_v1` | Server-side decorations where offered. Otherwise (e.g. GNOME) the runtime draws a 32-px title bar with title and close. |
| `wl_seat` (pointer, keyboard) | Input. The keymap goes through dlopen'd `libxkbcommon`. Key repeat follows `repeat_info` using a timerfd. |
| `wl_data_device_manager` | Clipboard (`text/plain;charset=utf-8`) |
| `wp_fractional_scale_v1` + `wp_viewporter` | Crisp fractional scaling. Integer `wl_output` scale is the fallback. |
| `wp_cursor_shape_v1` | Cursors without loading a cursor theme. If it is missing, the compositor's default is used. |

A frame callback is requested only while something is animating (smooth
scrolling, an indeterminate progress bar). The text caret does not blink, so a
focused input costs nothing while idle. Otherwise the
process sleeps in `poll()`.

## macOS platform

The Mac window is an `NSWindow` with one layer-backed `NSView`, driven through a
small Objective-C file (`platform/macos/shim.m`) that `build.rs` compiles with
`clang`. Rust sees plain C functions and a queue of event structs, never an
Objective-C object. There are no crates.

- **Drawing.** The frame is the runtime's own ARGB buffer. `present` copies it
  into a `CGImage` and sets it as the layer's contents, so the system draws the
  title bar, shadow and fullscreen. The window is Retina-aware: sizes are in
  physical pixels and the scale is `backingScaleFactor`.
- **Waiting.** Window events can't be polled with a file descriptor, so
  `Platform::wait` replaces the `poll()` call. The Mac version registers the
  loop's descriptors as `CFFileDescriptor` run loop sources and blocks in
  `nextEventMatchingMask`; when a descriptor becomes readable its callback posts
  an application-defined event to wake it. A frame timer (16 ms) stands in for
  Wayland's frame callback.
- **Keys.** Command acts as Ctrl, so Cmd+C, V, X, A and Z work as they do on
  Linux. Option+arrow moves by word, Cmd+Left and Cmd+Right go to the ends.
  Cmd+Q and Cmd+W send the same close request as the red button. Text comes from
  `NSEvent.characters`, so dead keys and input methods (which need
  `NSTextInputClient`) aren't supported yet.
- **Waking from other threads.** `sys::new_wake` is an `eventfd` on Linux and a
  kqueue with an `EVFILT_USER` event on macOS. Both poll as one descriptor.
- **Known gaps.** While the user drags a window edge, Cocoa runs its own loop and
  the window isn't redrawn until the drag ends.

## Platform trait

```rust
pub trait Platform {
    fn fd(&self) -> RawFd;                          // polled by the main loop
    fn wait(&mut self, extra: &[RawFd], ms: i32);   // sleep until input, an extra fd or the timeout
    fn dispatch(&mut self, out: &mut Vec<PEvent>);  // input, resize, close, frame done, paste
    fn frame(&mut self) -> Frame<'_>;               // &mut [u32] pixels, w, h, stride, age
    fn present(&mut self, damage: &[Rect]);
    fn set_title(&mut self, title: &str);
    fn set_cursor(&mut self, cursor: Cursor);
    fn clipboard_set(&mut self, text: String);
    fn clipboard_get(&mut self);                    // answered with PEvent::Paste
    fn begin_move(&mut self);
    fn begin_resize(&mut self, edge: Edge);
}
```

## Changing backends later

The parts that are hard to change are the **ABI, the patch format, layout
behavior and token names**. Keep those stable. Backends are cheap to swap:

| Change | Work |
|---|---|
| Add X11 | A new `Platform` using XShm or `PutImage`. About 1k lines, and nothing else changes. |
| Add macOS / Windows | A new `Platform` that blits the CPU buffer (CGImage / StretchDIBits) |
| GPU rendering | A new rasterizer that turns the display list into quads plus a glyph atlas. `Platform::frame` gains a GPU-surface variant. This is the largest of these changes. |
| Terminal backend (mtui-style) | A rasterizer that maps glyph runs to cells and fills to cell backgrounds, plus layout units snapped to cells. It is lossy (no images or canvas without kitty graphics) but possible. |
