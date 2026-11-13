# Theming and context

There is no CSS. All styling is **context**: string keys (tokens) mapped to
values, inherited down the tree. Components read tokens and nothing else
([components.md](components.md)).

## Lookup

To resolve token `k` for node `n`:

1. Walk from `n` up to the root and use the first node that has a `CTX` entry
   for `k`.
2. Otherwise use the **per-app theme file**, if there is one.
3. Otherwise use the **user theme file**.
4. Otherwise use the **built-in default**.

A token the user marks **locked** (`!`, see below) skips step 1. Apps can't
override locked tokens, which is useful for `font.scale` or contrast colors.

Components use fallback chains, such as `button.primary.bg` →
`color.accent`. A theme can therefore restyle one component without touching
the rest.

Each node keeps a pointer to its resolved context. That is a small map, shared
by every node below it until one of them adds a `CTX`. Lookups are O(depth) in
the worst case and cached per node, and the cache is cleared when a `CTX`
changes above the node.

## Core tokens

| Token | Default (light) | Use |
|---|---|---|
| `color.bg` | `#ffffff` | Window background |
| `color.fg` | `#1f2328` | Text |
| `color.muted` | `#656d76` | Secondary text |
| `color.surface` | `#f6f8fa` | Inputs, buttons, cards |
| `color.border` | `#d0d7de` | |
| `color.accent` / `color.accent_fg` | `#0969da` / `#ffffff` | Primary actions, selection marks |
| `color.danger` / `color.danger_fg` | `#cf222e` / `#ffffff` | |
| `color.success`, `color.warning` | `#1a7f37`, `#9a6700` | |
| `color.hover`, `color.press` | `#0000000d`, `#0000001a` | Tints laid over the base |
| `color.selection` | `#0969da26` | Selected rows, text selection |
| `color.focus` | `#0969da` | Focus ring |
| `color.link` | `color.accent` | Link spans in text |
| `color.code_bg` | `color.surface` | Background of code spans |
| `font.family` | first found of DejaVu Sans, Noto Sans, Liberation Sans | Path or file name |
| `font.mono.family` | DejaVu Sans Mono, … | |
| `font.fallback` | DejaVu Sans, Noto Sans, Noto symbols, … ([rendering.md](rendering.md#text)) | Comma-separated fonts for characters the main font lacks |
| `font.body.size` | 14 | Also `title` 22, `heading` 17, `caption` 12, `mono` 13 |
| `font.<style>.weight` | 400 (`title` and `heading` 600) | |
| `font.scale` | 1.0 | Multiplies every font size |
| `line.height` | 1.35 | |
| `space.gap` | 8 | Default grid gap |
| `space.pad` | 8 | Button and surface padding |
| `radius` | 6 | |
| `surface.card.*` | bg `color.surface`, border `color.border` | A ready-made `variant: "card"` |

Apps and themes may define extra tokens. For example, `color.tag = #8250df`
lets any text node use `variant: "tag"`. A canvas can't read tokens, so an app
that draws its own pixels picks its own colors.

## Theme files

```
~/.config/quick-apps/theme              # user theme (may be a symlink into themes/)
~/.config/quick-apps/themes/*.theme     # saved themes
~/.config/quick-apps/apps/<app-id>.theme  # optional per-app override
```

The format is one `key = value` per line. `#` starts a comment, and a trailing
`!` locks the token.

```
color.bg     = #1e1f22
color.fg     = #dfe1e5
color.accent = #7aa2f7
font.family  = Inter-Regular.ttf
font.scale   = 1.15 !
```

Values: `#rrggbb` or `#rrggbbaa` is a color, a number is an f32, and anything
else (quotes optional) is a string.

The runtime watches these files with inotify. A change re-resolves every token
and repaints. If a font token changed, it also relayouts. Dark mode is just
another theme file, and apps are never told which theme is active.

## Fonts

A font token is either an absolute path or a file name. A file name is looked
up in `~/.config/quick-apps/fonts/`, then `~/.local/share/fonts/`, then
`/usr/share/fonts/` (searched recursively once, with the results cached in
`~/.cache/quick-apps/fonts.idx`). No font ships in the binary. If no font is
found, `qa` exits with a message naming the `font.family` token to set. For weight ≥ 600 the runtime uses
`<family>.bold` if that token is set. Otherwise it draws a synthetic bold
(outlines thickened by about 1/30 em).
