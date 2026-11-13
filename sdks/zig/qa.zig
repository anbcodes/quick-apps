//! Quick Apps SDK for Zig (written against Zig 0.14).
//!
//! An app is a root file with `pub fn start() void` and
//! `pub fn event(e: qa.Event) void`, plus one line that exports them:
//!
//!     comptime { qa.exportApp(@This()); }
//!
//! Calls build up one patch, which is sent when `start`/`event` returns.
//! Strings inside an `Event` point into the event buffer: copy what you keep.
//! The wire format is in docs/app-abi.md; build with build.sh.
const std = @import("std");

pub const allocator = std.heap.wasm_allocator;
pub const Node = u32;

pub const Kind = enum(u8) { grid = 1, text = 2, image = 3, canvas = 4, scroll = 5, button = 10, input = 11, checkbox = 12, select = 13, tabs = 14, progress = 15, divider = 16 };

/// Prop ids (docs/ui-tree.md and docs/components.md).
pub const prop = struct {
    pub const row = 1;
    pub const col = 2;
    pub const row_span = 3;
    pub const col_span = 4;
    pub const priority = 5;
    pub const min_w = 6;
    pub const min_h = 7;
    pub const max_w = 8;
    pub const max_h = 9;
    pub const pad = 10;
    pub const align_x = 11;
    pub const align_y = 12;
    pub const hidden = 13;
    pub const variant = 14;
    pub const tooltip = 15;
    pub const tag = 16;
    pub const disabled = 17;
    pub const grid = struct {
        pub const cols = 20;
        pub const rows = 21;
        pub const gap = 22;
        pub const clickable = 23;
        pub const selected = 24;
        pub const flow = 25;
    };
    pub const text = struct {
        pub const text = 20;
        pub const wrap = 21;
        pub const style = 22;
        pub const text_align = 23;
        pub const selectable = 24;
        pub const max_lines = 25;
        pub const spans = 26;
    };
    pub const image = struct {
        pub const src = 20;
        pub const data = 21;
        pub const fit = 22;
    };
    pub const canvas = struct {
        pub const px_w = 20;
        pub const px_h = 21;
        pub const fit = 22;
        pub const focusable = 23;
    };
    pub const scroll = struct {
        pub const axis = 20;
        pub const stick_end = 21;
    };
    pub const button = struct {
        pub const label = 20;
    };
    pub const input = struct {
        pub const value = 20;
        pub const placeholder = 21;
        pub const multiline = 22;
        pub const secret = 23;
        pub const lines = 25;
    };
    pub const checkbox = struct {
        pub const label = 20;
        pub const checked = 21;
        pub const wrap = 23;
    };
    pub const select = struct {
        pub const options = 20;
        pub const selected = 21;
        pub const placeholder = 22;
    };
    pub const tabs = struct {
        pub const labels = 20;
        pub const selected = 21;
    };
    pub const progress = struct {
        pub const value = 20;
    };
};

pub const Align = enum(i32) { stretch = 0, start = 1, center = 2, end = 3 };

pub const key = struct {
    pub const backspace = 0xff08;
    pub const tab = 0xff09;
    pub const enter = 0xff0d;
    pub const escape = 0xff1b;
    pub const delete = 0xffff;
    pub const left = 0xff51;
    pub const up = 0xff52;
    pub const right = 0xff53;
    pub const down = 0xff54;
    pub const shift = 1;
    pub const ctrl = 2;
    pub const alt = 4;
    pub const super = 8;
};

/// Negative results from host calls.
pub const err = struct {
    pub const denied = -1;
    pub const not_found = -2;
    pub const exists = -3;
    pub const invalid = -4;
    pub const io = -5;
    pub const limit = -6;
    pub const bad_handle = -7;
    pub const too_small = -8;
};

// ---- the patch ----

var patch: std.ArrayListUnmanaged(u8) = .empty;
var next_id: Node = 0;
/// The tag of the node the current event came from (0 if none).
pub var tag: i32 = 0;

fn put(bytes: []const u8) void {
    patch.appendSlice(allocator, bytes) catch @trap();
}
fn putInt(comptime T: type, v: T) void {
    put(&std.mem.toBytes(std.mem.nativeToLittle(T, v)));
}
fn putF32(v: f32) void {
    putInt(u32, @bitCast(v));
}
fn putStr(s: []const u8) void {
    putInt(u32, @intCast(s.len));
    put(s);
}
fn op1(op: u8, a: Node) void {
    putInt(u8, op);
    putInt(u32, a);
}
fn op2(op: u8, a: Node, b: Node) void {
    op1(op, a);
    putInt(u32, b);
}
fn setHead(n: Node, p: u16, value_tag: u8) void {
    op1(6, n);
    putInt(u16, p);
    putInt(u8, value_tag);
}

/// Sends the patch now. Done for you after `start` and `event`.
pub fn flush() void {
    if (patch.items.len > 0) _ = sys.ui_patch(patch.items.ptr, patch.items.len);
    patch.clearRetainingCapacity();
}

pub const Track = struct { kind: u8, value: f32, min: f32 = std.math.inf(f32), max: f32 = std.math.inf(f32) };
pub fn px(v: f32) Track {
    return .{ .kind = 0, .value = v };
}
pub fn fr(v: f32) Track {
    return .{ .kind = 1, .value = v };
}
pub fn auto() Track {
    return .{ .kind = 2, .value = 0 };
}

// ---- changing nodes ----

pub fn setInt(n: Node, p: u16, v: i32) void {
    setHead(n, p, 1);
    putInt(i32, v);
}
pub fn setBool(n: Node, p: u16, v: bool) void {
    setInt(n, p, @intFromBool(v));
}
pub fn setNum(n: Node, p: u16, v: f32) void {
    setHead(n, p, 2);
    putF32(v);
}
pub fn setStr(n: Node, p: u16, s: []const u8) void {
    setHead(n, p, 3);
    putStr(s);
}
pub fn setBytes(n: Node, p: u16, b: []const u8) void {
    setHead(n, p, 5);
    putStr(b);
}
pub fn setTracks(n: Node, p: u16, tracks: []const Track) void {
    setHead(n, p, 6);
    putInt(u8, @intCast(tracks.len));
    for (tracks) |t| {
        putInt(u8, t.kind);
        putF32(t.value);
        putF32(t.min);
        putF32(t.max);
    }
}
/// Back to the prop's default.
pub fn reset(n: Node, p: u16) void {
    setHead(n, p, 0);
}
pub fn setText(n: Node, s: []const u8) void {
    setStr(n, prop.text.text, s);
}
pub fn setLabel(n: Node, s: []const u8) void {
    setStr(n, prop.button.label, s);
}
pub fn setValue(n: Node, s: []const u8) void {
    setStr(n, prop.input.value, s);
}
pub fn setChecked(n: Node, on: bool) void {
    setBool(n, prop.checkbox.checked, on);
}
pub fn hide(n: Node, on: bool) void {
    setBool(n, prop.hidden, on);
}
pub fn setDisabled(n: Node, on: bool) void {
    setBool(n, prop.disabled, on);
}
pub fn setVariant(n: Node, v: []const u8) void {
    setStr(n, prop.variant, v);
}

/// A theme token for a subtree (docs/theming.md).
pub const Tok = union(enum) { none, color: u32, num: f32, str: []const u8 };
pub fn ctx(n: Node, name: []const u8, v: Tok) void {
    op1(7, n);
    putStr(name);
    switch (v) {
        .none => putInt(u8, 0),
        .num => |x| {
            putInt(u8, 2);
            putF32(x);
        },
        .str => |s| {
            putInt(u8, 3);
            putStr(s);
        },
        .color => |c| {
            putInt(u8, 4);
            putInt(u32, c);
        },
    }
}

pub fn append(parent: Node, child: Node) void {
    op2(3, parent, child);
}
pub fn insert(parent: Node, child: Node, before: Node) void {
    op2(4, parent, child);
    putInt(u32, before);
}
pub fn detach(n: Node) void {
    op1(5, n);
}
pub fn destroy(n: Node) void {
    op1(2, n);
}
/// Puts `new` where `old` is and destroys `old`.
pub fn replace(old: Node, new: Node) void {
    op2(11, old, new);
}
pub fn setRoot(n: Node) void {
    op1(8, n);
}
pub fn focus(n: Node) void {
    op1(9, n);
}
pub fn scrollTo(n: Node) void {
    op1(10, n);
}

// ---- building nodes ----

/// A node being built. Finish with `addTo`, `root` or `detached`.
pub const B = struct {
    n: Node,

    pub fn int(b: B, p: u16, v: i32) B {
        setInt(b.n, p, v);
        return b;
    }
    pub fn num(b: B, p: u16, v: f32) B {
        setNum(b.n, p, v);
        return b;
    }
    pub fn str(b: B, p: u16, s: []const u8) B {
        setStr(b.n, p, s);
        return b;
    }
    pub fn cols(b: B, t: []const Track) B {
        setTracks(b.n, prop.grid.cols, t);
        return b;
    }
    pub fn rows(b: B, t: []const Track) B {
        setTracks(b.n, prop.grid.rows, t);
        return b;
    }
    pub fn gap(b: B, v: f32) B {
        return b.num(prop.grid.gap, v);
    }
    pub fn pad(b: B, v: f32) B {
        return b.pad4(v, v, v, v);
    }
    pub fn pad4(b: B, top: f32, right: f32, bottom: f32, left: f32) B {
        setHead(b.n, prop.pad, 7);
        for ([_]f32{ top, right, bottom, left }) |v| putF32(v);
        return b;
    }
    pub fn at(b: B, row: i32, col: i32) B {
        return b.int(prop.row, row).int(prop.col, col);
    }
    pub fn span(b: B, row_span: i32, col_span: i32) B {
        return b.int(prop.row_span, row_span).int(prop.col_span, col_span);
    }
    pub fn priority(b: B, p: i32) B {
        return b.int(prop.priority, p);
    }
    pub fn minW(b: B, v: f32) B {
        return b.num(prop.min_w, v);
    }
    pub fn minH(b: B, v: f32) B {
        return b.num(prop.min_h, v);
    }
    pub fn maxW(b: B, v: f32) B {
        return b.num(prop.max_w, v);
    }
    pub fn maxH(b: B, v: f32) B {
        return b.num(prop.max_h, v);
    }
    pub fn alignX(b: B, a: Align) B {
        return b.int(prop.align_x, @intFromEnum(a));
    }
    pub fn alignY(b: B, a: Align) B {
        return b.int(prop.align_y, @intFromEnum(a));
    }
    pub fn hidden(b: B, on: bool) B {
        return b.int(prop.hidden, @intFromBool(on));
    }
    pub fn disabled(b: B, on: bool) B {
        return b.int(prop.disabled, @intFromBool(on));
    }
    pub fn variant(b: B, v: []const u8) B {
        return b.str(prop.variant, v);
    }
    /// An i32 sent back with this node's events (see `qa.tag`).
    pub fn withTag(b: B, t: i32) B {
        return b.int(prop.tag, t);
    }
    /// Text style: "body", "title", "heading", "caption" or "mono".
    pub fn style(b: B, s: []const u8) B {
        return b.str(prop.text.style, s);
    }
    pub fn clickable(b: B, on: bool) B {
        return b.int(prop.grid.clickable, @intFromBool(on));
    }
    pub fn placeholder(b: B, s: []const u8) B {
        return b.str(prop.input.placeholder, s);
    }
    pub fn value(b: B, s: []const u8) B {
        return b.str(prop.input.value, s);
    }
    pub fn checked(b: B, on: bool) B {
        return b.int(prop.checkbox.checked, @intFromBool(on));
    }
    pub fn addTo(b: B, parent: Node) Node {
        append(parent, b.n);
        return b.n;
    }
    pub fn root(b: B) Node {
        setRoot(b.n);
        return b.n;
    }
    pub fn detached(b: B) Node {
        return b.n;
    }
};

pub fn create(kind: Kind) B {
    next_id += 1;
    op1(1, next_id);
    putInt(u8, @intFromEnum(kind));
    return .{ .n = next_id };
}
pub fn grid() B {
    return create(.grid);
}
pub fn text(s: []const u8) B {
    return create(.text).str(prop.text.text, s);
}
pub fn button(label: []const u8) B {
    return create(.button).str(prop.button.label, label);
}
pub fn input() B {
    return create(.input);
}
pub fn checkbox(label: []const u8) B {
    return create(.checkbox).str(prop.checkbox.label, label);
}
/// Options, one per line.
pub fn select(options: []const u8) B {
    return create(.select).str(prop.select.options, options);
}
/// Labels, one per line.
pub fn tabs(labels: []const u8) B {
    return create(.tabs).str(prop.tabs.labels, labels);
}
pub fn scroll() B {
    return create(.scroll);
}
pub fn image(src: []const u8) B {
    return create(.image).str(prop.image.src, src);
}
pub fn canvas(w: i32, h: i32) B {
    return create(.canvas).int(prop.canvas.px_w, w).int(prop.canvas.px_h, h);
}
pub fn progress(v: f32) B {
    return create(.progress).num(prop.progress.value, v);
}
pub fn divider() B {
    return create(.divider);
}

// ---- events (docs/events.md) ----

pub const Event = union(enum) {
    resize: struct { w: f32, h: f32, scale: f32 },
    click: Node,
    change: struct { node: Node, value: []const u8 },
    submit: struct { node: Node, value: []const u8 },
    toggle: struct { node: Node, on: bool },
    select: struct { node: Node, index: i32 },
    key: struct { node: Node, sym: u32, mods: u8, text: []const u8 },
    /// kind: 0 down, 1 up, 2 move, 3 wheel, 4 leave.
    pointer: struct { node: Node, kind: u8, x: f32, y: f32, buttons: u8, dx: f32, dy: f32 },
    timer: u32,
    /// status 0 means it failed, and `body` is the error text.
    http: struct { req: u32, status: u16, headers: []const u8, body: []const u8 },
    /// kind: 0 open, 1 text, 2 binary, 3 closed, 4 error.
    ws: struct { ws: u32, kind: u8, data: []const u8 },
    message: struct { from: []const u8, data: []const u8 },
    request_done: struct { req: u32, result: i32, info: []const u8 },
    close,
    activate,
    focus_out: Node,
    near_end: Node,
    link: struct { node: Node, span: u32 },
    double_click: Node,
    unknown,
};

const Reader = struct {
    b: []const u8,
    fn int(r: *Reader, comptime T: type) T {
        const n = @sizeOf(T);
        if (r.b.len < n) {
            r.b = r.b[r.b.len..];
            return 0;
        }
        const v = std.mem.readInt(T, r.b[0..n], .little);
        r.b = r.b[n..];
        return v;
    }
    fn float(r: *Reader) f32 {
        return @bitCast(r.int(u32));
    }
    fn bytes(r: *Reader) []const u8 {
        const n = @min(r.int(u32), r.b.len);
        defer r.b = r.b[n..];
        return r.b[0..n];
    }
};

/// Decodes one event and sets `qa.tag`.
pub fn decode(buf: []const u8) Event {
    var r = Reader{ .b = buf };
    var has_node = true;
    const e: Event = switch (r.int(u8)) {
        1 => blk: {
            has_node = false;
            break :blk .{ .resize = .{ .w = r.float(), .h = r.float(), .scale = r.float() } };
        },
        2 => .{ .click = r.int(u32) },
        3 => .{ .change = .{ .node = r.int(u32), .value = r.bytes() } },
        4 => .{ .submit = .{ .node = r.int(u32), .value = r.bytes() } },
        5 => .{ .toggle = .{ .node = r.int(u32), .on = r.int(u8) != 0 } },
        6 => .{ .select = .{ .node = r.int(u32), .index = r.int(i32) } },
        7 => .{ .key = .{ .node = r.int(u32), .sym = r.int(u32), .mods = r.int(u8), .text = r.bytes() } },
        8 => .{ .pointer = .{ .node = r.int(u32), .kind = r.int(u8), .x = r.float(), .y = r.float(), .buttons = r.int(u8), .dx = r.float(), .dy = r.float() } },
        9 => blk: {
            has_node = false;
            break :blk .{ .timer = r.int(u32) };
        },
        10 => blk: {
            has_node = false;
            break :blk .{ .http = .{ .req = r.int(u32), .status = r.int(u16), .headers = r.bytes(), .body = r.bytes() } };
        },
        11 => blk: {
            has_node = false;
            break :blk .{ .ws = .{ .ws = r.int(u32), .kind = r.int(u8), .data = r.bytes() } };
        },
        12 => blk: {
            has_node = false;
            break :blk .{ .message = .{ .from = r.bytes(), .data = r.bytes() } };
        },
        13 => blk: {
            has_node = false;
            break :blk .{ .request_done = .{ .req = r.int(u32), .result = r.int(i32), .info = r.bytes() } };
        },
        14 => blk: {
            has_node = false;
            break :blk .close;
        },
        15 => blk: {
            has_node = false;
            break :blk .activate;
        },
        16 => .{ .focus_out = r.int(u32) },
        17 => .{ .near_end = r.int(u32) },
        18 => .{ .link = .{ .node = r.int(u32), .span = r.int(u32) } },
        19 => .{ .double_click = r.int(u32) },
        else => blk: {
            has_node = false;
            break :blk .unknown;
        },
    };
    tag = if (has_node) r.int(i32) else 0;
    return e;
}

/// Exports `qa_alloc`, `qa_start` and `qa_event` for a root file that has
/// `pub fn start() void` and `pub fn event(e: qa.Event) void`.
pub fn exportApp(comptime App: type) void {
    const S = struct {
        fn alloc(len: u32) callconv(.C) u32 {
            const b = allocator.alloc(u8, @max(len, 1)) catch @trap();
            return @intFromPtr(b.ptr);
        }
        fn start() callconv(.C) void {
            App.start();
            flush();
        }
        fn event(ptr: [*]u8, len: u32) callconv(.C) void {
            const buf = ptr[0..@max(len, 1)];
            App.event(decode(buf[0..len]));
            flush();
            allocator.free(buf);
        }
    };
    @export(&S.alloc, .{ .name = "qa_alloc" });
    @export(&S.start, .{ .name = "qa_start" });
    @export(&S.event, .{ .name = "qa_event" });
}

// ---- helpers ----

pub fn log(s: []const u8) void {
    sys.log(1, s.ptr, s.len);
}
pub fn setTitle(s: []const u8) void {
    sys.set_title(s.ptr, s.len);
}
/// Fires one `timer` event after `ms`.
pub fn timer(ms: u32) u32 {
    return sys.timer(ms, 0);
}
/// Fires a `timer` event every `ms` until `sys.timer_cancel(id)`.
pub fn interval(ms: u32) u32 {
    return sys.timer(ms, 1);
}

pub const fs = struct {
    pub const app_dir = 0;
    pub const bundle_dir = 1;
    pub const READ = 1;
    pub const WRITE = 2;
    pub const CREATE = 4;
    pub const TRUNC = 8;
    pub const APPEND = 16;
    pub const EXCL = 32;

    /// A whole file. The caller frees it with `qa.allocator`. Null if it can't be read.
    pub fn read(dir: i32, path: []const u8) ?[]u8 {
        const fd = sys.fs_open(dir, path.ptr, path.len, READ);
        if (fd < 0) return null;
        defer _ = sys.fs_close(fd);
        var out: std.ArrayListUnmanaged(u8) = .empty;
        var chunk: [4096]u8 = undefined;
        while (true) {
            const n = sys.fs_read(fd, &chunk, chunk.len);
            if (n < 0) {
                out.deinit(allocator);
                return null;
            }
            if (n == 0) break;
            out.appendSlice(allocator, chunk[0..@intCast(n)]) catch @trap();
        }
        return out.toOwnedSlice(allocator) catch @trap();
    }

    /// Replaces a whole file. False if it can't be written.
    pub fn write(dir: i32, path: []const u8, data: []const u8) bool {
        const fd = sys.fs_open(dir, path.ptr, path.len, WRITE | CREATE | TRUNC);
        if (fd < 0) return false;
        defer _ = sys.fs_close(fd);
        var rest = data;
        while (rest.len > 0) {
            const n = sys.fs_write(fd, rest.ptr, rest.len);
            if (n <= 0) return false;
            rest = rest[@intCast(n)..];
        }
        return true;
    }
};

pub const net = struct {
    pub const Request = struct {
        method: []const u8 = "GET",
        url: []const u8,
        /// "Name: value\n" lines.
        headers: []const u8 = "",
        body: []const u8 = "",
        /// 0 means 30 s.
        timeout_ms: u32 = 0,
        /// 0 means 8 MB.
        max_body: u32 = 0,
    };

    /// Starts a request. The answer is an `http` event with the returned id.
    pub fn request(r: Request) u32 {
        var b: std.ArrayListUnmanaged(u8) = .empty;
        defer b.deinit(allocator);
        for ([_][]const u8{ r.method, r.url, r.headers, r.body }) |s| {
            b.appendSlice(allocator, &std.mem.toBytes(@as(u32, @intCast(s.len)))) catch @trap();
            b.appendSlice(allocator, s) catch @trap();
        }
        b.appendSlice(allocator, &std.mem.toBytes(r.timeout_ms)) catch @trap();
        b.appendSlice(allocator, &std.mem.toBytes(r.max_body)) catch @trap();
        return sys.http(b.items.ptr, b.items.len);
    }
    pub fn get(url: []const u8) u32 {
        return request(.{ .url = url });
    }
};

/// Host calls, exactly as in docs/app-abi.md.
pub const sys = struct {
    pub extern "qa" fn ui_patch(ptr: [*]const u8, len: usize) i32;
    pub extern "qa" fn canvas_put(node: u32, x: u32, y: u32, w: u32, h: u32, rgba: [*]const u8) i32;
    pub extern "qa" fn set_title(ptr: [*]const u8, len: usize) void;
    pub extern "qa" fn quit() void;
    pub extern "qa" fn env(out: [*]u8, cap: usize) i32;
    pub extern "qa" fn now_ms() i64;
    pub extern "qa" fn mono_ms() i64;
    pub extern "qa" fn random(ptr: [*]u8, len: usize) void;
    pub extern "qa" fn log(level: u32, ptr: [*]const u8, len: usize) void;
    pub extern "qa" fn timer(ms: u32, repeat: u32) u32;
    pub extern "qa" fn timer_cancel(id: u32) void;
    pub extern "qa" fn json_parse(ptr: [*]const u8, len: usize, out: [*]u8, cap: usize, needed: *u32) i32;
    pub extern "qa" fn fs_open(dir: i32, path: [*]const u8, len: usize, flags: u32) i32;
    pub extern "qa" fn fs_read(fd: i32, ptr: [*]u8, len: usize) i32;
    pub extern "qa" fn fs_write(fd: i32, ptr: [*]const u8, len: usize) i32;
    pub extern "qa" fn fs_seek(fd: i32, off: i64, whence: u32) i64;
    pub extern "qa" fn fs_close(fd: i32) i32;
    pub extern "qa" fn fs_stat(dir: i32, path: [*]const u8, len: usize, out: [*]u8) i32;
    pub extern "qa" fn fs_list(dir: i32, path: [*]const u8, len: usize, out: [*]u8, cap: usize, needed: *u32) i32;
    pub extern "qa" fn fs_mkdir(dir: i32, path: [*]const u8, len: usize) i32;
    pub extern "qa" fn fs_remove(dir: i32, path: [*]const u8, len: usize) i32;
    pub extern "qa" fn fs_rename(dir: i32, from: [*]const u8, from_len: usize, dir2: i32, to: [*]const u8, to_len: usize) i32;
    pub extern "qa" fn request(kind: u32, arg: [*]const u8, len: usize) u32;
    pub extern "qa" fn http(ptr: [*]const u8, len: usize) u32;
    pub extern "qa" fn http_cancel(req: u32) void;
    pub extern "qa" fn ws_open(ptr: [*]const u8, len: usize) u32;
    pub extern "qa" fn ws_send(ws: u32, kind: u32, ptr: [*]const u8, len: usize) i32;
    pub extern "qa" fn ws_close(ws: u32, code: u32) void;
    pub extern "qa" fn msg_send(to: [*]const u8, to_len: usize, data: [*]const u8, len: usize) i32;
    pub extern "qa" fn clipboard_write(ptr: [*]const u8, len: usize) i32;
    pub extern "qa" fn open_url(ptr: [*]const u8, len: usize) i32;
    pub extern "qa" fn secret_get(key: [*]const u8, key_len: usize, out: [*]u8, cap: usize, needed: *u32) i32;
    pub extern "qa" fn secret_set(key: [*]const u8, key_len: usize, val: [*]const u8, val_len: usize) i32;
};
