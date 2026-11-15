# Roadmap

## Milestones

Each milestone ends with something that runs.

| # | Milestone | Done when | Status |
|---|---|---|---|
| M0 | **Pixels.** Wayland client, shm buffers, rasterizer (fills, strokes, rounded rects, clips), font loading, glyph rasterizing and caching | A hard-coded display list with text renders crisply at scales 1 and 1.25, resizes smoothly, and sits at 0% CPU when idle | Done (`qa demo`) |
| M1 | **Tree and layout.** `qa-proto`, patch decoder, grid layout, text / button / input / checkbox / scroll, focus and keys, damage tracking | A native Rust test harness builds a todo UI through patches. Typing redraws only the input's rect. | Done (`qa demo-todo`, tests) |
| M2 | **Wasm.** wasmi host, ABI, `qa-sdk`, `app!`, `qa dev` with reload | `examples/counter` runs, and `counter.wasm` is ≤ 8 KB | Done (7.5 KB) |
| M3 | **Sandbox.** fs handles with `openat2`, timers, prompt band, grants file, file and folder pickers, memory and CPU limits | A runaway loop gets throttled. A memory hog triggers the prompt. `..` paths fail. | Done (`examples/sandbox`, tests) |
| M4 | **Network and messages.** HTTP and WebSocket via `mhttp`, internet prompt, app sockets and inbox, clipboard, `open_url` | An example app fetches JSON and another one receives messages | Done (`examples/fetch`, `examples/inbox`) |
| M5 | **Distribution.** zip reader, manifest, cache, update bar, `pack` / `install` / `list` / `remove` / `grants` / `rollback`, `.desktop` files | An app hosted on a static server installs, launches from the app menu, and updates | Done |
| M6 | **Polish.** Theme files with live reload, select / tabs / progress / divider / image / canvas, the GNOME title bar | Switching the theme file restyles every running app within a second | Done (`examples/widgets`) |

Measured on the reference machine: runtime binary 3.93 MB, counter app 7.2 KB,
about 15 MB RSS for the counter window, 0 CPU ticks while idle.

### Not yet implemented

These are described elsewhere in the docs but the code does less:

- **Text:** `tooltip` (prop 15) is stored but does nothing. A text selection
  stays inside one text node. GPOS kerning is not read (only the `kern` table).
  Color emoji draw only from bitmap fonts (CBDT, sbix); COLR fonts such as
  Twemoji or Segoe UI Emoji are not drawn in color.
- **Memory budget:** only wasm linear memory is counted. Host-side buffers
  (nodes, decoded images, pending network bodies) are not, and never return
  `LIMIT`.
- **Activate:** the event reaches the app, but the window is not raised
  (that needs `xdg_activation_v1`).
- **SDK:** `qa pack` does not run `wasm-opt`.
- **Secrets** are kept in a 0600 file, not in an encrypted keyring.
- **Title bar:** client-side decorations have move, edge resize and close, but
  no maximize. They were tested on sway with `QA_CSD=1`, not on GNOME.
- **Input testing:** real keyboard and pointer input were checked through the
  headless tests, not with a person at the keyboard.

## Out of scope for v1

- **Other platforms and backends:** X11, macOS, Windows, Android, iOS, a GPU
  renderer, a terminal backend ([rendering.md](rendering.md) explains how each
  would be added).
- **Text:** IME, complex scripts and RTL, rich-text spans.
- **UI:** accessibility (AT-SPI), an animation API, virtualized lists, more
  than one window per app, drag and drop, notifications, printing.
- **Wasm:** WASI, threads, SIMD, app-supplied components.
- **Distribution:** app signing (identity is the HTTPS URL), disk quotas,
  launching an app when a message arrives for it.
- **Network:** streaming HTTP bodies, and uploads bigger than the memory
  budget.

## Later: app-supplied components

All built-ins are native. If an app needs a custom reusable widget, the
planned design is a component `.wasm` module that the runtime caches once by
hash and shares between apps. An app places a node of kind ≥ 128 in its tree,
and the runtime asks the component module to expand it into built-in nodes,
styled only from context. The component's events go back to the module.
Instances of one component inside one app share one module instance. The app's
own `.wasm` stays small because it never links the component code.

## Settled decisions

These were open questions. Each is now settled, and the reason is recorded so
nobody has to argue it again.

| Question | Decision | Why |
|---|---|---|
| CPU limit | **50% of one core, averaged over 10 s** (not 50% of the whole system) | Wasm here is single-threaded, so on a multi-core machine an app could never hit 50% of the system. The point of the limit is to catch busy loops and battery drain. |
| Fonts | **System fonts** (DejaVu, Noto or Liberation, found on disk). None in the binary. | Keeps the binary small. Every Linux desktop has one of them. If none is found, `qa` exits with a message naming the `font.family` token to set. On macOS the system Arial, Courier New and Apple Symbols are used. |
| Theme precedence | **App `CTX` beats the user theme**, except tokens the user locks with `!` | Apps need small, local styling (a red tag, an accented header). The user still has the final say through locks and per-app theme files. |
| Message consent | **Prompt the sender on its first send to each receiver.** The answer is stored in the sender's grants. | Data leaving an app is what needs consent. No manifest fields, and receivers stay simple. |
| Prompt spoofing | **The in-window prompt band is enough** | A fake prompt drawn by the app can't grant anything: grants exist only in the runtime. The worst case is a confusing UI, not a privilege escalation. A separate window would add code and focus quirks on Wayland. |
| wasmi fuel | **Requires wasmi ≥ 0.46**, which can resume a call after fuel runs out ([wasmi #1498](https://github.com/wasmi-labs/wasmi/pull/1498)) | Throttling pauses and resumes the call instead of killing it. |
| Updates | **Offer an update only when the manifest's `version` is newer**, compared as dotted numbers (`1.10.0` > `1.9.2`; a pre-release suffix after `-` sorts before its release) | A misconfigured or rolled-back server never pushes a downgrade. Going back to an older version is an explicit `qa rollback`. |
