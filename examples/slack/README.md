# Slack

A small Slack client for Quick Apps, modeled on mtui's `mslack`. It shows
channels and DMs, history with day separators, threads, reactions and clickable
links (with *bold*, `code` and mentions styled), and it sends messages and thread replies. Outgoing `@name`, `#channel` and
`@here` become real Slack mentions.

```sh
../../target/release/qa dev .
```

## Signing in

The first launch shows a sign-in form. The tokens go to the runtime's secret
store (`~/.local/share/quick-apps/apps/<app-id>/secrets`, mode 0600 but not
encrypted). Tokens that an older version saved in `data/config` move there on
the next start.

1. At <https://api.slack.com/apps>, choose **Create New App → From an app
   manifest** and paste [`slack-app-manifest.yml`](slack-app-manifest.yml).
2. Install it and copy the **User OAuth Token** (`xoxp-…`).
3. For live updates, add an **App-Level Token** with `connections:write`
   (`xapp-…`).

A browser session token (`xoxc-…`) together with its `d` cookie (`xoxd-…`)
also works, and gets live updates through RTM. See mslack's README for how to
find them.

| Credentials | Live updates |
|---|---|
| `xoxp-…` + `xapp-…` | Socket Mode |
| `xoxc-…` + `xoxd-…` cookie | RTM |
| `xoxp-…` alone | none; polls the open conversation every 5 s |

## Keys

| Key | |
|---|---|
| `Enter` | send |
| `Alt+Up` / `Alt+Down` | previous / next conversation |
| `Esc` | close the thread |

## Testing without Slack

Point the app at a mock server with the runtime's URL mapping, and give it
the internet grant for that run only:

```sh
qa dev . --map https://slack.com/api/=http://127.0.0.1:8765/api/ --grant internet
qa shot . --data /tmp/slack-test --map https://slack.com/api/=http://127.0.0.1:8765/api/ \
    --grant internet wait:1000 "click:2 replies" --out /tmp/slack.png
```

An `api <url>` line in `data/config` does the same from inside the app.

## Limitations

- No file upload or download; files are shown by name.
- No color emoji font in the runtime, so most `:shortcodes:` stay as text.
  A few that DejaVu Sans can draw (♥ ★ ✔ ⚠ …) are swapped in.
- No search, editing, deleting, or unread tracking for conversations other
  than through `client.counts` and live events.
