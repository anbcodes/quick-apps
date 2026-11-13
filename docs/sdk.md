# Rust SDK (`qa-sdk`)

`qa-sdk` is `no_std` + `alloc`. It wraps the raw ABI ([app-abi.md](app-abi.md))
and adds nothing at runtime that an app doesn't use.

## Crate setup

```toml
[package]
name = "counter"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
qa-sdk = { path = "../../crates/qa-sdk" }

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

Build with `qa dev .`, or with
`cargo build --release --target wasm32-unknown-unknown`.

## Example: counter

```rust
#![no_std]
use qa_sdk::*;

struct Counter { n: i32, label: Node, plus: Node, reset: Node }

impl App for Counter {
    fn start(ui: &mut Ui) -> Self {
        let root  = ui.grid().pad(16.0).gap(12.0).root();
        let label = ui.text("0").style("title").align_x(CENTER).add_to(root);
        let row   = ui.grid().cols(&[fr(1.0), fr(1.0)]).gap(8.0).add_to(root);
        let plus  = ui.button("+1").variant("primary").add_to(row);
        let reset = ui.button("Reset").add_to(row);
        Counter { n: 0, label, plus, reset }
    }

    fn event(&mut self, ui: &mut Ui, ev: Event) {
        match ev {
            Event::Click(id) if id == self.plus => self.n += 1,
            Event::Click(id) if id == self.reset => self.n = 0,
            _ => return,
        }
        ui.set_text(self.label, itoa(self.n).as_str());
    }
}

app!(Counter);
```

`app!` generates `qa_alloc`, `qa_start` and `qa_event`, installs the SDK's
allocator, decodes each event, and after `start`/`event` returns, sends
everything `Ui` buffered in **one** `ui_patch` call.

## API overview

| Area | API |
|---|---|
| App | `trait App { fn start(ui: &mut Ui) -> Self; fn event(&mut self, ui: &mut Ui, ev: Event); }` |
| Create | `ui.grid()`, `text(s)`, `button(s)`, `input()`, `checkbox(s)`, `select(opts)`, `tabs(labels)`, `scroll()`, `image(src)`, `canvas(w, h)`, `progress(v)`, `divider()`: each returns a builder |
| Builder | Chainable setters (`.cols()`, `.rows()`, `.gap()`, `.pad()`, `.at(row, col)`, `.span(r, c)`, `.priority()`, `.variant()`, `.style()`, `.ctx(key, val)`, …), finished by `.add_to(parent)`, `.root()` or `.detached()`. Each returns a `Node`. |
| Update | `ui.set_text(n, s)`, `ui.set(n, prop::CHECKED, 1)`, `ui.hide(n, bool)`, `ui.ctx(n, key, val)`, `ui.append(p, c)`, `ui.insert(p, c, before)`, `ui.replace(old, new)`, `ui.detach(n)`, `ui.destroy(n)`, `ui.focus(n)`, `ui.scroll_to(n)`, `ui.set_cols(n, tracks)`, `ui.set_rows(n, tracks)`, `ui.set_tag(n, t)` |
| Tags | `.tag(i32)` on a builder; `ui.tag()` is the tag of the node the current event came from |
| Lists | `List<K>`: children of one parent by key. `put` (append or replace in place), `insert(at)`, `remove`, `clear`, `get`, `key_of` |
| Rich text | `Rich`: `push(s)`, `styled(s, span::BOLD \| span::LINK, variant) -> span index`, `mark(start, end, ..)`; then `ui.rich(&r)` or `ui.set_rich(n, &r)`. A click on a link span sends `Event::Link(node, index)`. |
| Flow | `.flow(true)` on a grid: children wrap like words |
| Tracks | `px(n)`, `fr(n)`, `auto()`, with `.min(x)` / `.max(x)` |
| Events | `enum Event<'a> { Resize{..}, Click(Node), Change(Node, &'a str), Submit(..), Toggle(..), Select(..), Key{..}, Pointer{..}, Timer(u32), Http{..}, Ws{..}, Message{..}, RequestDone{..}, Close, Activate, FocusOut(Node), NearEnd(Node), Link(Node, u32) }`: strings borrow from the event buffer |
| Files | `fs::read(dir, path) -> Result<Vec<u8>>`, `fs::write(dir, path, bytes)`, `fs::list(dir, path)`, `File` with `read`/`write`/`seek`, `APP_DIR` (= 0) |
| Network | `net::get(url) -> ReqId`, `net::request(Request{..}) -> ReqId`, `net::ws(url) -> Ws`; helpers `net::form(&[(k, v)])`, `url_encode`, `url_decode`, `header(headers, name)` |
| HTTP client | `net::Client<T>`: a queue that keeps at most `max` (6) requests in flight, drops a request identical to one still pending, retries 429/503 after `Retry-After`, and returns each response with the tag `T` it was sent with. `send`, `get`, `post_form`, `post_json`, `has(f)`, `cancel(f)`; call `handle(&ev)` first in `event` |
| JSON | `json::parse(bytes) -> Option<Doc>` (parsed by the runtime), `doc.root() -> Val` with `get`, `at`, `path("a.0.b")`, `iter`, `entries`, `str`, `opt_str`, `num`, `int`, `bool`, `len`, `is_null`; `json::stringify(val)`, `json::quote(s)`, `json::Writer` to build values |
| Time | `time::now()` (local seconds), `local(unix_secs)`, `day`, `weekday`, `civil(day) -> (y, m, d)`, `days_from_civil`, `hhmm`, `ymd`, `short_date`, `day_label` ("Today", "Yesterday", "Tuesday, Nov 10"), `ago`, `utc_offset()` |
| Secrets | `secret::get(key) -> Option<String>`, `secret::set(key, value)` ([io.md](io.md#secrets)) |
| Other | `timer(ms)`, `interval(ms)`, `msg::send(to, bytes)`, `request::{open_file, save_file, pick_folder, folder, internet, memory}`, `clipboard::write`, `open_url`, `env()`, `now_ms()`, `random(&mut buf)`, `log!` |
| Text utils | `itoa`, `ftoa`, `cat` (no `core::fmt`) |

## Patterns

**Lists.** Keep a `List<K>` keyed by your item ids, or a `Vec<(Node, Item)>`.
Make each row a `clickable` grid with a `tag`, and use `variant("card")` or
`selected` for looks. To change a row, `SET` its props, or build a new row and
`replace` the old one. Don't rebuild the list. To delete, `destroy` the row's
node. For long feeds, append rows on `NearEnd`.

**Saving.** Write to `APP_DIR` on `FocusOut`, on `Close`, or with a debounced
`timer`. For example, after `Change`, restart a 500 ms timer and save when it
fires.

**Fetching.** Use a `net::Client<Tag>` with an enum saying what each request
is for, and match on `response.tag`. Show a `progress(-1.0)` until it arrives.
`response.json()` parses the body.

**Responsive.** Give optional columns `priority(-1)` so the layout hides them
on narrow windows ([layout.md](layout.md)). Use `Resize` only to switch to a
truly different layout.

## Keeping apps small

- Avoid `core::fmt`, which means `format!`, `{:?}`, and `.unwrap()` and
  `.expect()` with panic messages. It alone adds 10–20 KB. Use `itoa`/`ftoa`
  and `unwrap_or`, or an `abort()` helper instead.
- Avoid `serde` and big crates. `json::parse` runs in the runtime, so apps
  don't carry a parser. `Val::num` doesn't parse strings, which keeps float
  parsing out of the binary.
- The SDK's allocator is a small free list with size classes (about 1 KB of
  code). It is the default, and `features = ["no-alloc"]` turns it off.
- Optionally run `wasm-opt -Oz` (binaryen) on `app.wasm` before `qa pack`.
