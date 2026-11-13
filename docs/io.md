# I/O

Every I/O call goes through the runtime. An app has no ambient authority: it
can only use handles it was given and grants the user approved
([permissions.md](permissions.md)).

## Files

### Handles

| Handle | What it is | How the app gets it |
|---|---|---|
| `0` | The app's own folder, `~/.local/share/quick-apps/apps/<app-id>/data/` | Always available |
| `1` | The app's bundle (`app.wasm`, `assets/`…), read-only | Always available |
| dir handle | A folder the user chose or allowed | `request(PICK_FOLDER)`, `request(FOLDER)` |
| file handle | A single file the user chose | `request(OPEN_FILE)` (read-only), `request(SAVE_FILE)` (write-only) |

Handles belong to the process and are gone after a restart. To get a folder
back silently, the app saves the path from `RequestDone.info` and calls
`request(FOLDER, path)` on the next run. If an "always" grant exists, no prompt
appears.

### Paths

- Paths are relative to the handle and `/`-separated. Empty means the handle
  itself.
- A path may not contain a leading `/`, `..`, or NUL. Such paths get
  `INVALID`.
- Paths are resolved with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)`,
  so a symlink can't escape the handle.
- For a file handle, the path must be empty.

### Calls

| Call | Details |
|---|---|
| `fs_open` | flags: 1 READ, 2 WRITE, 4 CREATE, 8 TRUNC, 16 APPEND, 32 EXCL. Returns an fd. At most 64 open at once. |
| `fs_seek` | whence: 0 set, 1 cur, 2 end |
| `fs_stat` | Writes 17 bytes to `out`: `u8 kind` (0 file, 1 dir, 2 other), `u64 size`, `i64 mtime_ms` |
| `fs_list` | Writes repeated `u8 kind, str name` and returns the bytes written. Returns `TOO_SMALL` and sets the `u32` at `needed` when `cap` is too small. |
| `fs_mkdir`, `fs_remove`, `fs_rename` | `fs_remove` deletes files and empty folders only |

File calls block. They run on the app thread, so the window stays responsive.

## JSON

`json_parse(ptr, len, out, cap, needed)` parses JSON on the host and writes
the structured value encoding ([app-abi.md](app-abi.md#structured-values)).
Parsing natively is much faster than in the interpreter, and apps don't need
to carry a parser.

## HTTP

`http(ptr, len)` takes this encoding:

```
str method, str url, str headers ("Name: value\n"…), bytes body, u32 timeout_ms (0 = 30 s), u32 max_body (0 = 8 MB)
```

It returns a request id right away. The result arrives as an `Http` event.

- The first call without an `internet` grant prompts the user, and the request
  waits for the answer. If the user denies it, the response is `status 0, body
  "denied"`.
- Redirects are followed (at most 5). Bodies are decompressed (gzip and
  deflate). There is no cookie jar and no CORS: the app sets its own headers.
- At most 8 requests per app are in flight. Pending bodies count toward the
  memory budget.
- TLS uses rustls (via `mhttp`) with the system trust store.

## WebSocket

`ws_open(ptr, len)` takes `str url, str headers` and returns a ws id. The
`internet` grant is required, the same as for HTTP. `ws_send(ws, kind, ptr,
len)` sends with kind 1 text or 2 binary. `ws_close(ws, code)` closes the
socket. Everything that happens arrives as `Ws` events (open, text, binary,
closed, error). The runtime answers pings itself.

## Messages between apps

`msg_send(to, data)` delivers `data` (at most 1 MB) to another app. `to` is
that app's manifest URL or app id.

- The first time app A sends to app B, A's window prompts *"Notes wants to
  send data to Calendar"*. The answer is stored in A's grants. Receiving needs
  no grant: the receiver sees `from` (A's manifest URL) and decides for
  itself.
- If B is running, the message is delivered right away over B's socket
  (`$XDG_RUNTIME_DIR/quick-apps/<b-id>.sock`) and `msg_send` returns 0.
- If B is not running, the message is appended to B's `inbox/` (at most 100
  messages or 4 MB, oldest dropped first) and `msg_send` returns 1. B gets
  queued messages as `Message` events right after `qa_start`. Sending a
  message never launches B.

## Clipboard

- `clipboard_write(text)` always works.
- Pasting into a native `input` always works, because the runtime does it.
- An app reads the clipboard directly with `request(CLIPBOARD_READ)`. Within
  1 s of a user input event this succeeds without a prompt. Otherwise the user
  is prompted. The text arrives in `RequestDone.info`.

## Timers

`timer(ms, repeat)` timers live on the app thread, which waits for its next
event with a timeout set to the earliest timer. The smallest interval is 4 ms. Timers never fire while the app is still handling
an event: they wait in the queue like every other event.

## Opening links

`open_url(url)` runs `xdg-open` for `http`/`https` URLs. It is allowed only
while the app is handling a `Click`, `Key`, `Submit`, `Link` or `Pointer` down
event. Otherwise it returns `DENIED`.

## Secrets

`secret_set(key, value)` and `secret_get(key)` keep small values such as API
tokens in `~/.local/share/quick-apps/apps/<id>/secrets`. The file sits outside
`data/`, so file imports can't reach it, and it is created with mode 0600.
It is **not encrypted**: anything running as the user can read it, the same as
a token in a dotfile. It keeps the token out of the app's data folder, its
backups and its exports.

- Keys are 1–64 characters of `A-Z a-z 0-9 . _ -`. Values are at most 64 KB,
  and an app has at most 100 secrets (`LIMIT`).
- Setting an empty value removes the key. `qa remove` deletes the file with
  the app folder; `--keep-data` keeps it.
