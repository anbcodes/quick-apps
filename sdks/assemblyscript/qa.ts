// Quick Apps SDK for AssemblyScript.
//
// An app's entry file registers two functions and re-exports the three
// functions the runtime calls:
//
//     import * as qa from "./qa";
//     export { qa_alloc, qa_start, qa_event } from "./qa";
//     qa.app(start, event);
//
// Calls build up one patch, which is sent when `start`/`event` returns.
// Build with build.sh. The wire format is in docs/app-abi.md.

export type Node = u32;

// Node kinds, prop ids and event types (docs/ui-tree.md, docs/components.md,
// docs/events.md).
export namespace kind {
  export const GRID: u8 = 1;
  export const TEXT: u8 = 2;
  export const IMAGE: u8 = 3;
  export const CANVAS: u8 = 4;
  export const SCROLL: u8 = 5;
  export const BUTTON: u8 = 10;
  export const INPUT: u8 = 11;
  export const CHECKBOX: u8 = 12;
  export const SELECT: u8 = 13;
  export const TABS: u8 = 14;
  export const PROGRESS: u8 = 15;
  export const DIVIDER: u8 = 16;
  export const POPUP: u8 = 17;
  export const MENU: u8 = 18;
}

export namespace prop {
  export const ROW: u16 = 1;
  export const COL: u16 = 2;
  export const ROW_SPAN: u16 = 3;
  export const COL_SPAN: u16 = 4;
  export const PRIORITY: u16 = 5;
  export const MIN_W: u16 = 6;
  export const MIN_H: u16 = 7;
  export const MAX_W: u16 = 8;
  export const MAX_H: u16 = 9;
  export const PAD: u16 = 10;
  export const ALIGN_X: u16 = 11;
  export const ALIGN_Y: u16 = 12;
  export const HIDDEN: u16 = 13;
  export const VARIANT: u16 = 14;
  export const TOOLTIP: u16 = 15;
  export const TAG: u16 = 16;
  export const DISABLED: u16 = 17;
  export const CONTEXT_MENU: u16 = 18;
  export const GRID_COLS: u16 = 20;
  export const GRID_ROWS: u16 = 21;
  export const GRID_GAP: u16 = 22;
  export const GRID_CLICKABLE: u16 = 23;
  export const GRID_SELECTED: u16 = 24;
  export const GRID_FLOW: u16 = 25;
  export const TEXT_TEXT: u16 = 20;
  export const TEXT_WRAP: u16 = 21;
  export const TEXT_STYLE: u16 = 22;
  export const TEXT_ALIGN: u16 = 23;
  export const TEXT_SELECTABLE: u16 = 24;
  export const TEXT_MAX_LINES: u16 = 25;
  export const TEXT_SPANS: u16 = 26;
  export const IMAGE_SRC: u16 = 20;
  export const IMAGE_DATA: u16 = 21;
  export const IMAGE_FIT: u16 = 22;
  export const CANVAS_PX_W: u16 = 20;
  export const CANVAS_PX_H: u16 = 21;
  export const CANVAS_FIT: u16 = 22;
  export const CANVAS_FOCUSABLE: u16 = 23;
  export const SCROLL_AXIS: u16 = 20;
  export const SCROLL_STICK_END: u16 = 21;
  export const BUTTON_LABEL: u16 = 20;
  export const INPUT_VALUE: u16 = 20;
  export const INPUT_PLACEHOLDER: u16 = 21;
  export const INPUT_MULTILINE: u16 = 22;
  export const INPUT_SECRET: u16 = 23;
  export const INPUT_LINES: u16 = 25;
  export const CHECKBOX_LABEL: u16 = 20;
  export const CHECKBOX_CHECKED: u16 = 21;
  export const CHECKBOX_WRAP: u16 = 23;
  export const SELECT_OPTIONS: u16 = 20;
  export const SELECT_SELECTED: u16 = 21;
  export const SELECT_PLACEHOLDER: u16 = 22;
  export const TABS_LABELS: u16 = 20;
  export const TABS_SELECTED: u16 = 21;
  export const PROGRESS_VALUE: u16 = 20;
  export const POPUP_OPEN: u16 = 20;
  export const POPUP_ANCHOR: u16 = 21;
  export const POPUP_PLACEMENT: u16 = 22;
  export const MENU_ITEMS: u16 = 20;
}

export namespace ev {
  export const RESIZE: u8 = 1;
  export const CLICK: u8 = 2;
  export const CHANGE: u8 = 3;
  export const SUBMIT: u8 = 4;
  export const TOGGLE: u8 = 5;
  export const SELECT: u8 = 6;
  export const KEY: u8 = 7;
  export const POINTER: u8 = 8;
  export const TIMER: u8 = 9;
  export const HTTP: u8 = 10;
  export const WS: u8 = 11;
  export const MESSAGE: u8 = 12;
  export const REQUEST_DONE: u8 = 13;
  export const CLOSE: u8 = 14;
  export const ACTIVATE: u8 = 15;
  export const FOCUS_OUT: u8 = 16;
  export const NEAR_END: u8 = 17;
  export const LINK: u8 = 18;
  export const DOUBLE_CLICK: u8 = 19;
  export const NEAR_START: u8 = 20;
  export const CONTEXT_MENU: u8 = 21;
  export const DISMISS: u8 = 22;
}

export const STRETCH: i32 = 0;
export const START: i32 = 1;
export const CENTER: i32 = 2;
export const END: i32 = 3;

export namespace key {
  export const BACKSPACE: u32 = 0xff08;
  export const TAB: u32 = 0xff09;
  export const ENTER: u32 = 0xff0d;
  export const ESCAPE: u32 = 0xff1b;
  export const DELETE: u32 = 0xffff;
  export const LEFT: u32 = 0xff51;
  export const UP: u32 = 0xff52;
  export const RIGHT: u32 = 0xff53;
  export const DOWN: u32 = 0xff54;
  export const SHIFT: u8 = 1;
  export const CTRL: u8 = 2;
  export const ALT: u8 = 4;
  export const SUPER: u8 = 8;
}

// ---- the patch ----

let buf: usize = heap.alloc(1024);
let cap: u32 = 1024;
let len: u32 = 0;
let nextId: Node = 0;

function room(more: u32): usize {
  if (len + more > cap) {
    while (len + more > cap) cap *= 2;
    buf = heap.realloc(buf, cap);
  }
  const at = buf + len;
  len += more;
  return at;
}
function u8_(v: u8): void {
  store<u8>(room(1), v);
}
function u16_(v: u16): void {
  store<u16>(room(2), v);
}
function u32_(v: u32): void {
  store<u32>(room(4), v);
}
function f32_(v: f32): void {
  store<f32>(room(4), v);
}
function str_(s: string): void {
  const n = <u32>String.UTF8.byteLength(s);
  u32_(n);
  String.UTF8.encodeUnsafe(changetype<usize>(s), s.length, room(n));
}
function op1(op: u8, a: Node): void {
  u8_(op);
  u32_(a);
}
function op2(op: u8, a: Node, b: Node): void {
  op1(op, a);
  u32_(b);
}
function setHead(n: Node, p: u16, tag: u8): void {
  op1(6, n);
  u16_(p);
  u8_(tag);
}

/** Sends the patch now. Done for you after `start` and `event`. */
export function flush(): void {
  if (len > 0) sys.ui_patch(buf, len);
  len = 0;
}

export class Track {
  constructor(
    public kind: u8,
    public value: f32,
    public min: f32 = f32.POSITIVE_INFINITY,
    public max: f32 = f32.POSITIVE_INFINITY,
  ) {}
}
export function px(v: f32): Track {
  return new Track(0, v);
}
export function fr(v: f32): Track {
  return new Track(1, v);
}
export function auto(): Track {
  return new Track(2, 0);
}

// ---- changing nodes ----

export function setInt(n: Node, p: u16, v: i32): void {
  setHead(n, p, 1);
  u32_(<u32>v);
}
export function setBool(n: Node, p: u16, v: bool): void {
  setInt(n, p, v ? 1 : 0);
}
export function setNum(n: Node, p: u16, v: f32): void {
  setHead(n, p, 2);
  f32_(v);
}
export function setStr(n: Node, p: u16, s: string): void {
  setHead(n, p, 3);
  str_(s);
}
export function setBytes(n: Node, p: u16, b: Uint8Array): void {
  setHead(n, p, 5);
  u32_(b.length);
  memory.copy(room(b.length), b.dataStart, b.length);
}
export function setTracks(n: Node, p: u16, tracks: Track[]): void {
  setHead(n, p, 6);
  u8_(<u8>tracks.length);
  for (let i = 0; i < tracks.length; i++) {
    const t = tracks[i];
    u8_(t.kind);
    f32_(t.value);
    f32_(t.min);
    f32_(t.max);
  }
}
/** Back to the prop's default. */
export function reset(n: Node, p: u16): void {
  setHead(n, p, 0);
}
export function setText(n: Node, s: string): void {
  setStr(n, prop.TEXT_TEXT, s);
}
export function setLabel(n: Node, s: string): void {
  setStr(n, prop.BUTTON_LABEL, s);
}
export function setValue(n: Node, s: string): void {
  setStr(n, prop.INPUT_VALUE, s);
}
export function setChecked(n: Node, on: bool): void {
  setBool(n, prop.CHECKBOX_CHECKED, on);
}
export function hide(n: Node, on: bool): void {
  setBool(n, prop.HIDDEN, on);
}
export function setDisabled(n: Node, on: bool): void {
  setBool(n, prop.DISABLED, on);
}
export function setVariant(n: Node, v: string): void {
  setStr(n, prop.VARIANT, v);
}

// Theme tokens for a subtree (docs/theming.md).
function ctxHead(n: Node, name: string, tag: u8): void {
  op1(7, n);
  str_(name);
  u8_(tag);
}
/** `rgba` is 0xRRGGBBAA. */
export function ctxColor(n: Node, name: string, rgba: u32): void {
  ctxHead(n, name, 4);
  u32_(rgba);
}
export function ctxNum(n: Node, name: string, v: f32): void {
  ctxHead(n, name, 2);
  f32_(v);
}
export function ctxStr(n: Node, name: string, v: string): void {
  ctxHead(n, name, 3);
  str_(v);
}

export function append(parent: Node, child: Node): void {
  op2(3, parent, child);
}
export function insert(parent: Node, child: Node, before: Node): void {
  op2(4, parent, child);
  u32_(before);
}
export function detach(n: Node): void {
  op1(5, n);
}
export function destroy(n: Node): void {
  op1(2, n);
}
/** Puts `with_` where `old` is and destroys `old`. */
export function replace(old: Node, with_: Node): void {
  op2(11, old, with_);
}
export function setRoot(n: Node): void {
  op1(8, n);
}
export function focus(n: Node): void {
  op1(9, n);
}
export function scrollTo(n: Node): void {
  op1(10, n);
}

// ---- building nodes ----

/** A node being built. Finish with `addTo`, `root` or `detached`. */
export class B {
  constructor(public n: Node) {}

  int(p: u16, v: i32): B {
    setInt(this.n, p, v);
    return this;
  }
  num(p: u16, v: f32): B {
    setNum(this.n, p, v);
    return this;
  }
  str(p: u16, s: string): B {
    setStr(this.n, p, s);
    return this;
  }
  cols(t: Track[]): B {
    setTracks(this.n, prop.GRID_COLS, t);
    return this;
  }
  rows(t: Track[]): B {
    setTracks(this.n, prop.GRID_ROWS, t);
    return this;
  }
  gap(v: f32): B {
    return this.num(prop.GRID_GAP, v);
  }
  pad(v: f32): B {
    return this.pad4(v, v, v, v);
  }
  pad4(top: f32, right: f32, bottom: f32, left: f32): B {
    setHead(this.n, prop.PAD, 7);
    f32_(top);
    f32_(right);
    f32_(bottom);
    f32_(left);
    return this;
  }
  at(row: i32, col: i32): B {
    return this.int(prop.ROW, row).int(prop.COL, col);
  }
  span(rows: i32, cols: i32): B {
    return this.int(prop.ROW_SPAN, rows).int(prop.COL_SPAN, cols);
  }
  priority(p: i32): B {
    return this.int(prop.PRIORITY, p);
  }
  minW(v: f32): B {
    return this.num(prop.MIN_W, v);
  }
  minH(v: f32): B {
    return this.num(prop.MIN_H, v);
  }
  maxW(v: f32): B {
    return this.num(prop.MAX_W, v);
  }
  maxH(v: f32): B {
    return this.num(prop.MAX_H, v);
  }
  alignX(a: i32): B {
    return this.int(prop.ALIGN_X, a);
  }
  alignY(a: i32): B {
    return this.int(prop.ALIGN_Y, a);
  }
  hidden(on: bool): B {
    return this.int(prop.HIDDEN, on ? 1 : 0);
  }
  disabled(on: bool): B {
    return this.int(prop.DISABLED, on ? 1 : 0);
  }
  variant(v: string): B {
    return this.str(prop.VARIANT, v);
  }
  /** An i32 sent back with this node's events (`Event.tag`). */
  tag(t: i32): B {
    return this.int(prop.TAG, t);
  }
  /** Text style: "body", "title", "heading", "caption" or "mono". */
  style(s: string): B {
    return this.str(prop.TEXT_STYLE, s);
  }
  clickable(on: bool): B {
    return this.int(prop.GRID_CLICKABLE, on ? 1 : 0);
  }
  placeholder(s: string): B {
    return this.str(prop.INPUT_PLACEHOLDER, s);
  }
  value(s: string): B {
    return this.str(prop.INPUT_VALUE, s);
  }
  checked(on: bool): B {
    return this.int(prop.CHECKBOX_CHECKED, on ? 1 : 0);
  }
  addTo(parent: Node): Node {
    append(parent, this.n);
    return this.n;
  }
  root(): Node {
    setRoot(this.n);
    return this.n;
  }
  detached(): Node {
    return this.n;
  }
}

export function create(k: u8): B {
  nextId++;
  op1(1, nextId);
  u8_(k);
  return new B(nextId);
}
export function grid(): B {
  return create(kind.GRID);
}
export function text(s: string): B {
  return create(kind.TEXT).str(prop.TEXT_TEXT, s);
}
export function button(label: string): B {
  return create(kind.BUTTON).str(prop.BUTTON_LABEL, label);
}
export function input(): B {
  return create(kind.INPUT);
}
export function checkbox(label: string): B {
  return create(kind.CHECKBOX).str(prop.CHECKBOX_LABEL, label);
}
/** Options, one per line. */
export function select(options: string): B {
  return create(kind.SELECT).str(prop.SELECT_OPTIONS, options);
}
/** Labels, one per line. */
export function tabs(labels: string): B {
  return create(kind.TABS).str(prop.TABS_LABELS, labels);
}
export function scroll(): B {
  return create(kind.SCROLL);
}
export function image(src: string): B {
  return create(kind.IMAGE).str(prop.IMAGE_SRC, src);
}
export function canvas(w: i32, h: i32): B {
  return create(kind.CANVAS).int(prop.CANVAS_PX_W, w).int(prop.CANVAS_PX_H, h);
}
export function progress(v: f32): B {
  return create(kind.PROGRESS).num(prop.PROGRESS_VALUE, v);
}
export function divider(): B {
  return create(kind.DIVIDER);
}
/** A popup: leave it detached, give it one child, and show it with `open`. */
export function popup(): B {
  return create(kind.POPUP);
}
/** A menu, one item per line (see docs/components.md). Choosing one sends SELECT. */
export function menu(items: string): B {
  return create(kind.MENU).str(prop.MENU_ITEMS, items);
}

// ---- events ----

/**
 * One event. Which fields are set depends on `type` (see `ev`):
 *
 *   RESIZE        w, h, scale
 *   CLICK, DOUBLE_CLICK, FOCUS_OUT, NEAR_END, NEAR_START, DISMISS   node
 *   CONTEXT_MENU  node, x, y
 *   CHANGE, SUBMIT   node, text
 *   TOGGLE        node, value (0/1)
 *   SELECT        node, value (index)
 *   LINK          node, value (span index)
 *   KEY           node (focused), sym, mods, text
 *   POINTER       node, kind (0 down 1 up 2 move 3 wheel 4 leave), x, y, buttons, dx, dy
 *   TIMER         id
 *   HTTP          id (request), status (0 = failed, data is the error), text (headers), data (body)
 *   WS            id (socket), kind (0 open 1 text 2 binary 3 closed 4 error), data
 *   MESSAGE       text (sender's manifest URL), data
 *   REQUEST_DONE  id (request), value (result), text (info)
 *
 * `tag` is the node's tag prop for every event that has a node.
 */
export class Event {
  type: u8 = 0;
  node: Node = 0;
  tag: i32 = 0;
  value: i32 = 0;
  id: u32 = 0;
  text: string = "";
  data: Uint8Array = new Uint8Array(0);
  sym: u32 = 0;
  mods: u8 = 0;
  kind: u8 = 0;
  buttons: u8 = 0;
  status: u16 = 0;
  x: f32 = 0;
  y: f32 = 0;
  dx: f32 = 0;
  dy: f32 = 0;
  w: f32 = 0;
  h: f32 = 0;
  scale: f32 = 0;

  /** `data` as text (an HTTP body, a WebSocket text frame). */
  dataText(): string {
    return String.UTF8.decodeUnsafe(this.data.dataStart, this.data.length);
  }
}

class Reader {
  constructor(
    public p: usize,
    public end: usize,
  ) {}
  u8(): u8 {
    if (this.p + 1 > this.end) return 0;
    const v = load<u8>(this.p);
    this.p += 1;
    return v;
  }
  u16(): u16 {
    if (this.p + 2 > this.end) return 0;
    const v = load<u16>(this.p);
    this.p += 2;
    return v;
  }
  u32(): u32 {
    if (this.p + 4 > this.end) {
      this.p = this.end;
      return 0;
    }
    const v = load<u32>(this.p);
    this.p += 4;
    return v;
  }
  f32(): f32 {
    return reinterpret<f32>(this.u32());
  }
  str(): string {
    const n = min<usize>(this.u32(), this.end - this.p);
    const s = String.UTF8.decodeUnsafe(this.p, n);
    this.p += n;
    return s;
  }
  bytes(): Uint8Array {
    const n = min<usize>(this.u32(), this.end - this.p);
    const a = new Uint8Array(<i32>n);
    memory.copy(a.dataStart, this.p, n);
    this.p += n;
    return a;
  }
}

export function decode(ptr: usize, length: u32): Event {
  const r = new Reader(ptr, ptr + length);
  const e = new Event();
  e.type = r.u8();
  let hasNode = true;
  switch (e.type) {
    case ev.RESIZE:
      e.w = r.f32();
      e.h = r.f32();
      e.scale = r.f32();
      hasNode = false;
      break;
    case ev.CLICK:
    case ev.DOUBLE_CLICK:
    case ev.FOCUS_OUT:
    case ev.NEAR_END:
    case ev.NEAR_START:
    case ev.DISMISS:
      e.node = r.u32();
      break;
    case ev.CONTEXT_MENU:
      e.node = r.u32();
      e.x = r.f32();
      e.y = r.f32();
      break;
    case ev.CHANGE:
    case ev.SUBMIT:
      e.node = r.u32();
      e.text = r.str();
      break;
    case ev.TOGGLE:
      e.node = r.u32();
      e.value = r.u8();
      break;
    case ev.SELECT:
    case ev.LINK:
      e.node = r.u32();
      e.value = <i32>r.u32();
      break;
    case ev.KEY:
      e.node = r.u32();
      e.sym = r.u32();
      e.mods = r.u8();
      e.text = r.str();
      break;
    case ev.POINTER:
      e.node = r.u32();
      e.kind = r.u8();
      e.x = r.f32();
      e.y = r.f32();
      e.buttons = r.u8();
      e.dx = r.f32();
      e.dy = r.f32();
      break;
    case ev.TIMER:
      e.id = r.u32();
      hasNode = false;
      break;
    case ev.HTTP:
      e.id = r.u32();
      e.status = r.u16();
      e.text = r.str();
      e.data = r.bytes();
      hasNode = false;
      break;
    case ev.WS:
      e.id = r.u32();
      e.kind = r.u8();
      e.data = r.bytes();
      hasNode = false;
      break;
    case ev.MESSAGE:
      e.text = r.str();
      e.data = r.bytes();
      hasNode = false;
      break;
    case ev.REQUEST_DONE:
      e.id = r.u32();
      e.value = <i32>r.u32();
      e.text = r.str();
      hasNode = false;
      break;
    default:
      hasNode = false;
  }
  if (hasNode) e.tag = <i32>r.u32();
  return e;
}

// ---- the app ----

let onStart: (() => void) | null = null;
let onEvent: ((e: Event) => void) | null = null;

/** Registers the app. Call it at the top level of the entry file. */
export function app(start: () => void, event: (e: Event) => void): void {
  onStart = start;
  onEvent = event;
}

export function qa_alloc(length: u32): usize {
  return heap.alloc(max<u32>(length, 1));
}
export function qa_start(): void {
  const f = onStart;
  if (f) f();
  flush();
}
export function qa_event(ptr: usize, length: u32): void {
  const e = decode(ptr, length);
  heap.free(ptr);
  const f = onEvent;
  if (f) f(e);
  flush();
  // Frees what the event left behind (built with --runtime minimal).
  __collect();
}

// ---- helpers ----

function utf8(s: string): ArrayBuffer {
  return String.UTF8.encode(s);
}

export function log(s: string): void {
  const b = utf8(s);
  sys.log(1, changetype<usize>(b), b.byteLength);
}
export function setTitle(s: string): void {
  const b = utf8(s);
  sys.set_title(changetype<usize>(b), b.byteLength);
}
/** Fires one TIMER event after `ms`. */
export function timer(ms: u32): u32 {
  return sys.timer(ms, 0);
}
/** Fires a TIMER event every `ms` until `sys.timer_cancel(id)`. */
export function interval(ms: u32): u32 {
  return sys.timer(ms, 1);
}

export namespace fs {
  export const APP_DIR: i32 = 0;
  export const BUNDLE_DIR: i32 = 1;
  export const READ: u32 = 1;
  export const WRITE: u32 = 2;
  export const CREATE: u32 = 4;
  export const TRUNC: u32 = 8;
  export const APPEND: u32 = 16;
  export const EXCL: u32 = 32;

  /** A whole file, or null if it can't be read. */
  export function read(dir: i32, path: string): Uint8Array | null {
    const p = String.UTF8.encode(path);
    const fd = sys.fs_open(dir, changetype<usize>(p), p.byteLength, READ);
    if (fd < 0) return null;
    let out = new Uint8Array(4096);
    let got = 0;
    while (true) {
      if (got == out.length) {
        const bigger = new Uint8Array(out.length * 2);
        memory.copy(bigger.dataStart, out.dataStart, got);
        out = bigger;
      }
      const n = sys.fs_read(fd, out.dataStart + got, out.length - got);
      if (n < 0) {
        sys.fs_close(fd);
        return null;
      }
      if (n == 0) break;
      got += n;
    }
    sys.fs_close(fd);
    return out.subarray(0, got);
  }

  /** A whole file as text, or null. */
  export function readText(dir: i32, path: string): string | null {
    const b = read(dir, path);
    return b ? String.UTF8.decodeUnsafe(b.dataStart, b.length) : null;
  }

  /** Replaces a whole file. False if it can't be written. */
  export function write(dir: i32, path: string, data: Uint8Array): bool {
    const p = String.UTF8.encode(path);
    const fd = sys.fs_open(dir, changetype<usize>(p), p.byteLength, WRITE | CREATE | TRUNC);
    if (fd < 0) return false;
    let put = 0;
    while (put < data.length) {
      const n = sys.fs_write(fd, data.dataStart + put, data.length - put);
      if (n <= 0) {
        sys.fs_close(fd);
        return false;
      }
      put += n;
    }
    sys.fs_close(fd);
    return true;
  }

  export function writeText(dir: i32, path: string, s: string): bool {
    return write(dir, path, Uint8Array.wrap(String.UTF8.encode(s)));
  }
}

export namespace net {
  /**
   * Starts a request. The answer is an HTTP event whose `id` is the returned
   * value. `headers` is "Name: value\n" lines.
   */
  export function request(method: string, url: string, headers: string = "", body: Uint8Array | null = null): u32 {
    const at = len; // borrow the patch buffer's tail, then give it back
    str_(method);
    str_(url);
    str_(headers);
    if (body) {
      u32_(body.length);
      memory.copy(room(body.length), body.dataStart, body.length);
    } else {
      u32_(0);
    }
    u32_(0); // timeout: 30 s
    u32_(0); // largest body: 8 MB
    const id = sys.http(buf + at, len - at);
    len = at;
    return id;
  }
  export function get(url: string): u32 {
    return request("GET", url);
  }
}

/** Host calls, exactly as in docs/app-abi.md. */
export namespace sys {
  // @ts-ignore: decorator
  @external("qa", "ui_patch")
  export declare function ui_patch(ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "canvas_put")
  export declare function canvas_put(node: u32, x: u32, y: u32, w: u32, h: u32, rgba: usize): i32;
  // @ts-ignore: decorator
  @external("qa", "set_title")
  export declare function set_title(ptr: usize, len: u32): void;
  // @ts-ignore: decorator
  @external("qa", "quit")
  export declare function quit(): void;
  // @ts-ignore: decorator
  @external("qa", "env")
  export declare function env(out: usize, cap: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "now_ms")
  export declare function now_ms(): i64;
  // @ts-ignore: decorator
  @external("qa", "mono_ms")
  export declare function mono_ms(): i64;
  // @ts-ignore: decorator
  @external("qa", "random")
  export declare function random(ptr: usize, len: u32): void;
  // @ts-ignore: decorator
  @external("qa", "log")
  export declare function log(level: u32, ptr: usize, len: u32): void;
  // @ts-ignore: decorator
  @external("qa", "timer")
  export declare function timer(ms: u32, repeat: u32): u32;
  // @ts-ignore: decorator
  @external("qa", "timer_cancel")
  export declare function timer_cancel(id: u32): void;
  // @ts-ignore: decorator
  @external("qa", "json_parse")
  export declare function json_parse(ptr: usize, len: u32, out: usize, cap: u32, needed: usize): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_open")
  export declare function fs_open(dir: i32, path: usize, len: u32, flags: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_read")
  export declare function fs_read(fd: i32, ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_write")
  export declare function fs_write(fd: i32, ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_seek")
  export declare function fs_seek(fd: i32, off: i64, whence: u32): i64;
  // @ts-ignore: decorator
  @external("qa", "fs_close")
  export declare function fs_close(fd: i32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_stat")
  export declare function fs_stat(dir: i32, path: usize, len: u32, out: usize): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_list")
  export declare function fs_list(dir: i32, path: usize, len: u32, out: usize, cap: u32, needed: usize): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_mkdir")
  export declare function fs_mkdir(dir: i32, path: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_remove")
  export declare function fs_remove(dir: i32, path: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "fs_rename")
  export declare function fs_rename(dir: i32, from: usize, fromLen: u32, dir2: i32, to: usize, toLen: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "request")
  export declare function request(kind: u32, arg: usize, len: u32): u32;
  // @ts-ignore: decorator
  @external("qa", "http")
  export declare function http(ptr: usize, len: u32): u32;
  // @ts-ignore: decorator
  @external("qa", "http_cancel")
  export declare function http_cancel(req: u32): void;
  // @ts-ignore: decorator
  @external("qa", "ws_open")
  export declare function ws_open(ptr: usize, len: u32): u32;
  // @ts-ignore: decorator
  @external("qa", "ws_send")
  export declare function ws_send(ws: u32, kind: u32, ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "ws_close")
  export declare function ws_close(ws: u32, code: u32): void;
  // @ts-ignore: decorator
  @external("qa", "msg_send")
  export declare function msg_send(to: usize, toLen: u32, data: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "clipboard_write")
  export declare function clipboard_write(ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "open_url")
  export declare function open_url(ptr: usize, len: u32): i32;
  // @ts-ignore: decorator
  @external("qa", "secret_get")
  export declare function secret_get(key: usize, keyLen: u32, out: usize, cap: u32, needed: usize): i32;
  // @ts-ignore: decorator
  @external("qa", "secret_set")
  export declare function secret_set(key: usize, keyLen: u32, val: usize, valLen: u32): i32;
}
