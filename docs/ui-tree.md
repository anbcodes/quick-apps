# UI tree

The runtime owns one tree per app. The app changes it by sending **patches**
(`ui_patch`), and the runtime reports user actions as **events**. The app never
reads the tree back, so it keeps whatever state it needs itself.

## Nodes

- The app picks node ids (`u32`, never 0). The SDK hands them out from a
  counter.
- Every node has a **kind**, a set of **props** (unset props use their
  defaults), an optional **context** map ([theming.md](theming.md)), and
  ordered children.
- Only containers (`grid`, `scroll`) may have children. A `scroll` has exactly
  one child.
- Exactly one node is the **root**. It fills the window.

Kinds:

| Id | Kind | Id | Kind |
|---|---|---|---|
| 1 | grid | 10 | button |
| 2 | text | 11 | input |
| 3 | image | 12 | checkbox |
| 4 | canvas | 13 | select |
| 5 | scroll | 14 | tabs |
| | | 15 | progress |
| | | 16 | divider |

Each kind is described in [components.md](components.md). Kind ids 128 and up
are reserved for app-supplied components ([roadmap.md](roadmap.md)).

## Patch ops

A patch is a run of ops back to back. Each op starts with a `u8` opcode.

| Op | Name | Fields | Effect |
|---|---|---|---|
| 1 | `CREATE` | `u32 id, u8 kind` | New detached node. Error if the id exists. |
| 2 | `DESTROY` | `u32 id` | Detaches the node and destroys it and its whole subtree |
| 3 | `APPEND` | `u32 parent, u32 child` | Moves `child` to the end of `parent` (detaching it first) |
| 4 | `INSERT` | `u32 parent, u32 child, u32 before` | Moves `child` in front of `before` |
| 5 | `DETACH` | `u32 child` | Removes it from its parent but keeps it, so it can be reattached later |
| 6 | `SET` | `u32 id, u16 prop, value` | Sets a prop. `value` tag 0 resets it. |
| 7 | `CTX` | `u32 id, str key, value` | Sets a context token for this subtree. Tag 0 removes it. |
| 8 | `ROOT` | `u32 id` | Makes this node the root |
| 9 | `FOCUS` | `u32 id` | Moves keyboard focus to the node |
| 10 | `SCROLL_TO` | `u32 id` | Scrolls the nearest `scroll` ancestors until the node is visible |
| 11 | `REPLACE` | `u32 old, u32 new` | Puts `new` where `old` is in its parent (or makes it the root, if `old` was the root), then destroys `old`. `new` may not be inside `old`. |

Rules:

- A malformed patch traps the app with an error message. In release this
  restarts the app, and in `qa dev` it reports the byte offset.
- Detached nodes still count toward the memory budget. Destroy what you no
  longer need.
- Repeated `SET`s of one prop within a patch cost the same as one: the runtime
  marks dirty only once, at the end.
- If a prop is set to the value it already has, nothing is marked dirty.

## Common props

Any node accepts these. The prop id is the `u16` in `SET`.

| Id | Prop | Type | Default | Meaning |
|---|---|---|---|---|
| 1 | `row` | i32 | auto | Grid row in the parent (0-based) |
| 2 | `col` | i32 | auto | Grid column in the parent |
| 3 | `row_span` | i32 | 1 | |
| 4 | `col_span` | i32 | 1 | |
| 5 | `priority` | i32 | 0 | Who gives up space first. Below 0 means the node may be hidden ([layout.md](layout.md)). |
| 6 | `min_w` | f32 | 0 | Clamps the measured size |
| 7 | `min_h` | f32 | 0 | |
| 8 | `max_w` | f32 | inf | |
| 9 | `max_h` | f32 | inf | |
| 10 | `pad` | edges | 0 | Inner padding |
| 11 | `align_x` | i32 | 0 | 0 stretch, 1 start, 2 center, 3 end |
| 12 | `align_y` | i32 | 0 | Same values |
| 13 | `hidden` | i32 | 0 | 1 takes the node out of layout and paint |
| 14 | `variant` | str | "" | Picks a family of theme tokens, e.g. `"primary"`, `"danger"`, `"card"` |
| 15 | `tooltip` | str | "" | Shown on hover after 600 ms |
| 16 | `tag` | i32 | 0 | Sent back with the node's `Click`, `Change`, `Submit`, `Toggle`, `Select` and `Link` events ([events.md](events.md)) |

Kind-specific props start at id 20 and are listed in
[components.md](components.md).

## Events and ids

Every event that comes from a node carries its id. The app maps ids to its own
handlers. The SDK does this by keeping a small `id → action` table.

For rows built from data, set `tag` instead, for example to the row's index:
the event then says which row it was without a lookup table.
