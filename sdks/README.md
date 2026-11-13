# SDKs for other languages

The Rust SDK (`crates/qa-sdk`, [docs/sdk.md](../docs/sdk.md)) is the main one.
These are smaller: each is **one file** that covers the UI tree, events and
every host call, so an app in that language needs nothing else.

| Language | File | Counter app | Build needs |
|---|---|---|---|
| C | [c/qa.h](c/qa.h) | 5.4 KB | clang with `wasm-ld`, or `zig cc` |
| Zig | [zig/qa.zig](zig/qa.zig) | 5.4 KB | Zig 0.14 |
| AssemblyScript | [assemblyscript/qa.ts](assemblyscript/qa.ts) | 6.2 KB | `npm install assemblyscript` |
| TinyGo | [tinygo/qa/qa.go](tinygo/qa/qa.go) | 7.7 KB | TinyGo (tested with 0.35) and Go |

(The Rust counter is 7.5 KB.)

## Build and run

Each folder has a `build.sh` and an `examples/` counter:

```sh
sdks/c/build.sh sdks/c/examples/counter.c
sdks/zig/build.sh sdks/zig/examples/counter.zig
sdks/assemblyscript/build.sh sdks/assemblyscript/examples/counter.ts
(cd sdks/tinygo && ./build.sh ./examples/counter)

qa run sdks/c/examples/counter.wasm
```

`qa run`, `qa shot` and `qa pack` take the `.wasm` file. `qa dev` (rebuild on
save) only knows how to build Rust crates.

## What every SDK has

- **Tree:** create each built-in component, set any prop (int, number,
  string, bytes, tracks, padding), theme tokens for a subtree, and the patch
  ops (append, insert, detach, destroy, replace, root, focus, scroll-to).
  Calls collect into one patch that is sent when your handler returns.
- **Events:** every event type decoded, with the node's `tag`.
- **Helpers:** log, window title, timers, read and write a whole file, start
  an HTTP request.
- **Host calls:** every import in [docs/app-abi.md](../docs/app-abi.md),
  declared and ready to call.

## What only the Rust SDK has

`List` (keyed rows), `Rich` (styled text spans), `ui.value(input)`, the JSON
reader and writer, `net::Client` (queueing and retries), the `time` helpers,
WebSocket and secret wrappers, and the permission-request wrappers. In the
other languages you call the host imports for these directly; the encodings
are in the docs.

## The same counter in each

```c
// C
void app_event(const qa_event *e) {
    if (e->type != QA_EV_CLICK) return;
    if (e->node == plus) n++;
    char buf[12];
    qa_set_text(label, qa_itoa(buf, n));
}
```

```zig
// Zig
pub fn event(e: qa.Event) void {
    switch (e) {
        .click => |node| {
            if (node == plus) n += 1;
            var buf: [12]u8 = undefined;
            qa.setText(label, std.fmt.bufPrint(&buf, "{d}", .{n}) catch "?");
        },
        else => {},
    }
}
```

```ts
// AssemblyScript
function event(e: qa.Event): void {
  if (e.type == qa.ev.CLICK && e.node == plus) qa.setText(label, (++n).toString());
}
```

```go
// TinyGo
func event(e *qa.Event) {
	if e.Type == qa.EvClick && e.Node == plus {
		n++
		qa.SetText(label, qa.Itoa(n))
	}
}
```

## Notes per language

- **C:** no libc and no `malloc`. `qa_bump` hands out memory that is never
  freed; bring an allocator if you need one. Strings in an event point into
  the event buffer and are not NUL-terminated.
- **Zig:** uses `std.heap.wasm_allocator` (`qa.allocator`). Strings in an
  event are slices of the event buffer, valid until `event` returns.
- **AssemblyScript:** built with `--runtime minimal`; the SDK collects garbage
  after each event. Strings are converted to UTF-8 on the way out.
- **TinyGo:** target `wasm-unknown` with `-gc conservative`. Register the app
  from `init`, because `main` is not run. The runtime calls the module's
  `_initialize` export first if it has one.

## Adding another language

Anything that compiles to `wasm32` without WASI works. The module needs to
export `memory`, `qa_alloc`, `qa_start` and `qa_event` and import what it
uses from module `"qa"`. [c/qa.h](c/qa.h) is the shortest full example of the
encoding.
