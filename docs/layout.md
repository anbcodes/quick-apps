# Layout

All lengths are **logical px** (`f32`). The rasterizer multiplies them by the
window scale. There is one layout container, the **grid**. A grid with no
props is a vertical stack: one `1fr` column, with rows added as `auto`.

## Tracks

`cols` and `rows` are lists of tracks:

| Kind | Size wanted | Can shrink to |
|---|---|---|
| `px(n)` | `n` | `n` (rigid) unless `min` is set lower |
| `auto` | Largest preferred size of its cells | Largest minimum size of its cells |
| `fr(n)` | A share `n` of the free space | Largest minimum size of its cells |

Every track also takes an optional `min`/`max` clamp. Tracks are separated by
`gap`.

## Cells

Each child is a cell. It is placed by `row`/`col`, or **auto-placed** in the
next free slot in row-major order, which adds new `auto` rows as needed. It may
span several tracks. Inside its area, a cell is positioned by
`align_x`/`align_y` (stretch by default) and clamped by its own
`min_*`/`max_*`.

A cell's **priority** (default 0) passes to the tracks it starts in. A track's
priority is the highest priority among its cells.

## Measuring

Every node can answer three questions. The answers are cached and recomputed
only when the node or a descendant is dirty.

- `min_w`: the narrowest width it can use. For text, the longest word, or
  the ellipsis width when it doesn't wrap. For an input, about 4 characters.
- `pref_w`: its natural width. For text, the width of one unwrapped line.
- `height(w)`: its height at width `w`. Text wraps to `w`.

A grid measures by sizing its tracks against an unbounded width (for `pref_w`)
or at their minimums (for `min_w`), then adds padding and gaps.

## Sizing tracks along one axis

Columns go first, against the available width. Rows go second, against the
available height, using each cell's `height(column width)`.

1. **Base sizes.** `px` gets its value, `auto` gets the largest preferred size
   of its single-span cells, and `fr` gets its minimum. Clamp each to the
   track's `min`/`max`.
2. **Surplus.** If the sum fits, the free space goes to the `fr` tracks in
   proportion to `n`. A track that reaches its `max` freezes, and the rest is
   shared again. With no `fr` tracks, the leftover space sits after the last
   track.
3. **Shortage.** If the sum doesn't fit, walk priority levels **from lowest
   to highest**. At each level:
   1. Shrink that level's tracks toward their minimums, in proportion to how
      far each one is above its minimum, until the shortage is covered.
   2. If the shortage remains **and the level is below 0**, hide that level's
      tracks one at a time, the last one first. A hidden track frees its size
      and one gap, and its cells are not painted and get no events.

   If tracks of priority 0 and above are all at their minimums and space is
   still short, the content overflows and is clipped at the grid's edge. Put
   it in a `scroll` to make it scroll instead.
4. **Spanning cells.** After single-span sizing, a cell that spans several
   tracks and still needs more room splits the difference evenly over the
   `auto` and `fr` tracks it covers.

When the available size is infinite (along the axis of a `scroll`), every track
gets its base size and `fr` behaves like `auto`.

The rule in step 3 is: **low-priority content gives up everything, shrinking
and then hiding, before higher-priority content gives up anything.**

## Example: sidebar, content and an optional inspector

```
cols: [px(220, min 140), fr(1, min 240), px(280)]
sidebar   priority 0
content   priority 1
inspector priority -1
```

As the window gets narrower, the inspector is hidden first. It can't shrink,
because a `px` track without a lower `min` is rigid. Next the sidebar shrinks
from 220 to 140. Then the content shrinks to 240. Below that, the content is
clipped.

## Flow

A grid with `flow` set has no tracks. It places its visible
children left to right at their preferred widths (capped at its own width)
and starts a new line when the next one doesn't fit. `gap` separates items
and lines. Its minimum width is its widest child's minimum, its preferred
width is all children on one line, and its height at a given width is the sum
of its lines. Use it for chips, tags and toolbars, where a fixed column count
leaves gaps or overflows.

## Scroll

A `scroll` measures its child with infinite space along its axis. Its own
minimum along that axis is 0, so it can always shrink. Its preferred size is
the child's preferred size.

## Incremental layout

- A patch that changes size-affecting props (text, label, tracks, padding,
  `hidden`…) marks the node **measure-dirty**, and the mark spreads to its
  ancestors.
- Layout runs top-down. A subtree whose rect and inputs didn't change is
  skipped.
- Paint-only props (`checked`, `selected`, `variant`, a context color) skip
  layout entirely and only mark the node's rect as damaged.

Changing one label costs O(depth × siblings).
