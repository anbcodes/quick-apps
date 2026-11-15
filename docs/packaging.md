# Packaging, hosting and the CLI

## Bundle

A zip file, with entries stored or deflated:

```
app.wasm        required
icon.png        optional, 256×256
assets/…        optional, readable through image `src` and fs handle 1 (read-only)
```

Other entries are ignored. The unzipped size may be at most 64 MB.

### Icon

`icon.png` is the app's icon everywhere one is shown: the menu entry an
install writes, the install prompt, and the app's window (the title bar, task
bar or Dock, depending on the system). Any size works. Make it square, since
other shapes are centred on a transparent square. Without one, the app gets
whatever the system shows for a program with no icon.

On Wayland the window icon needs a compositor with `xdg-toplevel-icon-v1`.
Others take it from the menu entry, so only installed apps have one there.

## Manifest

A JSON file stored **next to** the bundle. Its URL is the app's identity.

```json
{
  "name": "Notes",
  "version": "1.2.0",
  "description": "Plain text notes",
  "bundle": "notes-1.2.0.zip",
  "size": 48213,
  "sha256": "9f2c…",
  "runtime": 1,
  "command": "notes"
}
```

- `bundle` is resolved relative to the manifest URL.
- `runtime` is the ABI version the app needs. A runtime older than that says
  so and refuses to start the app.
- `command` (optional) is the terminal command an install links. Without it,
  the name in lowercase with dashes is used ("Slack Lite" → `slack-lite`).
- **App id** = the first 16 hex characters of `sha256(manifest URL)`. Two URLs
  are two different apps, each with its own folder and grants.
- Remote manifests must use `https://`. `http://` is accepted only for
  loopback. `file://` and local folders are allowed for development.

Any static file host works, such as nginx, S3 or GitHub Pages.

## On disk

```
~/.local/share/quick-apps/apps/<app-id>/
  source          manifest URL
  manifest.json   cached manifest (+ ETag)
  bundle/         extracted current bundle
  bundle.old/     previous bundle (for qa rollback)
  data/           the app's folder (handle 0)
  grants          permission grants
  secrets         secret_set values (mode 0600, not encrypted)
  command         the ~/.local/bin name it is linked as, if any
  inbox/          messages queued while the app wasn't running
```

## Launch and update flow

1. `qa run <url>` computes the app id. If the bundle is cached, the app
   **starts immediately** from `bundle/`. If not, the runtime downloads the
   bundle, checks `size` and `sha256`, extracts it, and then starts the app.
2. At the same time, it fetches the manifest with `If-None-Match` and a 5-s
   timeout. Being offline or getting a fetch error is silent.
3. If `version` is **newer** than the cached one (compared as dotted numbers,
   so `1.10.0` > `1.9.2`; a `-suffix` sorts before its release), the update bar (a runtime overlay)
   shows *"Notes 1.3.0 is available"* with **Update / Later / Skip this
   version**.
4. Update downloads and verifies the new bundle into `bundle.new/`. Then the
   runtime sends `Close` to the app, rotates `bundle` → `bundle.old` and
   `bundle.new` → `bundle`, and restarts. The `data/` folder and grants are
   kept.

## CLI

`<app>` can be an app id, a name, a command name, or a manifest URL.

| Command | Does |
|---|---|
| `qa run <url\|path>` | Runs an app, or activates it if it is already running |
| `qa dev <dir> [--map a=b] [--grant g]` | Builds a Rust app crate, runs it, and rebuilds and reloads it on changes |
| `qa shot <app> [options] [step…]` | Runs an app without a window, performs scripted steps, and writes a PNG and a UI tree dump |
| `qa install <url>` | Caches the app, writes a `.desktop` file and links its command |
| `qa open <qa:url>` | What a `qa:` link runs: starts the app if it is installed, otherwise asks first (see below) |
| `qa setup` | Registers `qa open` as the handler for `qa:` links |
| `qa list` | Lists installed apps with name, version, URL and data size |
| `qa update <app>` | Checks for an update and applies it now |
| `qa rollback <app>` | Swaps `bundle.old` back in |
| `qa remove <app> [--keep-data]` | Deletes the app folder, its `.desktop` file and its command link |
| `qa grants <app> [--revoke N]` | Shows or revokes grants |
| `qa pack <dir> [--out <dir>]` | Builds a zip and manifest, filling in `size` and `sha256` |

### `qa pack`

The input folder holds `app.wasm`, an optional `icon.png` and `assets/`, and an
`app.json` that gives `name`, `version` and `description`. The output is
`<name>-<version>.zip` plus a complete `app.json`. Upload both side by side.
`icon.png` is also copied next to them, so the page that links to the app can
show it.

### `qa dev`

`qa dev examples/notes`:

1. Runs `cargo build --release --target wasm32-unknown-unknown` in that
   folder and loads `target/wasm32-unknown-unknown/release/<crate>.wasm`.
   `assets/` and `icon.png` come from the crate folder.
2. Watches `src/`, `assets/` and `Cargo.toml` with inotify. A change triggers
   a rebuild. On success the app restarts **in the same window**, with its
   tree reset and its data folder kept. Build errors appear in an overlay and
   on stderr.
3. The app id is derived from `file://<absolute dir>`. Logs, traps with
   backtraces, and patch errors go to stderr.

Options (also accepted by `qa shot`):

- `--map <from>=<to>` rewrites HTTP and WebSocket URLs that start with
  `<from>`, e.g. `--map https://slack.com/api/=http://127.0.0.1:8765/api/` to
  point an app at a mock server without changing its code.
- `--grant <grant>` grants a permission for this run only, in the grants file
  syntax ([permissions.md](permissions.md)): `internet`, `cpu`,
  `memory 209715200`, `folder rw /path`, `message <url>`. It is never saved.

### `qa shot`

`<app>` is a crate folder (built like `qa dev`, with the same app id and
data), an app folder or `.wasm`, or an installed app. A `qa:` link shows the
install prompt that `qa open` would show, and clicking Install really
installs, so pair it with a scratch `HOME`. The app runs headless
at `--size WxH` (default 800x600) and `--scale`, then the steps run in order,
each followed by 300 ms for the app to settle:

| Step | Does |
|---|---|
| `click:<text>` | Clicks the button, tab, checkbox, input (by placeholder), menu item, link span or text showing `<text>`; exact matches first, then substrings |
| `dblclick:<text>` | Clicks it twice, to send `DoubleClick` |
| `rclick:<text>` | Right-clicks it, which opens its `context_menu` |
| `hover:<text>` | Moves the pointer over it |
| `wheel:<px>` | Scrolls the `scroll` under the pointer by `<px>` (negative is up) |
| `select:<text>` | Drag-selects `<text>` inside the first text node that contains it |
| `type:<text>` | Types into the focused input |
| `key:<name>` | `Enter`, `Tab`, `Escape`, `Backspace`, `Up`, `Down`, `PageDown`, a letter …, with `ctrl+`, `shift+`, `alt+` prefixes |
| `wait:<ms>` | Lets timers, requests and sockets run |
| `shot:<file.png>` | Writes an extra screenshot |

At the end it writes `--out` (default `shot.png`) and a `.txt` next to it:
one line per visible node with its kind, id, text, variant, tag, focus and
rect, plus any runtime prompt and app error. `--data <dir>` uses another
folder as the app's home, so a test can't touch real data. Links are printed
instead of opened. The exit code is 1 if a step failed or the app trapped.

```
qa shot examples/counter click:+1 click:+1 --out /tmp/counter.png
```

## Desktop integration

`qa install` writes `~/.local/share/applications/qa-<app-id>.desktop`:

```
[Desktop Entry]
Type=Application
Name=Notes
Comment=Plain text notes
Icon=/home/andrew/.local/share/quick-apps/apps/<app-id>/bundle/icon.png
Exec=qa run https://apps.example.com/notes/app.json
StartupWMClass=qa.notes
```

The window's `xdg_toplevel` app_id (and `WM_CLASS` on X11) is `qa.` and the
app's name in lowercase with dashes: `qa.notes`, `qa.slack-lite`. Bars and
switchers that show the app id itself show something readable, and the ones
that look it up find the name and icon here through `StartupWMClass`. Two
apps with the same name share an app id, so a dock may group them.

An app that runs without being installed (`qa run`, `qa dev`, or **Just run
it**) gets the same file with `NoDisplay=true`, so it has its name and icon
while it runs but stays out of the menu. `qa remove` deletes it.

### Commands

An install also links `~/.local/bin/<command>` to the `qa` binary. When `qa`
starts under a name other than `qa`, it runs the installed app linked as that
name, so `notes` works like `qa run notes`. The link is skipped, never
replaced, when the name is taken: by a file in `~/.local/bin` that this app
didn't make, or by a command anywhere else on `PATH` (so an app called Slack
can't shadow the real `slack`).

### `qa:` links

`qa setup`, `qa install` and `qa open` write
`~/.local/share/applications/qa-open.desktop`:

```
[Desktop Entry]
Type=Application
Name=Quick Apps
Exec=/home/andrew/.local/bin/qa open %u
MimeType=x-scheme-handler/qa;
NoDisplay=true
```

and set `x-scheme-handler/qa=qa-open.desktop` under `[Default Applications]`
in `~/.config/mimeapps.list`. A web page can then link to an app:

```html
<a href="qa:https://apps.example.com/notes/app.json">Install Notes</a>
```

`qa://host/path` is short for `qa:https://host/path`. The browser asks once
whether to open `qa` links. Then `qa open`:

1. Starts the app if it is installed (bundle cached and `.desktop` present).
2. Otherwise opens a small runtime window. Any page can fire a `qa:` link, so
   nothing is installed without the user's click. The window shows the
   name, version, size, description and host, and downloads and verifies the
   bundle into `quick-apps/staging/<app-id>` while the user reads.
   - **Install** moves the bundle into place, writes the `.desktop` file,
     links the command, and offers **Open**.
   - **Just run it** caches the bundle and starts the app with no menu entry
     or command. The next click on the link asks again.
   - **Cancel** or Escape throws the download away.

### On macOS

Menu entries are `.app` bundles in `~/Applications`. `qa install` writes
`<Name>.app` with a one-line launcher (`exec qa run <manifest url>`), an
`Info.plist`, and an icon made from `icon.png` with `sips` and `iconutil`, and
tells Launch Services about it. They show up in Spotlight and Launchpad. The
bundle path is saved in the app's folder as `bundle_path`, so updates and
`qa remove` find it.

`qa setup` writes `~/Applications/Quick Apps.app`, which declares the `qa`
URL scheme. Its launcher runs `qa url-handler`, which waits for the Apple Event
that carries the link and then does what `qa open` does. Command links in
`~/.local/bin` work the same as on Linux. Link sockets live in
`/tmp/quick-apps-<uid>`.

### Bootstrap

People without `qa` yet run the install script, which a site serves next to
the runtime binary (see `site/build.sh`):

```
curl -fsSL https://quick-apps.example/install.sh | sh -s -- https://apps.example.com/notes/app.json
```

It picks `qa-x86_64-linux`, `qa-x86_64-darwin` or `qa-aarch64-darwin` for the
machine, checks its sha256, moves it to
`~/.local/bin/qa`, runs `qa setup`, and then `qa open` on the URL if one was
given. It needs no root.
