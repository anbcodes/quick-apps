// Package qa is the Quick Apps SDK for TinyGo.
//
// An app registers two functions from an init function and has an empty
// main:
//
//	func init() { qa.App(start, event) }
//	func main() {}
//
// Calls build up one patch, which is sent when start/event returns. Strings
// and byte slices in an Event are copies, so they can be kept. Build with
// build.sh. The wire format is in docs/app-abi.md.
package qa

import (
	"math"
	"unsafe"
)

// Node is a node id.
type Node uint32

// Node kinds (docs/ui-tree.md).
const (
	KindGrid     = 1
	KindText     = 2
	KindImage    = 3
	KindCanvas   = 4
	KindScroll   = 5
	KindButton   = 10
	KindInput    = 11
	KindCheckbox = 12
	KindSelect   = 13
	KindTabs     = 14
	KindProgress = 15
	KindDivider  = 16
)

// Prop ids (docs/ui-tree.md and docs/components.md).
const (
	PropRow      = 1
	PropCol      = 2
	PropRowSpan  = 3
	PropColSpan  = 4
	PropPriority = 5
	PropMinW     = 6
	PropMinH     = 7
	PropMaxW     = 8
	PropMaxH     = 9
	PropPad      = 10
	PropAlignX   = 11
	PropAlignY   = 12
	PropHidden   = 13
	PropVariant  = 14
	PropTooltip  = 15
	PropTag      = 16
	PropDisabled = 17

	GridCols      = 20
	GridRows      = 21
	GridGap       = 22
	GridClickable = 23
	GridSelected  = 24
	GridFlow      = 25

	TextText       = 20
	TextWrap       = 21
	TextStyle      = 22
	TextAlign      = 23
	TextSelectable = 24
	TextMaxLines   = 25
	TextSpans      = 26

	ImageSrc  = 20
	ImageData = 21
	ImageFit  = 22

	CanvasPxW       = 20
	CanvasPxH       = 21
	CanvasFit       = 22
	CanvasFocusable = 23

	ScrollAxis     = 20
	ScrollStickEnd = 21

	ButtonLabel = 20

	InputValue       = 20
	InputPlaceholder = 21
	InputMultiline   = 22
	InputSecret      = 23
	InputLines       = 25

	CheckboxLabel   = 20
	CheckboxChecked = 21
	CheckboxWrap    = 23

	SelectOptions     = 20
	SelectSelected    = 21
	SelectPlaceholder = 22

	TabsLabels   = 20
	TabsSelected = 21

	ProgressValue = 20
)

// Alignment within a grid cell.
const (
	Stretch = 0
	Start   = 1
	Center  = 2
	End     = 3
)

// Event types (docs/events.md).
const (
	EvResize      = 1
	EvClick       = 2
	EvChange      = 3
	EvSubmit      = 4
	EvToggle      = 5
	EvSelect      = 6
	EvKey         = 7
	EvPointer     = 8
	EvTimer       = 9
	EvHTTP        = 10
	EvWS          = 11
	EvMessage     = 12
	EvRequestDone = 13
	EvClose       = 14
	EvActivate    = 15
	EvFocusOut    = 16
	EvNearEnd     = 17
	EvLink        = 18
	EvDoubleClick = 19
)

// Keysyms and modifier bits for EvKey.
const (
	KeyBackspace = 0xff08
	KeyTab       = 0xff09
	KeyEnter     = 0xff0d
	KeyEscape    = 0xff1b
	KeyDelete    = 0xffff
	KeyLeft      = 0xff51
	KeyUp        = 0xff52
	KeyRight     = 0xff53
	KeyDown      = 0xff54

	ModShift = 1
	ModCtrl  = 2
	ModAlt   = 4
	ModSuper = 8
)

// Negative results from host calls.
const (
	ErrDenied    = -1
	ErrNotFound  = -2
	ErrExists    = -3
	ErrInvalid   = -4
	ErrIO        = -5
	ErrLimit     = -6
	ErrBadHandle = -7
	ErrTooSmall  = -8
)

// ---- the patch ----

var (
	patch  []byte
	nextID Node
)

func u8(v uint8)   { patch = append(patch, v) }
func u16(v uint16) { patch = append(patch, byte(v), byte(v>>8)) }
func u32(v uint32) { patch = append(patch, byte(v), byte(v>>8), byte(v>>16), byte(v>>24)) }
func f32(v float32) {
	u32(math.Float32bits(v))
}
func str(s string) {
	u32(uint32(len(s)))
	patch = append(patch, s...)
}
func op1(op uint8, a Node) {
	u8(op)
	u32(uint32(a))
}
func op2(op uint8, a, b Node) {
	op1(op, a)
	u32(uint32(b))
}
func setHead(n Node, prop uint16, tag uint8) {
	op1(6, n)
	u16(prop)
	u8(tag)
}

// Flush sends the patch now. Done for you after start and event.
func Flush() {
	if len(patch) > 0 {
		sysUIPatch(unsafe.Pointer(&patch[0]), uint32(len(patch)))
	}
	patch = patch[:0]
}

// Track is one grid column or row.
type Track struct {
	Kind            uint8
	Value, Min, Max float32
}

var inf = float32(math.Inf(1))

func Px(v float32) Track { return Track{0, v, inf, inf} }
func Fr(v float32) Track { return Track{1, v, inf, inf} }
func Auto() Track        { return Track{2, 0, inf, inf} }

// ---- changing nodes ----

func SetInt(n Node, prop uint16, v int32) {
	setHead(n, prop, 1)
	u32(uint32(v))
}
func SetBool(n Node, prop uint16, v bool) {
	if v {
		SetInt(n, prop, 1)
	} else {
		SetInt(n, prop, 0)
	}
}
func SetNum(n Node, prop uint16, v float32) {
	setHead(n, prop, 2)
	f32(v)
}
func SetStr(n Node, prop uint16, s string) {
	setHead(n, prop, 3)
	str(s)
}
func SetBytes(n Node, prop uint16, b []byte) {
	setHead(n, prop, 5)
	u32(uint32(len(b)))
	patch = append(patch, b...)
}
func SetTracks(n Node, prop uint16, tracks ...Track) {
	setHead(n, prop, 6)
	u8(uint8(len(tracks)))
	for _, t := range tracks {
		u8(t.Kind)
		f32(t.Value)
		f32(t.Min)
		f32(t.Max)
	}
}

// Reset puts a prop back to its default.
func Reset(n Node, prop uint16) { setHead(n, prop, 0) }

func SetText(n Node, s string)    { SetStr(n, TextText, s) }
func SetLabel(n Node, s string)   { SetStr(n, ButtonLabel, s) }
func SetValue(n Node, s string)   { SetStr(n, InputValue, s) }
func SetChecked(n Node, on bool)  { SetBool(n, CheckboxChecked, on) }
func Hide(n Node, on bool)        { SetBool(n, PropHidden, on) }
func SetDisabled(n Node, on bool) { SetBool(n, PropDisabled, on) }
func SetVariant(n Node, v string) { SetStr(n, PropVariant, v) }

// Theme tokens for a subtree (docs/theming.md). A color is 0xRRGGBBAA.
func ctxHead(n Node, key string, tag uint8) {
	op1(7, n)
	str(key)
	u8(tag)
}
func CtxColor(n Node, key string, rgba uint32) {
	ctxHead(n, key, 4)
	u32(rgba)
}
func CtxNum(n Node, key string, v float32) {
	ctxHead(n, key, 2)
	f32(v)
}
func CtxStr(n Node, key string, v string) {
	ctxHead(n, key, 3)
	str(v)
}

func Append(parent, child Node) { op2(3, parent, child) }
func Insert(parent, child, before Node) {
	op2(4, parent, child)
	u32(uint32(before))
}
func Detach(n Node)  { op1(5, n) }
func Destroy(n Node) { op1(2, n) }

// Replace puts with where old is and destroys old.
func Replace(old, with Node) { op2(11, old, with) }
func SetRoot(n Node)         { op1(8, n) }
func Focus(n Node)           { op1(9, n) }
func ScrollTo(n Node)        { op1(10, n) }

// ---- building nodes ----

// B is a node being built. Finish with AddTo, Root or Detached.
type B struct{ N Node }

func (b B) Int(prop uint16, v int32) B   { SetInt(b.N, prop, v); return b }
func (b B) Bool(prop uint16, v bool) B   { SetBool(b.N, prop, v); return b }
func (b B) Num(prop uint16, v float32) B { SetNum(b.N, prop, v); return b }
func (b B) Str(prop uint16, s string) B  { SetStr(b.N, prop, s); return b }
func (b B) Cols(t ...Track) B            { SetTracks(b.N, GridCols, t...); return b }
func (b B) Rows(t ...Track) B            { SetTracks(b.N, GridRows, t...); return b }
func (b B) Gap(v float32) B              { return b.Num(GridGap, v) }
func (b B) Pad(v float32) B              { return b.Pad4(v, v, v, v) }
func (b B) Pad4(top, right, bottom, left float32) B {
	setHead(b.N, PropPad, 7)
	f32(top)
	f32(right)
	f32(bottom)
	f32(left)
	return b
}
func (b B) At(row, col int32) B     { return b.Int(PropRow, row).Int(PropCol, col) }
func (b B) Span(rows, cols int32) B { return b.Int(PropRowSpan, rows).Int(PropColSpan, cols) }
func (b B) Priority(p int32) B      { return b.Int(PropPriority, p) }
func (b B) MinW(v float32) B        { return b.Num(PropMinW, v) }
func (b B) MinH(v float32) B        { return b.Num(PropMinH, v) }
func (b B) MaxW(v float32) B        { return b.Num(PropMaxW, v) }
func (b B) MaxH(v float32) B        { return b.Num(PropMaxH, v) }
func (b B) AlignX(a int32) B        { return b.Int(PropAlignX, a) }
func (b B) AlignY(a int32) B        { return b.Int(PropAlignY, a) }
func (b B) Hidden(on bool) B        { return b.Bool(PropHidden, on) }
func (b B) Disabled(on bool) B      { return b.Bool(PropDisabled, on) }
func (b B) Variant(v string) B      { return b.Str(PropVariant, v) }

// Tag sets an int32 sent back with this node's events (Event.Tag).
func (b B) Tag(t int32) B { return b.Int(PropTag, t) }

// Style is a text style: "body", "title", "heading", "caption" or "mono".
func (b B) Style(s string) B       { return b.Str(TextStyle, s) }
func (b B) Clickable(on bool) B    { return b.Bool(GridClickable, on) }
func (b B) Placeholder(s string) B { return b.Str(InputPlaceholder, s) }
func (b B) Value(s string) B       { return b.Str(InputValue, s) }
func (b B) Checked(on bool) B      { return b.Bool(CheckboxChecked, on) }
func (b B) AddTo(parent Node) Node { Append(parent, b.N); return b.N }
func (b B) Root() Node             { SetRoot(b.N); return b.N }
func (b B) Detached() Node         { return b.N }

func Create(kind uint8) B {
	nextID++
	op1(1, nextID)
	u8(kind)
	return B{nextID}
}
func Grid() B                 { return Create(KindGrid) }
func Text(s string) B         { return Create(KindText).Str(TextText, s) }
func Button(label string) B   { return Create(KindButton).Str(ButtonLabel, label) }
func Input() B                { return Create(KindInput) }
func Checkbox(label string) B { return Create(KindCheckbox).Str(CheckboxLabel, label) }
func Scroll() B               { return Create(KindScroll) }
func Image(src string) B      { return Create(KindImage).Str(ImageSrc, src) }
func Progress(v float32) B    { return Create(KindProgress).Num(ProgressValue, v) }
func Divider() B              { return Create(KindDivider) }
func Canvas(w, h int32) B     { return Create(KindCanvas).Int(CanvasPxW, w).Int(CanvasPxH, h) }

// Select takes its options one per line.
func Select(options string) B { return Create(KindSelect).Str(SelectOptions, options) }

// Tabs takes its labels one per line.
func Tabs(labels string) B { return Create(KindTabs).Str(TabsLabels, labels) }

// ---- events ----

// Event is one event. Which fields are set depends on Type:
//
//	EvResize       W, H, Scale
//	EvClick, EvDoubleClick, EvFocusOut, EvNearEnd   Node
//	EvChange, EvSubmit   Node, Text
//	EvToggle       Node, Value (0/1)
//	EvSelect       Node, Value (index)
//	EvLink         Node, Value (span index)
//	EvKey          Node (focused), Sym, Mods, Text
//	EvPointer      Node, Kind (0 down 1 up 2 move 3 wheel 4 leave), X, Y, Buttons, DX, DY
//	EvTimer        ID
//	EvHTTP         ID (request), Status (0 = failed, Data is the error), Text (headers), Data (body)
//	EvWS           ID (socket), Kind (0 open 1 text 2 binary 3 closed 4 error), Data
//	EvMessage      Text (sender's manifest URL), Data
//	EvRequestDone  ID (request), Value (result), Text (info)
//
// Tag is the node's tag prop for every event that has a node.
type Event struct {
	Type                      uint8
	Node                      Node
	Tag                       int32
	Value                     int32
	ID                        uint32
	Text                      string
	Data                      []byte
	Sym                       uint32
	Mods, Kind, Buttons       uint8
	Status                    uint16
	X, Y, DX, DY, W, H, Scale float32
}

type reader struct{ b []byte }

func (r *reader) u8() uint8 {
	if len(r.b) < 1 {
		return 0
	}
	v := r.b[0]
	r.b = r.b[1:]
	return v
}
func (r *reader) u32() uint32 {
	if len(r.b) < 4 {
		r.b = nil
		return 0
	}
	v := uint32(r.b[0]) | uint32(r.b[1])<<8 | uint32(r.b[2])<<16 | uint32(r.b[3])<<24
	r.b = r.b[4:]
	return v
}
func (r *reader) f32() float32 { return math.Float32frombits(r.u32()) }
func (r *reader) bytes() []byte {
	n := int(r.u32())
	if n > len(r.b) {
		n = len(r.b)
	}
	out := append([]byte(nil), r.b[:n]...)
	r.b = r.b[n:]
	return out
}
func (r *reader) str() string { return string(r.bytes()) }

// Decode reads one event from the runtime's encoding.
func Decode(buf []byte) Event {
	r := reader{buf}
	e := Event{Type: r.u8()}
	hasNode := true
	switch e.Type {
	case EvResize:
		e.W, e.H, e.Scale = r.f32(), r.f32(), r.f32()
		hasNode = false
	case EvClick, EvDoubleClick, EvFocusOut, EvNearEnd:
		e.Node = Node(r.u32())
	case EvChange, EvSubmit:
		e.Node = Node(r.u32())
		e.Text = r.str()
	case EvToggle:
		e.Node = Node(r.u32())
		e.Value = int32(r.u8())
	case EvSelect, EvLink:
		e.Node = Node(r.u32())
		e.Value = int32(r.u32())
	case EvKey:
		e.Node = Node(r.u32())
		e.Sym = r.u32()
		e.Mods = r.u8()
		e.Text = r.str()
	case EvPointer:
		e.Node = Node(r.u32())
		e.Kind = r.u8()
		e.X, e.Y = r.f32(), r.f32()
		e.Buttons = r.u8()
		e.DX, e.DY = r.f32(), r.f32()
	case EvTimer:
		e.ID = r.u32()
		hasNode = false
	case EvHTTP:
		e.ID = r.u32()
		e.Status = uint16(r.u8()) | uint16(r.u8())<<8
		e.Text = r.str()
		e.Data = r.bytes()
		hasNode = false
	case EvWS:
		e.ID = r.u32()
		e.Kind = r.u8()
		e.Data = r.bytes()
		hasNode = false
	case EvMessage:
		e.Text = r.str()
		e.Data = r.bytes()
		hasNode = false
	case EvRequestDone:
		e.ID = r.u32()
		e.Value = int32(r.u32())
		e.Text = r.str()
		hasNode = false
	default:
		hasNode = false
	}
	if hasNode {
		e.Tag = int32(r.u32())
	}
	return e
}

// ---- the app ----

var (
	onStart func()
	onEvent func(e *Event)
	evBuf   []byte
)

// App registers the app. Call it from an init function.
func App(start func(), event func(e *Event)) {
	onStart = start
	onEvent = event
}

// The runtime copies each event into one reused buffer.
//
//export qa_alloc
func qaAlloc(n uint32) unsafe.Pointer {
	if n == 0 {
		n = 1
	}
	if uint32(cap(evBuf)) < n {
		evBuf = make([]byte, n)
	}
	evBuf = evBuf[:n]
	return unsafe.Pointer(&evBuf[0])
}

//export qa_start
func qaStart() {
	if onStart != nil {
		onStart()
	}
	Flush()
}

//export qa_event
func qaEvent(ptr unsafe.Pointer, n uint32) {
	e := Decode(unsafe.Slice((*byte)(ptr), n))
	if onEvent != nil {
		onEvent(&e)
	}
	Flush()
}

// ---- helpers ----

func ptr(s string) unsafe.Pointer { return unsafe.Pointer(unsafe.StringData(s)) }

// Itoa formats an integer without pulling in strconv.
func Itoa(v int) string {
	if v == 0 {
		return "0"
	}
	var b [20]byte
	i := len(b)
	neg := v < 0
	u := uint(v)
	if neg {
		u = uint(-v)
	}
	for u > 0 {
		i--
		b[i] = byte('0' + u%10)
		u /= 10
	}
	if neg {
		i--
		b[i] = '-'
	}
	return string(b[i:])
}

func Log(s string)      { sysLog(1, ptr(s), uint32(len(s))) }
func SetTitle(s string) { sysSetTitle(ptr(s), uint32(len(s))) }
func Quit()             { sysQuit() }

// Timer fires one EvTimer after ms.
func Timer(ms uint32) uint32 { return sysTimer(ms, 0) }

// Interval fires an EvTimer every ms until TimerCancel.
func Interval(ms uint32) uint32 { return sysTimer(ms, 1) }
func TimerCancel(id uint32)     { sysTimerCancel(id) }
func NowMs() int64              { return sysNowMs() }

// Folders and open flags for files (docs/io.md).
const (
	AppDir    = 0
	BundleDir = 1

	FsRead   = 1
	FsWrite  = 2
	FsCreate = 4
	FsTrunc  = 8
	FsAppend = 16
	FsExcl   = 32
)

// ReadFile reads a whole file. ok is false if it can't be read.
func ReadFile(dir int32, path string) (data []byte, ok bool) {
	fd := sysFsOpen(dir, ptr(path), uint32(len(path)), FsRead)
	if fd < 0 {
		return nil, false
	}
	defer sysFsClose(fd)
	var chunk [4096]byte
	for {
		n := sysFsRead(fd, unsafe.Pointer(&chunk[0]), uint32(len(chunk)))
		if n < 0 {
			return nil, false
		}
		if n == 0 {
			return data, true
		}
		data = append(data, chunk[:n]...)
	}
}

// WriteFile replaces a whole file. It returns false if it can't be written.
func WriteFile(dir int32, path string, data []byte) bool {
	fd := sysFsOpen(dir, ptr(path), uint32(len(path)), FsWrite|FsCreate|FsTrunc)
	if fd < 0 {
		return false
	}
	defer sysFsClose(fd)
	for len(data) > 0 {
		n := sysFsWrite(fd, unsafe.Pointer(&data[0]), uint32(len(data)))
		if n <= 0 {
			return false
		}
		data = data[n:]
	}
	return true
}

// HTTP starts a request. The answer is an EvHTTP whose ID is the returned
// value. headers is "Name: value\n" lines.
func HTTP(method, url, headers string, body []byte) uint32 {
	var b []byte
	put := func(s []byte) {
		n := uint32(len(s))
		b = append(b, byte(n), byte(n>>8), byte(n>>16), byte(n>>24))
		b = append(b, s...)
	}
	put([]byte(method))
	put([]byte(url))
	put([]byte(headers))
	put(body)
	b = append(b, 0, 0, 0, 0, 0, 0, 0, 0) // timeout 30 s, largest body 8 MB
	return sysHTTP(unsafe.Pointer(&b[0]), uint32(len(b)))
}

// Get starts a GET request.
func Get(url string) uint32 { return HTTP("GET", url, "", nil) }

// ---- host calls, as in docs/app-abi.md ----

//go:wasmimport qa ui_patch
func sysUIPatch(p unsafe.Pointer, n uint32) int32

//go:wasmimport qa set_title
func sysSetTitle(p unsafe.Pointer, n uint32)

//go:wasmimport qa quit
func sysQuit()

//go:wasmimport qa now_ms
func sysNowMs() int64

//go:wasmimport qa log
func sysLog(level uint32, p unsafe.Pointer, n uint32)

//go:wasmimport qa timer
func sysTimer(ms uint32, repeat uint32) uint32

//go:wasmimport qa timer_cancel
func sysTimerCancel(id uint32)

//go:wasmimport qa fs_open
func sysFsOpen(dir int32, path unsafe.Pointer, n uint32, flags uint32) int32

//go:wasmimport qa fs_read
func sysFsRead(fd int32, p unsafe.Pointer, n uint32) int32

//go:wasmimport qa fs_write
func sysFsWrite(fd int32, p unsafe.Pointer, n uint32) int32

//go:wasmimport qa fs_close
func sysFsClose(fd int32) int32

//go:wasmimport qa http
func sysHTTP(p unsafe.Pointer, n uint32) uint32

// The rest of the host calls, for what the helpers above don't cover.
// Pointers are to the first byte; lengths are in bytes.

//go:wasmimport qa canvas_put
func SysCanvasPut(node, x, y, w, h uint32, rgba unsafe.Pointer) int32

//go:wasmimport qa env
func SysEnv(out unsafe.Pointer, cap uint32) int32

//go:wasmimport qa mono_ms
func SysMonoMs() int64

//go:wasmimport qa random
func SysRandom(p unsafe.Pointer, n uint32)

//go:wasmimport qa json_parse
func SysJSONParse(p unsafe.Pointer, n uint32, out unsafe.Pointer, cap uint32, needed unsafe.Pointer) int32

//go:wasmimport qa fs_seek
func SysFsSeek(fd int32, off int64, whence uint32) int64

//go:wasmimport qa fs_stat
func SysFsStat(dir int32, path unsafe.Pointer, n uint32, out unsafe.Pointer) int32

//go:wasmimport qa fs_list
func SysFsList(dir int32, path unsafe.Pointer, n uint32, out unsafe.Pointer, cap uint32, needed unsafe.Pointer) int32

//go:wasmimport qa fs_mkdir
func SysFsMkdir(dir int32, path unsafe.Pointer, n uint32) int32

//go:wasmimport qa fs_remove
func SysFsRemove(dir int32, path unsafe.Pointer, n uint32) int32

//go:wasmimport qa fs_rename
func SysFsRename(dir int32, from unsafe.Pointer, fromLen uint32, dir2 int32, to unsafe.Pointer, toLen uint32) int32

//go:wasmimport qa request
func SysRequest(kind uint32, arg unsafe.Pointer, n uint32) uint32

//go:wasmimport qa http_cancel
func SysHTTPCancel(req uint32)

//go:wasmimport qa ws_open
func SysWsOpen(p unsafe.Pointer, n uint32) uint32

//go:wasmimport qa ws_send
func SysWsSend(ws uint32, kind uint32, p unsafe.Pointer, n uint32) int32

//go:wasmimport qa ws_close
func SysWsClose(ws uint32, code uint32)

//go:wasmimport qa msg_send
func SysMsgSend(to unsafe.Pointer, toLen uint32, data unsafe.Pointer, n uint32) int32

//go:wasmimport qa clipboard_write
func SysClipboardWrite(p unsafe.Pointer, n uint32) int32

//go:wasmimport qa open_url
func SysOpenURL(p unsafe.Pointer, n uint32) int32

//go:wasmimport qa secret_get
func SysSecretGet(key unsafe.Pointer, keyLen uint32, out unsafe.Pointer, cap uint32, needed unsafe.Pointer) int32

//go:wasmimport qa secret_set
func SysSecretSet(key unsafe.Pointer, keyLen uint32, val unsafe.Pointer, valLen uint32) int32
