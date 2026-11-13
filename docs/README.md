# Quick Apps

Quick Apps is a small runtime for tiny, resource-efficient desktop apps. An app
is a WebAssembly module that describes its UI as a tree of native components. It
never draws pixels, never sees the real filesystem, and can only reach the
network or other folders after the user agrees in a prompt drawn by the runtime.

Think of it as the web with less in it. There is no HTML, CSS or JS engine and no
GPU. A UI is a tree of grids, styled only through an inherited theme context, and
an app is about 10–100 KB.

> **Status:** milestones M0–M6 are implemented ([roadmap.md](roadmap.md)).
> The few places where the code does less than these docs describe are listed
> under "Not yet implemented" in the roadmap.

## Build and try it

```sh
cargo build --release                       # the runtime: target/release/qa
cd examples/counter && cargo build --release --target wasm32-unknown-unknown
../../target/release/qa run target/wasm32-unknown-unknown/release/counter.wasm
../../target/release/qa dev .               # rebuild and reload on every save
cargo test --release                        # from the repo root; add -- --include-ignored for slow/network tests
```

Examples: `counter`, `widgets` (every component), `sandbox` (files, prompts,
limits), `fetch` (HTTPS), `inbox` (messages between apps). `qa demo` and
`qa demo-todo` are native harnesses for the renderer and the UI tree.

## Goals

1. **Minimal runtime.** One static binary, a handful of dependencies, and code
   that one person can read in full.
2. **Efficient.** An idle app costs 0% CPU and wakes up only for input, timers
   and I/O. Only damaged regions are redrawn.
3. **Tiny apps.** Built-in components live in the runtime, not in the app.
4. **Safe by default.** Each app gets its own folder. Anything beyond that
   asks the user first.
5. **Fast to write.** Making a new app for a small task takes minutes. It has
   a Rust SDK and `qa dev` with live reload.

## Budgets (targets)

| What | Target |
|---|---|
| Runtime binary (stripped) | ≤ 4 MB |
| Hello-world `app.wasm` | ≤ 8 KB |
| Typical app `app.wasm` | ≤ 100 KB |
| Launch to first frame (cached app) | ≤ 100 ms |
| Idle CPU / wakeups | 0 |
| RSS, hello world, 1600×1000 window | ≤ 25 MB (mostly the framebuffer) |
| Default app memory limit | 50 MB |
| Default app CPU limit | 50% of one core, averaged over 10 s |

## v1 scope

- Linux desktop on Wayland and macOS, with a CPU renderer.
- The wasm engine is wasmi, a pure-Rust interpreter.
- Apps are written in Rust (`no_std` + `alloc`) against `qa-sdk`.
- All components are native and built into the runtime.

## Docs

| File | What it covers |
|---|---|
| [architecture.md](architecture.md) | Processes, threads, the frame pipeline, source layout, design decisions |
| [app-abi.md](app-abi.md) | Wasm exports and imports, memory, error codes, value encoding, structured values (JSON) |
| [ui-tree.md](ui-tree.md) | Nodes, the patch op stream, common props |
| [events.md](events.md) | Event types and how they are encoded |
| [layout.md](layout.md) | Grid tracks, priorities and the layout algorithm |
| [components.md](components.md) | Built-in components with their props, events and theme tokens |
| [theming.md](theming.md) | The context cascade, tokens and the user theme file |
| [rendering.md](rendering.md) | Display list, rasterizer, text, Wayland, swapping backends |
| [io.md](io.md) | Files, JSON, network, messages between apps, clipboard, timers, links, secrets |
| [permissions.md](permissions.md) | Prompts, grants, memory and CPU limits |
| [packaging.md](packaging.md) | Bundles, manifests, cache, updates, install, `qa:` links, commands, CLI, `qa dev` and `qa shot` |
| [sdk.md](sdk.md) | The Rust SDK and a sample app |
| [roadmap.md](roadmap.md) | Milestones, what is out of scope, settled decisions and why |

## Glossary

- **Runtime**: the `qa` binary. Each running app has its own `qa` process.
- **App**: `app.wasm` plus assets, shipped as a zip bundle that sits next to a
  manifest.
- **App id**: the first 16 hex characters of `sha256(manifest URL)`.
- **Node**: one element of the UI tree. Its id is a `u32` chosen by the app.
- **Patch**: a byte stream of tree operations sent from the app to the runtime.
- **Context**: inherited key/value theme tokens. They replace CSS.
- **Grant**: a permission the user has approved, stored for each app.
- **Handle**: a small integer that stands for a folder or file the app may use.
  Handle `0` is the app's own folder.
