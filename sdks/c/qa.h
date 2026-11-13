/* Quick Apps SDK for C. One header, no libc.
 *
 * In exactly one .c file:
 *     #define QA_IMPL
 *     #include "qa.h"
 * and define:
 *     void app_start(void);               build the first tree
 *     void app_event(const qa_event *e);  handle one event
 *
 * Build (clang with wasm-ld, or zig cc):
 *     zig cc -target wasm32-freestanding -Oz -nostdlib -Wl,--no-entry -o app.wasm app.c
 * or run build.sh, which picks whichever is installed.
 *
 * Calls build up one patch, which is sent when app_start/app_event returns.
 * Strings are (pointer, length) pairs; the `_s` helpers take NUL-terminated
 * ones. Strings in an event point into the event buffer and are NOT
 * NUL-terminated: copy what you keep. The wire format is in docs/app-abi.md.
 */
#ifndef QA_H
#define QA_H

typedef unsigned int qa_node;
typedef unsigned int qa_u32;
typedef unsigned char qa_u8;

/* ---- ids (docs/ui-tree.md, docs/components.md, docs/events.md) ---- */
enum { QA_GRID = 1, QA_TEXT = 2, QA_IMAGE = 3, QA_CANVAS = 4, QA_SCROLL = 5, QA_BUTTON = 10, QA_INPUT = 11,
       QA_CHECKBOX = 12, QA_SELECT = 13, QA_TABS = 14, QA_PROGRESS = 15, QA_DIVIDER = 16 };

enum { /* common props */
       QA_ROW = 1, QA_COL = 2, QA_ROW_SPAN = 3, QA_COL_SPAN = 4, QA_PRIORITY = 5, QA_MIN_W = 6, QA_MIN_H = 7,
       QA_MAX_W = 8, QA_MAX_H = 9, QA_PAD = 10, QA_ALIGN_X = 11, QA_ALIGN_Y = 12, QA_HIDDEN = 13, QA_VARIANT = 14,
       QA_TOOLTIP = 15, QA_TAG = 16, QA_DISABLED = 17,
       /* grid */ QA_GRID_COLS = 20, QA_GRID_ROWS = 21, QA_GRID_GAP = 22, QA_GRID_CLICKABLE = 23, QA_GRID_SELECTED = 24, QA_GRID_FLOW = 25,
       /* text */ QA_TEXT_TEXT = 20, QA_TEXT_WRAP = 21, QA_TEXT_STYLE = 22, QA_TEXT_ALIGN = 23, QA_TEXT_SELECTABLE = 24, QA_TEXT_MAX_LINES = 25, QA_TEXT_SPANS = 26,
       /* image */ QA_IMAGE_SRC = 20, QA_IMAGE_DATA = 21, QA_IMAGE_FIT = 22,
       /* canvas */ QA_CANVAS_PX_W = 20, QA_CANVAS_PX_H = 21, QA_CANVAS_FIT = 22, QA_CANVAS_FOCUSABLE = 23,
       /* scroll */ QA_SCROLL_AXIS = 20, QA_SCROLL_STICK_END = 21,
       /* button */ QA_BUTTON_LABEL = 20,
       /* input */ QA_INPUT_VALUE = 20, QA_INPUT_PLACEHOLDER = 21, QA_INPUT_MULTILINE = 22, QA_INPUT_SECRET = 23, QA_INPUT_LINES = 25,
       /* checkbox */ QA_CHECKBOX_LABEL = 20, QA_CHECKBOX_CHECKED = 21, QA_CHECKBOX_WRAP = 23,
       /* select */ QA_SELECT_OPTIONS = 20, QA_SELECT_SELECTED = 21, QA_SELECT_PLACEHOLDER = 22,
       /* tabs */ QA_TABS_LABELS = 20, QA_TABS_SELECTED = 21,
       /* progress */ QA_PROGRESS_VALUE = 20 };

enum { QA_STRETCH = 0, QA_START = 1, QA_CENTER = 2, QA_END = 3 };

enum { QA_EV_RESIZE = 1, QA_EV_CLICK = 2, QA_EV_CHANGE = 3, QA_EV_SUBMIT = 4, QA_EV_TOGGLE = 5, QA_EV_SELECT = 6,
       QA_EV_KEY = 7, QA_EV_POINTER = 8, QA_EV_TIMER = 9, QA_EV_HTTP = 10, QA_EV_WS = 11, QA_EV_MESSAGE = 12,
       QA_EV_REQUEST_DONE = 13, QA_EV_CLOSE = 14, QA_EV_ACTIVATE = 15, QA_EV_FOCUS_OUT = 16, QA_EV_NEAR_END = 17,
       QA_EV_LINK = 18, QA_EV_DOUBLE_CLICK = 19 };

enum { QA_KEY_BACKSPACE = 0xff08, QA_KEY_TAB = 0xff09, QA_KEY_RETURN = 0xff0d, QA_KEY_ESCAPE = 0xff1b,
       QA_KEY_DELETE = 0xffff, QA_KEY_LEFT = 0xff51, QA_KEY_UP = 0xff52, QA_KEY_RIGHT = 0xff53, QA_KEY_DOWN = 0xff54 };
enum { QA_MOD_SHIFT = 1, QA_MOD_CTRL = 2, QA_MOD_ALT = 4, QA_MOD_SUPER = 8 };

enum { QA_ERR_DENIED = -1, QA_ERR_NOT_FOUND = -2, QA_ERR_EXISTS = -3, QA_ERR_INVALID = -4, QA_ERR_IO = -5,
       QA_ERR_LIMIT = -6, QA_ERR_BAD_HANDLE = -7, QA_ERR_TOO_SMALL = -8 };

/* fs: folders and open flags (docs/io.md) */
enum { QA_APP_DIR = 0, QA_BUNDLE_DIR = 1 };
enum { QA_READ = 1, QA_WRITE = 2, QA_CREATE = 4, QA_TRUNC = 8, QA_APPEND = 16, QA_EXCL = 32 };

/* ---- events ---- */
/* One decoded event. Which fields are set depends on `type`:
 *   RESIZE        w, h, scale
 *   CLICK, DOUBLE_CLICK, FOCUS_OUT, NEAR_END   node
 *   CHANGE, SUBMIT   node, text
 *   TOGGLE        node, value (0/1)
 *   SELECT        node, value (index)
 *   LINK          node, value (span index)
 *   KEY           node (focused), sym, mods, text
 *   POINTER       node, kind (0 down 1 up 2 move 3 wheel 4 leave), x, y, buttons, dx, dy
 *   TIMER         id
 *   HTTP          id (request), status (0 = failed, data holds the error), text (headers), data (body)
 *   WS            id (socket), kind (0 open 1 text 2 binary 3 closed 4 error), data
 *   MESSAGE       text (sender's manifest URL), data
 *   REQUEST_DONE  id (request), value (result), text (info)
 * `tag` is the node's tag prop for every event that has a node. */
typedef struct {
    int type;
    qa_node node;
    int tag;
    int value;
    qa_u32 id;
    const char *text;
    qa_u32 text_len;
    const qa_u8 *data;
    qa_u32 data_len;
    qa_u32 sym;
    int mods, kind, buttons, status;
    float x, y, dx, dy, w, h, scale;
} qa_event;

void app_start(void);
void app_event(const qa_event *e);

/* ---- tree ---- */
typedef struct { qa_u8 kind; float value, min, max; } qa_track;
qa_track qa_px(float v);
qa_track qa_fr(float v);
qa_track qa_auto(void);

qa_node qa_create(int kind);
qa_node qa_grid(void);
qa_node qa_text(const char *s);
qa_node qa_button(const char *label);
qa_node qa_input(const char *placeholder);
qa_node qa_checkbox(const char *label, int checked);
qa_node qa_select(const char *options); /* one per line */
qa_node qa_tabs(const char *labels);    /* one per line */
qa_node qa_scroll(void);
qa_node qa_image(const char *src);
qa_node qa_canvas(int px_w, int px_h);
qa_node qa_progress(float v);
qa_node qa_divider(void);

void qa_root(qa_node n);
qa_node qa_add(qa_node parent, qa_node child); /* appends; returns child */
void qa_insert(qa_node parent, qa_node child, qa_node before);
void qa_detach(qa_node n);
void qa_destroy(qa_node n);
void qa_replace(qa_node old, qa_node with);
void qa_focus(qa_node n);
void qa_scroll_to(qa_node n);

void qa_set_i(qa_node n, int prop, int v);
void qa_set_f(qa_node n, int prop, float v);
void qa_set_str(qa_node n, int prop, const char *s, qa_u32 len);
void qa_set_s(qa_node n, int prop, const char *s);
void qa_set_bytes(qa_node n, int prop, const void *p, qa_u32 len);
void qa_set_tracks(qa_node n, int prop, const qa_track *t, int count);
void qa_reset(qa_node n, int prop);
void qa_pad(qa_node n, float all);
void qa_pad4(qa_node n, float top, float right, float bottom, float left);
/* Theme tokens for a subtree (docs/theming.md). */
void qa_ctx_color(qa_node n, const char *key, qa_u32 rrggbbaa);
void qa_ctx_f(qa_node n, const char *key, float v);
void qa_ctx_s(qa_node n, const char *key, const char *v);

/* Shorthands. */
#define qa_cols(n, ...) qa_set_tracks((n), QA_GRID_COLS, (qa_track[]){__VA_ARGS__}, sizeof((qa_track[]){__VA_ARGS__}) / sizeof(qa_track))
#define qa_rows(n, ...) qa_set_tracks((n), QA_GRID_ROWS, (qa_track[]){__VA_ARGS__}, sizeof((qa_track[]){__VA_ARGS__}) / sizeof(qa_track))
#define qa_gap(n, v) qa_set_f((n), QA_GRID_GAP, (v))
#define qa_variant(n, s) qa_set_s((n), QA_VARIANT, (s))
#define qa_style(n, s) qa_set_s((n), QA_TEXT_STYLE, (s))
#define qa_tag(n, t) qa_set_i((n), QA_TAG, (t))
#define qa_hide(n, on) qa_set_i((n), QA_HIDDEN, (on))
#define qa_disable(n, on) qa_set_i((n), QA_DISABLED, (on))
#define qa_align_x(n, a) qa_set_i((n), QA_ALIGN_X, (a))
#define qa_align_y(n, a) qa_set_i((n), QA_ALIGN_Y, (a))
#define qa_set_text(n, s) qa_set_s((n), QA_TEXT_TEXT, (s))
#define qa_set_label(n, s) qa_set_s((n), QA_BUTTON_LABEL, (s))
#define qa_set_value(n, s) qa_set_s((n), QA_INPUT_VALUE, (s))
#define qa_set_checked(n, on) qa_set_i((n), QA_CHECKBOX_CHECKED, (on))

/* Sends the patch now. Done for you after app_start and app_event. */
void qa_flush(void);

/* ---- helpers ---- */
/* Memory that is never freed (there is no malloc here). Bring your own
 * allocator if you need one. */
void *qa_bump(qa_u32 len);
qa_u32 qa_strlen(const char *s);
/* Writes v in decimal and a NUL into buf (12 bytes is enough); returns buf. */
char *qa_itoa(char *buf, int v);
void qa_log(const char *s);
void qa_title(const char *s);
qa_u32 qa_timer(qa_u32 ms);    /* fires once */
qa_u32 qa_interval(qa_u32 ms); /* fires until qa_sys_timer_cancel */
qa_u32 qa_http_get(const char *url);
/* Whole files. Read returns the byte count or a negative error. */
int qa_fs_read(int dir, const char *path, void *buf, qa_u32 cap);
int qa_fs_write(int dir, const char *path, const void *data, qa_u32 len);

/* ---- host calls, as in docs/app-abi.md ---- */
#define QA_IMPORT(name) __attribute__((import_module("qa"), import_name(#name)))
QA_IMPORT(ui_patch) int qa_sys_ui_patch(const void *p, qa_u32 len);
QA_IMPORT(canvas_put) int qa_sys_canvas_put(qa_node n, qa_u32 x, qa_u32 y, qa_u32 w, qa_u32 h, const void *rgba);
QA_IMPORT(set_title) void qa_sys_set_title(const char *p, qa_u32 len);
QA_IMPORT(quit) void qa_sys_quit(void);
QA_IMPORT(env) int qa_sys_env(void *out, qa_u32 cap);
QA_IMPORT(now_ms) long long qa_sys_now_ms(void);
QA_IMPORT(mono_ms) long long qa_sys_mono_ms(void);
QA_IMPORT(random) void qa_sys_random(void *p, qa_u32 len);
QA_IMPORT(log) void qa_sys_log(qa_u32 level, const char *p, qa_u32 len);
QA_IMPORT(timer) qa_u32 qa_sys_timer(qa_u32 ms, qa_u32 repeat);
QA_IMPORT(timer_cancel) void qa_sys_timer_cancel(qa_u32 id);
QA_IMPORT(json_parse) int qa_sys_json_parse(const void *p, qa_u32 len, void *out, qa_u32 cap, qa_u32 *needed);
QA_IMPORT(fs_open) int qa_sys_fs_open(int dir, const char *path, qa_u32 len, qa_u32 flags);
QA_IMPORT(fs_read) int qa_sys_fs_read(int fd, void *p, qa_u32 len);
QA_IMPORT(fs_write) int qa_sys_fs_write(int fd, const void *p, qa_u32 len);
QA_IMPORT(fs_seek) long long qa_sys_fs_seek(int fd, long long off, qa_u32 whence);
QA_IMPORT(fs_close) int qa_sys_fs_close(int fd);
QA_IMPORT(fs_stat) int qa_sys_fs_stat(int dir, const char *path, qa_u32 len, void *out);
QA_IMPORT(fs_list) int qa_sys_fs_list(int dir, const char *path, qa_u32 len, void *out, qa_u32 cap, qa_u32 *needed);
QA_IMPORT(fs_mkdir) int qa_sys_fs_mkdir(int dir, const char *path, qa_u32 len);
QA_IMPORT(fs_remove) int qa_sys_fs_remove(int dir, const char *path, qa_u32 len);
QA_IMPORT(fs_rename) int qa_sys_fs_rename(int dir, const char *from, qa_u32 from_len, int dir2, const char *to, qa_u32 to_len);
QA_IMPORT(request) qa_u32 qa_sys_request(qa_u32 kind, const void *arg, qa_u32 len);
QA_IMPORT(http) qa_u32 qa_sys_http(const void *p, qa_u32 len);
QA_IMPORT(http_cancel) void qa_sys_http_cancel(qa_u32 req);
QA_IMPORT(ws_open) qa_u32 qa_sys_ws_open(const void *p, qa_u32 len);
QA_IMPORT(ws_send) int qa_sys_ws_send(qa_u32 ws, qa_u32 kind, const void *p, qa_u32 len);
QA_IMPORT(ws_close) void qa_sys_ws_close(qa_u32 ws, qa_u32 code);
QA_IMPORT(msg_send) int qa_sys_msg_send(const char *to, qa_u32 to_len, const void *data, qa_u32 len);
QA_IMPORT(clipboard_write) int qa_sys_clipboard_write(const char *p, qa_u32 len);
QA_IMPORT(open_url) int qa_sys_open_url(const char *p, qa_u32 len);
QA_IMPORT(secret_get) int qa_sys_secret_get(const char *key, qa_u32 key_len, void *out, qa_u32 cap, qa_u32 *needed);
QA_IMPORT(secret_set) int qa_sys_secret_set(const char *key, qa_u32 key_len, const char *val, qa_u32 val_len);

#ifdef QA_IMPL

/* The compiler may call these even with no libc. */
void *memcpy(void *d, const void *s, unsigned long n) {
    char *a = d;
    const char *b = s;
    while (n--) *a++ = *b++;
    return d;
}
void *memset(void *d, int c, unsigned long n) {
    char *a = d;
    while (n--) *a++ = (char)c;
    return d;
}

qa_u32 qa_strlen(const char *s) {
    qa_u32 n = 0;
    while (s[n]) n++;
    return n;
}

/* Memory: a bump pointer from the end of the module's data, growing the wasm
 * memory as needed. */
extern unsigned char __heap_base;
static qa_u32 qa__top;
void *qa_bump(qa_u32 len) {
    if (!qa__top) qa__top = (qa_u32)&__heap_base;
    qa_u32 p = (qa__top + 7) & ~7u;
    qa_u32 end = p + len;
    qa_u32 have = __builtin_wasm_memory_size(0) << 16;
    if (end > have && __builtin_wasm_memory_grow(0, (end - have + 0xffff) >> 16) == (unsigned long)-1) __builtin_trap();
    qa__top = end;
    return (void *)p;
}

/* A buffer that doubles. The old one is left behind, which wastes at most
 * as much again in total. */
typedef struct { qa_u8 *p; qa_u32 len, cap; } qa__buf;
static void qa__room(qa__buf *b, qa_u32 more) {
    if (b->len + more <= b->cap) return;
    qa_u32 cap = b->cap ? b->cap * 2 : 1024;
    while (cap < b->len + more) cap *= 2;
    qa_u8 *p = qa_bump(cap);
    memcpy(p, b->p, b->len);
    b->p = p;
    b->cap = cap;
}

static qa__buf qa__patch, qa__ev;
static qa_u32 qa__next;

static void qa__put(const void *p, qa_u32 n) {
    qa__room(&qa__patch, n);
    memcpy(qa__patch.p + qa__patch.len, p, n);
    qa__patch.len += n;
}
static void qa__u8(qa_u8 v) { qa__put(&v, 1); }
static void qa__u16(unsigned short v) { qa__put(&v, 2); }
static void qa__u32(qa_u32 v) { qa__put(&v, 4); }
static void qa__f32(float v) { qa__put(&v, 4); }
static void qa__str(const char *s, qa_u32 n) {
    qa__u32(n);
    qa__put(s, n);
}
static void qa__op1(qa_u8 op, qa_u32 a) {
    qa__u8(op);
    qa__u32(a);
}
static void qa__op2(qa_u8 op, qa_u32 a, qa_u32 b) {
    qa__op1(op, a);
    qa__u32(b);
}
static void qa__set(qa_node n, int prop, qa_u8 tag) {
    qa__op1(6, n);
    qa__u16((unsigned short)prop);
    qa__u8(tag);
}
static void qa__ctx(qa_node n, const char *key, qa_u8 tag) {
    qa__op1(7, n);
    qa__str(key, qa_strlen(key));
    qa__u8(tag);
}

void qa_flush(void) {
    if (qa__patch.len) qa_sys_ui_patch(qa__patch.p, qa__patch.len);
    qa__patch.len = 0;
}

qa_track qa_px(float v) { return (qa_track){0, v, __builtin_inff(), __builtin_inff()}; }
qa_track qa_fr(float v) { return (qa_track){1, v, __builtin_inff(), __builtin_inff()}; }
qa_track qa_auto(void) { return (qa_track){2, 0, __builtin_inff(), __builtin_inff()}; }

qa_node qa_create(int kind) {
    qa_node n = ++qa__next;
    qa__op1(1, n);
    qa__u8((qa_u8)kind);
    return n;
}
static qa_node qa__with(int kind, int prop, const char *s) {
    qa_node n = qa_create(kind);
    if (s && *s) qa_set_s(n, prop, s);
    return n;
}
qa_node qa_grid(void) { return qa_create(QA_GRID); }
qa_node qa_text(const char *s) { return qa__with(QA_TEXT, QA_TEXT_TEXT, s); }
qa_node qa_button(const char *s) { return qa__with(QA_BUTTON, QA_BUTTON_LABEL, s); }
qa_node qa_input(const char *s) { return qa__with(QA_INPUT, QA_INPUT_PLACEHOLDER, s); }
qa_node qa_checkbox(const char *s, int checked) {
    qa_node n = qa__with(QA_CHECKBOX, QA_CHECKBOX_LABEL, s);
    if (checked) qa_set_i(n, QA_CHECKBOX_CHECKED, 1);
    return n;
}
qa_node qa_select(const char *s) { return qa__with(QA_SELECT, QA_SELECT_OPTIONS, s); }
qa_node qa_tabs(const char *s) { return qa__with(QA_TABS, QA_TABS_LABELS, s); }
qa_node qa_scroll(void) { return qa_create(QA_SCROLL); }
qa_node qa_image(const char *s) { return qa__with(QA_IMAGE, QA_IMAGE_SRC, s); }
qa_node qa_canvas(int w, int h) {
    qa_node n = qa_create(QA_CANVAS);
    qa_set_i(n, QA_CANVAS_PX_W, w);
    qa_set_i(n, QA_CANVAS_PX_H, h);
    return n;
}
qa_node qa_progress(float v) {
    qa_node n = qa_create(QA_PROGRESS);
    qa_set_f(n, QA_PROGRESS_VALUE, v);
    return n;
}
qa_node qa_divider(void) { return qa_create(QA_DIVIDER); }

void qa_root(qa_node n) { qa__op1(8, n); }
qa_node qa_add(qa_node parent, qa_node child) {
    qa__op2(3, parent, child);
    return child;
}
void qa_insert(qa_node parent, qa_node child, qa_node before) {
    qa__op2(4, parent, child);
    qa__u32(before);
}
void qa_detach(qa_node n) { qa__op1(5, n); }
void qa_destroy(qa_node n) { qa__op1(2, n); }
void qa_replace(qa_node old, qa_node with) { qa__op2(11, old, with); }
void qa_focus(qa_node n) { qa__op1(9, n); }
void qa_scroll_to(qa_node n) { qa__op1(10, n); }

void qa_set_i(qa_node n, int prop, int v) {
    qa__set(n, prop, 1);
    qa__u32((qa_u32)v);
}
void qa_set_f(qa_node n, int prop, float v) {
    qa__set(n, prop, 2);
    qa__f32(v);
}
void qa_set_str(qa_node n, int prop, const char *s, qa_u32 len) {
    qa__set(n, prop, 3);
    qa__str(s, len);
}
void qa_set_s(qa_node n, int prop, const char *s) { qa_set_str(n, prop, s, qa_strlen(s)); }
void qa_set_bytes(qa_node n, int prop, const void *p, qa_u32 len) {
    qa__set(n, prop, 5);
    qa__str(p, len);
}
void qa_set_tracks(qa_node n, int prop, const qa_track *t, int count) {
    qa__set(n, prop, 6);
    qa__u8((qa_u8)count);
    for (int i = 0; i < count; i++) {
        qa__u8(t[i].kind);
        qa__f32(t[i].value);
        qa__f32(t[i].min);
        qa__f32(t[i].max);
    }
}
void qa_reset(qa_node n, int prop) { qa__set(n, prop, 0); }
void qa_pad4(qa_node n, float top, float right, float bottom, float left) {
    qa__set(n, QA_PAD, 7);
    qa__f32(top);
    qa__f32(right);
    qa__f32(bottom);
    qa__f32(left);
}
void qa_pad(qa_node n, float all) { qa_pad4(n, all, all, all, all); }
void qa_ctx_color(qa_node n, const char *key, qa_u32 c) {
    qa__ctx(n, key, 4);
    qa__u32(c);
}
void qa_ctx_f(qa_node n, const char *key, float v) {
    qa__ctx(n, key, 2);
    qa__f32(v);
}
void qa_ctx_s(qa_node n, const char *key, const char *v) {
    qa__ctx(n, key, 3);
    qa__str(v, qa_strlen(v));
}

char *qa_itoa(char *buf, int v) {
    char t[12];
    int i = 0, j = 0;
    unsigned u = v < 0 ? 0u - (unsigned)v : (unsigned)v;
    do t[i++] = (char)('0' + u % 10); while (u /= 10);
    if (v < 0) buf[j++] = '-';
    while (i) buf[j++] = t[--i];
    buf[j] = 0;
    return buf;
}
void qa_log(const char *s) { qa_sys_log(1, s, qa_strlen(s)); }
void qa_title(const char *s) { qa_sys_set_title(s, qa_strlen(s)); }
qa_u32 qa_timer(qa_u32 ms) { return qa_sys_timer(ms, 0); }
qa_u32 qa_interval(qa_u32 ms) { return qa_sys_timer(ms, 1); }

qa_u32 qa_http_get(const char *url) {
    /* str method, str url, str headers, bytes body, u32 timeout_ms, u32 max_body */
    qa_u32 n = qa_strlen(url);
    qa_u8 *b = qa_bump(n + 27), *p = b;
    qa_u32 three = 3, zero = 0;
    memcpy(p, &three, 4), memcpy(p + 4, "GET", 3), p += 7;
    memcpy(p, &n, 4), memcpy(p + 4, url, n), p += 4 + n;
    for (int i = 0; i < 4; i++) memcpy(p, &zero, 4), p += 4;
    return qa_sys_http(b, (qa_u32)(p - b));
}

int qa_fs_read(int dir, const char *path, void *buf, qa_u32 cap) {
    int fd = qa_sys_fs_open(dir, path, qa_strlen(path), QA_READ);
    if (fd < 0) return fd;
    qa_u32 got = 0;
    int n = 0;
    while (got < cap && (n = qa_sys_fs_read(fd, (char *)buf + got, cap - got)) > 0) got += (qa_u32)n;
    qa_sys_fs_close(fd);
    return n < 0 ? n : (int)got;
}
int qa_fs_write(int dir, const char *path, const void *data, qa_u32 len) {
    int fd = qa_sys_fs_open(dir, path, qa_strlen(path), QA_WRITE | QA_CREATE | QA_TRUNC);
    if (fd < 0) return fd;
    qa_u32 put = 0;
    int n = 0;
    while (put < len && (n = qa_sys_fs_write(fd, (const char *)data + put, len - put)) > 0) put += (qa_u32)n;
    qa_sys_fs_close(fd);
    return n < 0 ? n : 0;
}

/* ---- exports ---- */
/* Each event is copied into one reused buffer, so nothing needs freeing. */
__attribute__((export_name("qa_alloc"))) qa_u32 qa__x_alloc(qa_u32 len) {
    qa__ev.len = 0;
    qa__room(&qa__ev, len ? len : 1);
    return (qa_u32)qa__ev.p;
}

__attribute__((export_name("qa_start"))) void qa__x_start(void) {
    app_start();
    qa_flush();
}

typedef struct { const qa_u8 *p, *end; } qa__rd;
static qa_u32 qa__r32(qa__rd *r) {
    qa_u32 v = 0;
    if (r->end - r->p >= 4) memcpy(&v, r->p, 4), r->p += 4;
    else r->p = r->end;
    return v;
}
static float qa__rf(qa__rd *r) {
    qa_u32 v = qa__r32(r);
    float f;
    memcpy(&f, &v, 4);
    return f;
}
static int qa__r8(qa__rd *r) { return r->p < r->end ? *r->p++ : 0; }
static const qa_u8 *qa__rbytes(qa__rd *r, qa_u32 *len) {
    qa_u32 n = qa__r32(r);
    if (n > (qa_u32)(r->end - r->p)) n = (qa_u32)(r->end - r->p);
    const qa_u8 *p = r->p;
    r->p += n;
    *len = n;
    return p;
}
#define QA__TEXT(e, r) ((e).text = (const char *)qa__rbytes((r), &(e).text_len))

__attribute__((export_name("qa_event"))) void qa__x_event(const qa_u8 *p, qa_u32 len) {
    qa__rd r = {p, p + len};
    qa_event e;
    memset(&e, 0, sizeof e);
    e.text = "";
    e.type = qa__r8(&r);
    int has_node = 1;
    switch (e.type) {
    case QA_EV_RESIZE: e.w = qa__rf(&r), e.h = qa__rf(&r), e.scale = qa__rf(&r), has_node = 0; break;
    case QA_EV_CLICK: case QA_EV_DOUBLE_CLICK: case QA_EV_FOCUS_OUT: case QA_EV_NEAR_END: e.node = qa__r32(&r); break;
    case QA_EV_CHANGE: case QA_EV_SUBMIT: e.node = qa__r32(&r), QA__TEXT(e, &r); break;
    case QA_EV_TOGGLE: e.node = qa__r32(&r), e.value = qa__r8(&r); break;
    case QA_EV_SELECT: case QA_EV_LINK: e.node = qa__r32(&r), e.value = (int)qa__r32(&r); break;
    case QA_EV_KEY: e.node = qa__r32(&r), e.sym = qa__r32(&r), e.mods = qa__r8(&r), QA__TEXT(e, &r); break;
    case QA_EV_POINTER:
        e.node = qa__r32(&r), e.kind = qa__r8(&r), e.x = qa__rf(&r), e.y = qa__rf(&r);
        e.buttons = qa__r8(&r), e.dx = qa__rf(&r), e.dy = qa__rf(&r);
        break;
    case QA_EV_TIMER: e.id = qa__r32(&r), has_node = 0; break;
    case QA_EV_HTTP:
        e.id = qa__r32(&r), e.status = qa__r8(&r), e.status |= qa__r8(&r) << 8, QA__TEXT(e, &r);
        e.data = qa__rbytes(&r, &e.data_len), has_node = 0;
        break;
    case QA_EV_WS: e.id = qa__r32(&r), e.kind = qa__r8(&r), e.data = qa__rbytes(&r, &e.data_len), has_node = 0; break;
    case QA_EV_MESSAGE: QA__TEXT(e, &r), e.data = qa__rbytes(&r, &e.data_len), has_node = 0; break;
    case QA_EV_REQUEST_DONE: e.id = qa__r32(&r), e.value = (int)qa__r32(&r), QA__TEXT(e, &r), has_node = 0; break;
    default: has_node = 0;
    }
    if (has_node) e.tag = (int)qa__r32(&r);
    app_event(&e);
    qa_flush();
}

#endif /* QA_IMPL */
#endif /* QA_H */
