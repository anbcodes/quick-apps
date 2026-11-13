# Architecture

## Process model

There is **one OS process per running app**. `qa run <app>` starts a process
that holds the runtime and exactly one app instance. This gives:

- Crash isolation and plain OS accounting (`top` shows each app).
- An easy kill: closing the window ends the process.
- Shared memory without extra work: the runtime binary and mmap'd font files
  are shared between processes by the OS page cache.

**Single instance.** Every app listens on
`$XDG_RUNTIME_DIR/quick-apps/<app-id>.sock`. When `qa run` finds a live socket
for the same app, it sends `activate` (which raises the window) and exits. The
same socket carries messages between apps (see [io.md](io.md)).

## Threads

```
main thread ─ poll() loop:  Wayland+key-repeat (epoll fd) │ eventfds (app/io wakeups) │ inotify (themes) │ app socket
   │  owns: platform, UI tree, layout, rasterizer, prompts
   │
   ├─ app thread ─ runs wasm (wasmi). Blocks on an event channel, with a timeout for the next app timer.
   │     event → qa_event() → patches collected → sent back to main → applied
   │
   └─ io threads ─ one per HTTP request / WebSocket (blocking client), bounded pool
```

- Wasm runs off the main thread, so a slow or looping handler never freezes
  the window. The main thread keeps drawing the last committed tree and can
  show "not responding: Wait / Stop".
- Patches are **atomic per event**. Every `ui_patch` call made during one
  `qa_event` is buffered, then applied together once the call returns. The UI
  never shows a half-applied update.
- Host imports that need the main thread (prompts, clipboard) post a request
  and either block the app thread (memory growth) or return a request id whose
  result arrives later as an event.

## Frame pipeline

The main thread does this only when something is dirty. There is no
continuous render loop.

1. **Apply patches.** Update nodes, mark them and their ancestors as needing
   layout, and mark paint-dirty rects.
2. **Layout.** Re-measure and re-arrange only dirty subtrees
   ([layout.md](layout.md)).
3. **Build display list.** Covers only nodes that touch the damage region.
4. **Rasterize.** Draw into the shared-memory buffer, clipped to the damage
   rects ([rendering.md](rendering.md)).
5. **Present.** `wl_surface.damage_buffer` for each rect, then `commit`. A frame
   callback is requested only while more frames are due, for example during
   smooth scrolling or the text cursor blink.

## Source layout

```
Cargo.toml              workspace; release: opt-level=3, LTO, panic=abort, strip. examples/ is excluded.
crates/qa/              the runtime binary
  src/main.rs           command dispatch (including an app's own command name), run_app
  src/cli.rs            install, setup, list, update, rollback, remove, grants, pack
  src/dev.rs            qa dev: cargo build, inotify, reload
  src/open.rs           qa open: the install prompt behind qa: links
  src/shot.rs           qa shot: headless run, scripted steps, PNG + tree dump; --map/--grant
  src/shell.rs          the main loop: platform events → UI → damage-limited frames; title bar
  src/host/             app.rs (app thread, imports, fuel and CPU throttling), fs.rs (openat2
                        sandbox), grants.rs, secrets.rs, encode.rs (events), mod.rs (driver: requests,
                        prompts)
  src/ui/               mod.rs (tree, patch decoder, tokens), layout.rs, paint.rs, interact.rs
                        (pointer, keys, focus), edit.rs (input editing), theme.rs
  src/gfx/              display list, raster.rs, path.rs (glyph outlines), glyph.rs (cache), png.rs
                        (decode, encode)
  src/text/             font index and loading, fallback fonts, shaping (cmap, advances, kern),
                        line breaking
  src/platform/         Platform trait + wayland/ (wire.rs protocol, xkb.rs dlopen, mod.rs)
  src/prompt/           runtime-owned overlays (prompt band, file picker) as a second native UI tree
  src/io/               http.rs, ws.rs, messages, inbox, open_url, URL mapping
  src/pkg/              store.rs (manifest, cache, updates, .desktop, pack), zip.rs, json.rs, sha256.rs
  src/demo.rs, todo.rs  native harnesses; tests.rs holds the headless tests
crates/qa-proto/        no_std: op/event/prop ids and encoding, shared by runtime and SDK;
                        val.rs is the structured value format, its reader, writer and JSON
crates/qa-sdk/          no_std + alloc: the app SDK (json, time, net client, List, Rich, secret)
examples/               counter, widgets, sandbox, fetch, inbox, slack
site/                   the landing page, install.sh, and build.sh, which packs the examples into site/dist
```

## Dependencies

| Crate | Why |
|---|---|
| `libc` | syscalls, Wayland socket, shm, poll, timerfd, inotify |
| `wasmi` 2.x (+ `wasmi_core` for the limiter's error type) | wasm interpreter with fuel metering, calls that resume after fuel runs out, and resource limits |
| `mhttp` (from the mtui repo) | HTTP/1.1 and WebSocket client over rustls |
| `miniz_oxide` | inflate, for zip bundles and PNG |
| `ttf-parser` | font tables (no dependencies of its own) |
| `libxkbcommon` | dlopen'd at runtime to turn keymaps into keysyms and text |

Nothing else. JSON, SHA-256, the zip reader, the PNG decoder, the rasterizer and
the Wayland protocol are small enough to write in-tree.

## Design decisions

| Decision | Why |
|---|---|
| wasmi (interpreter), not a JIT | About 1 MB, instant start with lazy compilation, fuel for CPU limits, runs everywhere (JIT is banned on iOS). UI logic is light. |
| Native built-in components | They keep apps tiny, give every app the same look and keys, and make theming central. Wasm has no standard dynamic linking. |
| Retained tree + patches | Idle apps cost nothing. An update sends only what changed, and the runtime redraws only damaged rects. |
| Grid tracks + priority, not a constraint solver | One predictable O(n) pass that is easy to debug and covers almost every app layout. |
| CPU rasterizer on Wayland shm | Small, no GPU driver memory (often 50+ MB per process), simple damage tracking. The display list boundary lets a GPU or X11 backend be added later. |
| One process per app | Isolation and accounting come free from the OS. Cross-app sharing happens through the page cache. |
| HTTP(S) + WebSocket only | Covers API clients. TLS stays in the runtime, so apps don't ship crypto. |
