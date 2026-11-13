# App ABI

An app is a core wasm module (`wasm32-unknown-unknown`, MVP features plus
`bulk-memory`, `sign-ext`, `mutable-globals`). It has no WASI. Every host
function lives in the import module `"qa"`. The SDK wraps all of this, and
these docs are the contract the SDK implements.

## Exports (required)

| Export | Signature | Called |
|---|---|---|
| `memory` | memory | — |
| `qa_alloc` | `(len: u32) -> u32` | Before each `qa_event`. Returns a pointer to `len` bytes that the app owns. |
| `qa_start` | `()` | Once, after instantiation. The app builds its first tree here. |
| `qa_event` | `(ptr: u32, len: u32)` | Once per event ([events.md](events.md)). The app frees the buffer. |

Calls are never re-entrant. The runtime makes one call at a time on the app
thread.

## Encoding

Everything is little-endian. `qa-proto` holds the reference encoder and decoder.

| Type | Bytes |
|---|---|
| `u8 u16 u32 u64 i32 i64 f32 f64` | raw LE |
| `str` | `u32` byte length + UTF-8 |
| `bytes` | `u32` length + raw bytes |
| `value` | `u8` tag + payload (below) |

Value tags (used by props and context):

| Tag | Type | Payload |
|---|---|---|
| 0 | none | — (resets a prop to its default) |
| 1 | i32 | `i32` (bools are 0/1, enums are small ints) |
| 2 | f32 | `f32` (lengths are logical px) |
| 3 | str | `str` |
| 4 | color | `u32` as `0xRRGGBBAA` |
| 5 | bytes | `bytes` |
| 6 | tracks | `u8` count, then per track: `u8` kind (0 px, 1 fr, 2 auto), `f32` value, `f32` min, `f32` max (`inf` means none) |
| 7 | edges | 4 × `f32`: top, right, bottom, left |

### Structured values

`json_parse` produces this separate encoding for JSON-shaped
data. It has its own tags, so any language can read it without a JSON parser:

| Tag | Type | Payload |
|---|---|---|
| 0 | null | — |
| 1 | false | — |
| 2 | true | — |
| 3 | number | `f64` |
| 4 | string | `str` |
| 5 | list | `u32 count, u32 size`, then `count` values |
| 6 | map | `u32 count, u32 size`, then `count` × (`str` key, value) |

`size` is the byte length of what follows it, so a reader can skip a list or
map without walking it. Map keys keep their source order and may repeat.
`qa-proto::val` has a zero-copy reader (`Val`: `get`, `at`, `path`, `iter`,
`entries`, `str`, `num`, `bool`; anything missing reads as null), a `Writer`,
`from_json` and `to_json`.

## Return conventions

Imports that can fail return `i32`. A value ≥ 0 is success or a result, and a
negative value is an error:

| Code | Name | Meaning |
|---|---|---|
| -1 | `DENIED` | No grant, or the user said no |
| -2 | `NOT_FOUND` | |
| -3 | `EXISTS` | |
| -4 | `INVALID` | Bad argument, bad path or malformed bytes |
| -5 | `IO` | OS error |
| -6 | `LIMIT` | The memory budget or a size limit would be exceeded |
| -7 | `BAD_HANDLE` | |
| -8 | `TOO_SMALL` | Output buffer too small. Call again with the size in `*needed`. |

A trap ends the app. The runtime shows the message and a **Restart** button. In
`qa dev` the trap and the wasm backtrace also go to stderr.

## Imports (`"qa"` module)

Pointers and lengths are `u32`. Strings are UTF-8 (`ptr, len`) with no NUL.

### UI

| Import | Signature | Notes |
|---|---|---|
| `ui_patch` | `(ptr, len) -> i32` | Appends ops ([ui-tree.md](ui-tree.md)). They are applied when the current call returns. |
| `canvas_put` | `(node, x, y, w, h, ptr) -> i32` | Copies RGBA8 pixels, row-major, `w*4` stride, into a canvas node |
| `set_title` | `(ptr, len)` | Window title |
| `quit` | `()` | Exits after the current call returns |

### Environment and utilities

| Import | Signature | Notes |
|---|---|---|
| `env` | `(out, cap) -> i32` | Writes `f32 win_w, f32 win_h, f32 screen_w, f32 screen_h, f32 scale, u8 is_mobile` (21 bytes), then, if `cap` ≥ 25, `i32 utc_offset` (the local zone's offset from UTC in seconds). Returns the bytes written. Nothing else about the machine is exposed. |
| `now_ms` | `() -> i64` | Wall clock, ms since the Unix epoch |
| `mono_ms` | `() -> i64` | Monotonic clock |
| `random` | `(ptr, len)` | Fills with OS randomness |
| `log` | `(level: u32, ptr, len)` | 0 debug … 3 error. Goes to stderr, or the `qa dev` console. |
| `timer` | `(ms: u32, repeat: u32) -> u32` | Returns a timer id. Fires a `Timer` event. |
| `timer_cancel` | `(id)` | |
| `json_parse` | `(ptr, len, out, cap, needed) -> i32` | Parses JSON into the structured value encoding above and returns the bytes written. `INVALID` if it isn't JSON; `TOO_SMALL` sets `*needed`. |

### Files ([io.md](io.md))

| Import | Signature |
|---|---|
| `fs_open` | `(dir, path, path_len, flags) -> i32 fd` |
| `fs_read` | `(fd, ptr, len) -> i32 n` |
| `fs_write` | `(fd, ptr, len) -> i32 n` |
| `fs_seek` | `(fd, off: i64, whence) -> i64` |
| `fs_close` | `(fd) -> i32` |
| `fs_stat` | `(dir, path, path_len, out) -> i32` |
| `fs_list` | `(dir, path, path_len, out, cap, needed) -> i32` |
| `fs_mkdir` | `(dir, path, path_len) -> i32` |
| `fs_remove` | `(dir, path, path_len) -> i32` |
| `fs_rename` | `(dir, from, from_len, dir2, to, to_len) -> i32` |

### Requests (async, may prompt)

| Import | Signature | Notes |
|---|---|---|
| `request` | `(kind, arg, arg_len) -> u32 req` | The result arrives as a `RequestDone` event. Kinds are in [permissions.md](permissions.md). |

### Network ([io.md](io.md))

| Import | Signature |
|---|---|
| `http` | `(ptr, len) -> u32 req` |
| `http_cancel` | `(req)` |
| `ws_open` | `(ptr, len) -> u32 ws` |
| `ws_send` | `(ws, kind, ptr, len) -> i32` |
| `ws_close` | `(ws, code)` |

### Other

| Import | Signature | Notes |
|---|---|---|
| `msg_send` | `(to, to_len, data, data_len) -> i32` | Message to another app |
| `clipboard_write` | `(ptr, len) -> i32` | Text only |
| `open_url` | `(ptr, len) -> i32` | Opens `http(s)` URLs in the system browser. Allowed only while handling a user input event. |
| `secret_get` | `(key, key_len, out, cap, needed) -> i32` | Returns the value's length, `NOT_FOUND`, or `TOO_SMALL` ([io.md](io.md#secrets)) |
| `secret_set` | `(key, key_len, val, val_len) -> i32` | An empty value removes the key. |


