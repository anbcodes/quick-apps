# Permissions and limits

## Default

Every app can do the following without asking. Everything else needs a grant.

- Draw in its own window.
- Read and write its own folder (handle `0`).
- Use timers, randomness, clocks, logging and clipboard writes.
- Receive messages.
- Use up to **50 MB** of memory and **50% of one core** of CPU.

## Prompts

The runtime draws prompts **outside the app's control**:

- A band slides in at the top of the window, showing the app's icon and name,
  a plain sentence, and buttons.
- While it is open, the app's content is dimmed and gets no input. The app
  can't draw over the band, read it, or click it.
- Prompts are queued and shown one at a time. `Esc` means Deny.
- An app could draw a fake band in its own content, but that can't grant
  anything, because grants exist only in the runtime. That risk is accepted
  ([roadmap.md](roadmap.md)).
- The buttons are **Allow** (saved in grants), **Allow once** (until the
  process exits) and **Deny**.

File and folder pickers are also runtime overlays. They are built from the
same native components and styled by the user's theme. Choosing a file in a
picker counts as consent for that file, so no further prompt is shown.

## Requests

`request(kind, arg, arg_len) -> req`. The answer arrives as `RequestDone{req,
result, info}`. `result` is a handle, 0, or an error code.

| Kind | Name | Arg | Result / info | Prompt? |
|---|---|---|---|---|
| 1 | `OPEN_FILE` | `str` filter, e.g. `"*.md;*.txt"` | file handle (read) / file name | Picker |
| 2 | `SAVE_FILE` | `str` suggested name | file handle (write) / file name | Picker |
| 3 | `PICK_FOLDER` | `u8 mode` (0 read, 1 read-write) | dir handle / absolute path | Picker, with a "Remember" box |
| 4 | `FOLDER` | `u8 mode, str abs_path` | dir handle / path | Yes, unless a grant exists |
| 5 | `INTERNET` | — | 0 | Yes, unless a grant exists (also asked on the first network call) |
| 6 | `MEMORY` | `u64 bytes` | 0 | Yes |
| 7 | `CPU` | — | 0 (no limit) | Yes |
| 8 | `CLIPBOARD_READ` | — | 0 / text | Only when not close to user input ([io.md](io.md)) |
| 9 | `MESSAGE` | `str target` | 0 | Yes, unless a grant exists (also asked on the first `msg_send`) |

## Grants file

`~/.local/share/quick-apps/apps/<app-id>/grants` holds one grant per line.
People can read and edit it.

```
internet
folder rw /home/andrew/notes
folder r /home/andrew/Pictures
memory 209715200
cpu
message https://apps.example.com/calendar/app.json
```

`qa grants <app>` lists the grants with line numbers, and `qa grants <app>
--revoke N` deletes one. A change takes effect at the app's next launch.

`qa dev` and `qa shot` take `--grant <line>` (same syntax) to grant something
for that run only; it is never written to the file
([packaging.md](packaging.md#qa-dev)).

## Memory limit

**Counted:** the wasm linear memory, plus bytes the host holds for the app.
That covers nodes and their strings (detached ones too), decoded images,
canvas buffers, pending HTTP and WebSocket buffers, and queued events.
**Not counted:** runtime code, fonts, the glyph cache and the window buffers.

- **Linear memory growth.** The wasmi `ResourceLimiter` sees `memory.grow`.
  Growth past the budget **blocks the app thread** and prompts *"Notes wants
  to use more than 50 MB"*. Allow doubles the budget (and saves it). Deny makes
  `memory.grow` return -1.
- **Host allocations** are not counted yet (see the roadmap). The plan: past the budget they fail with `LIMIT` and don't prompt. An
  app that expects big data calls `request(MEMORY, bytes)` first.

## CPU limit

- The runtime measures the app thread's CPU time (`CLOCK_THREAD_CPUTIME_ID`)
  over a sliding 10-s window. Network and TLS threads are not counted.
- Wasm runs in **fuel slices** of about 1M instructions. When a slice runs
  out, the call pauses (wasmi ≥ 0.46 can resume it afterwards), and the
  runtime checks the window. If the app is over the limit, the app thread
  sleeps until it is back under. That throttles the app without killing it.
- After 10 s of throttling in total, the runtime prompts *"Notes is using a lot
  of CPU"*. The choices are **Allow** (no limit, saved), **Keep limited**, or
  **Stop**.
- **Not responding.** If one `qa_event` call has run for more than 5 s of wall
  time, the runtime shows **Wait / Stop**. Stop kills the instance and offers
  Restart.
