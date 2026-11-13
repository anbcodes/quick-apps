// The Cocoa side of the macOS platform. Rust calls these C functions from the main thread
// and never sees an Objective-C object. Events come out through qa_poll as plain structs.

#import <Cocoa/Cocoa.h>
#import <QuartzCore/QuartzCore.h>
#include <poll.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    int kind;
    double a, b, c;
    unsigned button;
    unsigned sym;
    unsigned mods;
    int pressed;
    int repeat;
    char text[16];
} QaEv;

enum { EV_RESIZE = 1, EV_CLOSE, EV_MOVE, EV_LEAVE, EV_BUTTON, EV_SCROLL, EV_KEY, EV_MODS, EV_FOCUS_IN, EV_FOCUS_OUT };

enum { MOD_SHIFT = 1, MOD_CTRL = 2, MOD_ALT = 4 };

static QaEv *queue;
static int qlen, qcap;
static NSWindow *window;
static NSView *view;
static int cursorKind;

static void push(QaEv e) {
    if (qlen == qcap) {
        qcap = qcap ? qcap * 2 : 64;
        queue = realloc(queue, qcap * sizeof(QaEv));
    }
    queue[qlen++] = e;
}

static double scaleFactor(void) { return window ? window.backingScaleFactor : 1.0; }

static unsigned modsOf(NSEventModifierFlags f) {
    unsigned m = 0;
    if (f & NSEventModifierFlagShift) m |= MOD_SHIFT;
    // Command does what Ctrl does elsewhere (copy, paste, select all).
    if (f & (NSEventModifierFlagCommand | NSEventModifierFlagControl)) m |= MOD_CTRL;
    if (f & NSEventModifierFlagOption) m |= MOD_ALT;
    return m;
}

// X11 keysyms for the keys that have no text.
static unsigned symForKeyCode(unsigned short k) {
    switch (k) {
    case 36: return 0xff0d;
    case 76: return 0xff8d;
    case 48: return 0xff09;
    case 49: return 0x20;
    case 51: return 0xff08;
    case 53: return 0xff1b;
    case 117: return 0xffff;
    case 115: return 0xff50;
    case 119: return 0xff57;
    case 116: return 0xff55;
    case 121: return 0xff56;
    case 123: return 0xff51;
    case 124: return 0xff53;
    case 125: return 0xff54;
    case 126: return 0xff52;
    }
    return 0;
}

@interface QaView : NSView <NSWindowDelegate>
@end

@implementation QaView
- (BOOL)isFlipped { return YES; }
- (BOOL)acceptsFirstResponder { return YES; }
- (BOOL)acceptsFirstMouse:(NSEvent *)e { return YES; }
- (BOOL)wantsUpdateLayer { return YES; }

- (void)pushSize {
    double s = scaleFactor();
    NSSize z = self.bounds.size;
    QaEv e = {.kind = EV_RESIZE, .a = z.width * s, .b = z.height * s, .c = s};
    push(e);
}
- (void)setFrameSize:(NSSize)s {
    [super setFrameSize:s];
    [self pushSize];
}
- (void)viewDidChangeBackingProperties {
    [super viewDidChangeBackingProperties];
    self.layer.contentsScale = scaleFactor();
    [self pushSize];
}
- (void)updateTrackingAreas {
    [super updateTrackingAreas];
    for (NSTrackingArea *a in self.trackingAreas) [self removeTrackingArea:a];
    [self addTrackingArea:[[NSTrackingArea alloc] initWithRect:NSZeroRect
                                                       options:NSTrackingMouseMoved | NSTrackingMouseEnteredAndExited | NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect
                                                         owner:self
                                                      userInfo:nil]];
}
- (void)resetCursorRects {
    NSCursor *c;
    switch (cursorKind) {
    case 1: c = NSCursor.pointingHandCursor; break;
    case 2: c = NSCursor.IBeamCursor; break;
    case 3: c = NSCursor.openHandCursor; break;
    case 4: c = NSCursor.operationNotAllowedCursor; break;
    case 5: case 7: case 8: c = NSCursor.resizeLeftRightCursor; break;
    case 6: c = NSCursor.resizeUpDownCursor; break;
    default: c = NSCursor.arrowCursor;
    }
    [self addCursorRect:self.bounds cursor:c];
}

- (NSPoint)pos:(NSEvent *)e {
    NSPoint p = [self convertPoint:e.locationInWindow fromView:nil];
    double s = scaleFactor();
    return NSMakePoint(p.x * s, p.y * s);
}
- (void)move:(NSEvent *)e {
    NSPoint p = [self pos:e];
    QaEv ev = {.kind = EV_MOVE, .a = p.x, .b = p.y};
    push(ev);
}
- (void)mouseMoved:(NSEvent *)e { [self move:e]; }
- (void)mouseDragged:(NSEvent *)e { [self move:e]; }
- (void)rightMouseDragged:(NSEvent *)e { [self move:e]; }
- (void)otherMouseDragged:(NSEvent *)e { [self move:e]; }
- (void)mouseExited:(NSEvent *)e {
    QaEv ev = {.kind = EV_LEAVE};
    push(ev);
}
- (void)button:(NSEvent *)e pressed:(int)down {
    NSPoint p = [self pos:e];
    unsigned b = 0x110;
    if (e.type == NSEventTypeRightMouseDown || e.type == NSEventTypeRightMouseUp) b = 0x111;
    else if (e.type == NSEventTypeOtherMouseDown || e.type == NSEventTypeOtherMouseUp) b = 0x112;
    QaEv ev = {.kind = EV_BUTTON, .a = p.x, .b = p.y, .button = b, .pressed = down};
    push(ev);
}
- (void)mouseDown:(NSEvent *)e { [self button:e pressed:1]; }
- (void)mouseUp:(NSEvent *)e { [self button:e pressed:0]; }
- (void)rightMouseDown:(NSEvent *)e { [self button:e pressed:1]; }
- (void)rightMouseUp:(NSEvent *)e { [self button:e pressed:0]; }
- (void)otherMouseDown:(NSEvent *)e { [self button:e pressed:1]; }
- (void)otherMouseUp:(NSEvent *)e { [self button:e pressed:0]; }
- (void)scrollWheel:(NSEvent *)e {
    double s = scaleFactor();
    // Cocoa's deltas are positive when the content moves down; ours are positive when it moves up.
    double k = e.hasPreciseScrollingDeltas ? s : 12.0 * s;
    QaEv ev = {.kind = EV_SCROLL, .a = -e.scrollingDeltaX * k, .b = -e.scrollingDeltaY * k};
    push(ev);
}

- (void)key:(NSEvent *)e pressed:(int)down {
    unsigned mods = modsOf(e.modifierFlags);
    unsigned sym = symForKeyCode(e.keyCode);
    NSString *chars = e.characters;
    QaEv ev = {.kind = EV_KEY, .pressed = down, .repeat = e.isARepeat};
    if (sym) {
        if (sym == 0xff09 && (mods & MOD_SHIFT)) sym = 0xfe20;
        // Option+arrow moves by word, which is what Ctrl+arrow does here. Command+arrow goes to the ends.
        NSEventModifierFlags f = e.modifierFlags;
        if (sym >= 0xff51 && sym <= 0xff54) {
            if (f & NSEventModifierFlagOption) mods = (mods & ~MOD_ALT) | MOD_CTRL;
            else if ((f & NSEventModifierFlagCommand) && (sym == 0xff51 || sym == 0xff53)) {
                sym = sym == 0xff51 ? 0xff50 : 0xff57;
                mods &= ~MOD_CTRL;
            }
        }
        if (sym == 0x20 && !(mods & MOD_CTRL)) strlcpy(ev.text, " ", sizeof ev.text);
    } else {
        NSString *base = (mods & MOD_CTRL) || (mods & MOD_ALT) ? e.charactersIgnoringModifiers : chars;
        if (base.length == 0) return;
        unichar c0 = [base characterAtIndex:0];
        // Function keys come as private-use characters.
        if (c0 >= 0xF700 && c0 <= 0xF8FF) return;
        if (mods & MOD_CTRL) {
            unsigned c = c0;
            if (c >= 0x41 && c <= 0x5a) c += 0x20;
            sym = c;
        } else {
            sym = [base characterAtIndex:0];
            if (sym >= 0xD800 && sym < 0xDC00 && base.length > 1) {
                sym = 0x10000 + ((sym - 0xD800) << 10) + ([base characterAtIndex:1] - 0xDC00);
            }
            if (sym >= 0x100) sym |= 0x1000000;
            if (c0 >= 0x20 && c0 != 0x7f) strlcpy(ev.text, base.UTF8String, sizeof ev.text);
        }
    }
    ev.sym = sym;
    ev.mods = mods;
    push(ev);
}
- (void)keyDown:(NSEvent *)e { [self key:e pressed:1]; }
- (void)keyUp:(NSEvent *)e { [self key:e pressed:0]; }
- (void)flagsChanged:(NSEvent *)e {
    QaEv ev = {.kind = EV_MODS, .mods = modsOf(e.modifierFlags)};
    push(ev);
}
- (BOOL)performKeyEquivalent:(NSEvent *)e {
    // Menu shortcuts are handled by the menu items; everything else goes to keyDown.
    return [super performKeyEquivalent:e];
}

- (BOOL)windowShouldClose:(id)sender {
    QaEv ev = {.kind = EV_CLOSE};
    push(ev);
    return NO;
}
- (void)windowDidBecomeKey:(NSNotification *)n {
    QaEv ev = {.kind = EV_FOCUS_IN};
    push(ev);
}
- (void)windowDidResignKey:(NSNotification *)n {
    QaEv ev = {.kind = EV_FOCUS_OUT};
    push(ev);
}
- (void)qaQuit:(id)sender {
    QaEv ev = {.kind = EV_CLOSE};
    push(ev);
}
@end

static void buildMenu(NSString *name, QaView *target) {
    NSMenu *bar = [NSMenu new];
    NSMenuItem *appItem = [NSMenuItem new];
    [bar addItem:appItem];
    NSMenu *m = [[NSMenu alloc] initWithTitle:name];
    [m addItemWithTitle:[@"Hide " stringByAppendingString:name] action:@selector(hide:) keyEquivalent:@"h"];
    [m addItem:[NSMenuItem separatorItem]];
    NSMenuItem *q = [m addItemWithTitle:[@"Quit " stringByAppendingString:name] action:@selector(qaQuit:) keyEquivalent:@"q"];
    q.target = target;
    appItem.submenu = m;
    NSMenuItem *winItem = [NSMenuItem new];
    [bar addItem:winItem];
    NSMenu *w = [[NSMenu alloc] initWithTitle:@"Window"];
    [w addItemWithTitle:@"Minimize" action:@selector(performMiniaturize:) keyEquivalent:@"m"];
    NSMenuItem *c = [w addItemWithTitle:@"Close" action:@selector(qaQuit:) keyEquivalent:@"w"];
    c.target = target;
    winItem.submenu = w;
    [NSApp setMainMenu:bar];
    [NSApp setWindowsMenu:w];
}

// Creates the application and its one window. `w` and `h` are in points.
int qa_open(const char *title, const char *autosave, int w, int h) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
        QaView *v = [[QaView alloc] initWithFrame:NSMakeRect(0, 0, w, h)];
        v.wantsLayer = YES;
        v.layer.contentsGravity = kCAGravityTopLeft;
        NSWindowStyleMask style = NSWindowStyleMaskTitled | NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable | NSWindowStyleMaskResizable;
        window = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, w, h) styleMask:style backing:NSBackingStoreBuffered defer:NO];
        window.releasedWhenClosed = NO;
        window.contentView = v;
        window.delegate = v;
        window.acceptsMouseMovedEvents = YES;
        window.title = [NSString stringWithUTF8String:title];
        view = v;
        v.layer.contentsScale = window.backingScaleFactor;
        if (![window setFrameAutosaveName:[NSString stringWithUTF8String:autosave]]) [window center];
        buildMenu([NSString stringWithUTF8String:title], v);
        [window makeFirstResponder:v];
        [NSApp finishLaunching];
        [window makeKeyAndOrderFront:nil];
        [NSApp activateIgnoringOtherApps:YES];
        [v pushSize];
        return 0;
    }
}

int qa_poll(QaEv *out, int max) {
    int n = qlen < max ? qlen : max;
    memcpy(out, queue, n * sizeof(QaEv));
    memmove(queue, queue + n, (qlen - n) * sizeof(QaEv));
    qlen -= n;
    return n;
}

void qa_size(int *w, int *h, double *scale) {
    double s = scaleFactor();
    NSSize z = view.bounds.size;
    *w = (int)(z.width * s + 0.5);
    *h = (int)(z.height * s + 0.5);
    *scale = s;
}

static void pump(NSDate *until) {
    NSEvent *e = [NSApp nextEventMatchingMask:NSEventMaskAny untilDate:until inMode:NSDefaultRunLoopMode dequeue:YES];
    while (e) {
        [NSApp sendEvent:e];
        e = [NSApp nextEventMatchingMask:NSEventMaskAny untilDate:nil inMode:NSDefaultRunLoopMode dequeue:YES];
    }
}

static void wakeCb(CFFileDescriptorRef f, CFOptionFlags flags, void *info) {
    NSEvent *e = [NSEvent otherEventWithType:NSEventTypeApplicationDefined location:NSZeroPoint modifierFlags:0 timestamp:0 windowNumber:0 context:nil subtype:1 data1:0 data2:0];
    [NSApp postEvent:e atStart:YES];
}

// Waits for a window event, for one of `fds` to become readable, or for `ms` (-1: forever).
void qa_wait(const int *fds, int n, int ms) {
    @autoreleasepool {
        if (qlen > 0) return;
        struct pollfd pf[16];
        if (n > 16) n = 16;
        for (int i = 0; i < n; i++) pf[i] = (struct pollfd){.fd = fds[i], .events = POLLIN};
        if (n > 0 && poll(pf, n, 0) > 0) {
            pump(nil);
            return;
        }
        CFFileDescriptorRef refs[16];
        CFRunLoopSourceRef srcs[16];
        for (int i = 0; i < n; i++) {
            refs[i] = CFFileDescriptorCreate(NULL, fds[i], false, wakeCb, NULL);
            srcs[i] = CFFileDescriptorCreateRunLoopSource(NULL, refs[i], 0);
            CFRunLoopAddSource(CFRunLoopGetMain(), srcs[i], (CFStringRef)NSDefaultRunLoopMode);
            CFFileDescriptorEnableCallBacks(refs[i], kCFFileDescriptorReadCallBack);
        }
        pump(ms < 0 ? [NSDate distantFuture] : [NSDate dateWithTimeIntervalSinceNow:ms / 1000.0]);
        for (int i = 0; i < n; i++) {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), srcs[i], (CFStringRef)NSDefaultRunLoopMode);
            CFFileDescriptorInvalidate(refs[i]);
            CFRelease(srcs[i]);
            CFRelease(refs[i]);
        }
    }
}

static void freeBytes(void *info, const void *data, size_t size) { free((void *)data); }

// Shows `w`×`h` premultiplied ARGB pixels (`stride` pixels per row).
void qa_present(const uint32_t *px, int w, int h, size_t stride) {
    @autoreleasepool {
        size_t row = (size_t)w * 4;
        uint8_t *copy = malloc(row * h);
        for (int y = 0; y < h; y++) memcpy(copy + y * row, px + y * stride, row);
        CGDataProviderRef prov = CGDataProviderCreateWithData(NULL, copy, row * h, freeBytes);
        CGColorSpaceRef cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGImageRef img = CGImageCreate(w, h, 8, 32, row, cs, kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little, prov, NULL, false, kCGRenderingIntentDefault);
        [CATransaction begin];
        [CATransaction setDisableActions:YES];
        view.layer.contentsScale = scaleFactor();
        view.layer.contents = (__bridge id)img;
        [CATransaction commit];
        CGImageRelease(img);
        CGColorSpaceRelease(cs);
        CGDataProviderRelease(prov);
    }
}

// Closes the window, so another can be opened after it in the same process.
void qa_close(void) {
    @autoreleasepool {
        window.delegate = nil;
        [window orderOut:nil];
        [window close];
        window = nil;
        view = nil;
        qlen = 0;
        cursorKind = 0;
    }
}

void qa_set_title(const char *t) { window.title = [NSString stringWithUTF8String:t] ?: @""; }

void qa_set_cursor(int kind) {
    if (kind == cursorKind) return;
    cursorKind = kind;
    [window invalidateCursorRectsForView:view];
}

void qa_set_min_size(int w, int h) { window.contentMinSize = NSMakeSize(w, h); }

void qa_clip_set(const char *t) {
    NSPasteboard *pb = NSPasteboard.generalPasteboard;
    [pb clearContents];
    [pb setString:[NSString stringWithUTF8String:t] ?: @"" forType:NSPasteboardTypeString];
}

// The clipboard text as a malloc'd string, or NULL.
char *qa_clip_get(void) {
    NSString *s = [NSPasteboard.generalPasteboard stringForType:NSPasteboardTypeString];
    return s ? strdup(s.UTF8String) : NULL;
}

// Arguments for a URL handler: waits briefly for the `qa:` link the system sends this process.
static NSString *gotURL;
@interface QaURL : NSObject
@end
@implementation QaURL
- (void)get:(NSAppleEventDescriptor *)e reply:(NSAppleEventDescriptor *)r {
    gotURL = [[e paramDescriptorForKeyword:keyDirectObject] stringValue];
}
@end

// Runs a minimal application that waits up to `ms` for a URL, for the `qa:` handler. Returns
// a malloc'd string, or NULL if none came.
char *qa_wait_url(int ms) {
    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        QaURL *h = [QaURL new];
        [[NSAppleEventManager sharedAppleEventManager] setEventHandler:h andSelector:@selector(get:reply:) forEventClass:kInternetEventClass andEventID:kAEGetURL];
        [NSApp finishLaunching];
        NSDate *end = [NSDate dateWithTimeIntervalSinceNow:ms / 1000.0];
        while (!gotURL && [end timeIntervalSinceNow] > 0) {
            NSEvent *e = [NSApp nextEventMatchingMask:NSEventMaskAny untilDate:[NSDate dateWithTimeIntervalSinceNow:0.1] inMode:NSDefaultRunLoopMode dequeue:YES];
            if (e) [NSApp sendEvent:e];
        }
        return gotURL ? strdup(gotURL.UTF8String) : NULL;
    }
}
