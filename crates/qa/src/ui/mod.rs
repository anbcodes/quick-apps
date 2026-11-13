//! The retained UI tree: patches in, events out, layout, paint and input.

pub mod edit;
pub mod interact;
pub mod layout;
pub mod paint;
pub mod theme;

use crate::gfx::{Color, RectF};
use crate::platform::Cursor;
use crate::text::{self, FontId, Fonts};
use qa_proto::{kind, op, prop, Reader, Track, Value};
use std::collections::HashMap;

/// An owned prop or token value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    None,
    I(i32),
    F(f32),
    S(String),
    C(Color),
    B(Vec<u8>),
    T(Vec<Track>),
    E([f32; 4]),
}

impl Val {
    fn from_wire(v: &Value) -> Val {
        match *v {
            Value::None => Val::None,
            Value::I32(x) => Val::I(x),
            Value::F32(x) => Val::F(x),
            Value::Str(s) => Val::S(s.to_string()),
            Value::Color(c) => Val::C(Color::hex(c)),
            Value::Bytes(b) => Val::B(b.to_vec()),
            Value::Tracks(raw) => Val::T(qa_proto::tracks(raw).collect()),
            Value::Edges(e) => Val::E(e),
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        match *self {
            Val::F(f) => Some(f),
            Val::I(i) => Some(i as f32),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::S(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<Color> {
        match *self {
            Val::C(c) => Some(c),
            _ => None,
        }
    }
}

/// Measurements cached until the node or a descendant changes.
#[derive(Default, Clone)]
pub struct Measure {
    pub min_w: Option<f32>,
    pub pref_w: Option<f32>,
    pub h: Option<(f32, f32)>,
    /// Wrapped text lines for a content width.
    pub lines: Option<(f32, Vec<(usize, usize)>)>,
}

#[derive(Default)]
pub struct ScrollSt {
    pub x: f32,
    pub y: f32,
    pub content_w: f32,
    pub content_h: f32,
    pub near_end_sent: bool,
    pub at_end: bool,
}

pub enum State {
    None,
    Input(Box<edit::Edit>),
    Scroll(ScrollSt),
}

pub struct Node {
    pub kind: u8,
    pub parent: u32,
    pub children: Vec<u32>,
    pub props: Vec<(u16, Val)>,
    pub ctx: Vec<(String, Val)>,
    /// Logical px, window coordinates (scroll offsets applied).
    pub rect: RectF,
    /// Hidden by layout because its track was hidden.
    pub collapsed: bool,
    /// Children overflow and must be clipped.
    pub clips: bool,
    pub m: Measure,
    /// This node or a descendant needs layout.
    pub dirty: bool,
    pub st: State,
}

impl Node {
    fn new(kind: u8) -> Node {
        let st = match kind {
            kind::INPUT => State::Input(Box::default()),
            kind::SCROLL => State::Scroll(ScrollSt::default()),
            _ => State::None,
        };
        Node {
            kind,
            parent: 0,
            children: Vec::new(),
            props: Vec::new(),
            ctx: Vec::new(),
            rect: RectF::default(),
            collapsed: false,
            clips: false,
            m: Measure::default(),
            dirty: true,
            st,
        }
    }
    pub fn get(&self, p: u16) -> Option<&Val> {
        self.props.iter().find(|(k, _)| *k == p).map(|(_, v)| v)
    }
    pub fn i(&self, p: u16, def: i32) -> i32 {
        match self.get(p) {
            Some(Val::I(v)) => *v,
            Some(Val::F(v)) => *v as i32,
            _ => def,
        }
    }
    pub fn f(&self, p: u16, def: f32) -> f32 {
        self.get(p).and_then(|v| v.as_f32()).unwrap_or(def)
    }
    pub fn s(&self, p: u16) -> &str {
        self.get(p).and_then(|v| v.as_str()).unwrap_or("")
    }
    pub fn edges(&self, p: u16) -> [f32; 4] {
        match self.get(p) {
            Some(Val::E(e)) => *e,
            Some(Val::F(f)) => [*f; 4],
            Some(Val::I(i)) => [*i as f32; 4],
            _ => [0.0; 4],
        }
    }
    pub fn hidden(&self) -> bool {
        self.i(prop::HIDDEN, 0) != 0 || self.collapsed
    }
    /// Disabled by its own props. `Ui::is_disabled` also looks at ancestors.
    pub fn disabled(&self) -> bool {
        if self.i(prop::DISABLED, 0) != 0 {
            return true;
        }
        let p = match self.kind {
            kind::BUTTON => prop::button::DISABLED,
            kind::INPUT => prop::input::DISABLED,
            kind::CHECKBOX => prop::checkbox::DISABLED,
            kind::SELECT => prop::select::DISABLED,
            _ => return false,
        };
        self.i(p, 0) != 0
    }
    pub fn focusable(&self) -> bool {
        if self.disabled() {
            return false;
        }
        match self.kind {
            kind::BUTTON | kind::INPUT | kind::CHECKBOX | kind::SELECT | kind::TABS => true,
            kind::GRID => self.i(prop::grid::CLICKABLE, 0) != 0,
            kind::CANVAS => self.i(prop::canvas::FOCUSABLE, 0) != 0,
            _ => false,
        }
    }
    pub fn edit(&mut self) -> Option<&mut edit::Edit> {
        match &mut self.st {
            State::Input(e) => Some(e),
            _ => None,
        }
    }
    pub fn scroll(&mut self) -> Option<&mut ScrollSt> {
        match &mut self.st {
            State::Scroll(s) => Some(s),
            _ => None,
        }
    }
}

/// An event for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    Click(u32),
    Change(u32, String),
    Submit(u32, String),
    Toggle(u32, bool),
    Select(u32, i32),
    Key { node: u32, sym: u32, mods: u8, text: String },
    Pointer { node: u32, kind: u8, x: f32, y: f32, buttons: u8, dx: f32, dy: f32 },
    FocusOut(u32),
    NearEnd(u32),
    /// A link span was clicked: node, span index.
    Link(u32, u32),
    /// The second of two quick clicks on a button, checkbox or clickable grid.
    DoubleClick(u32),
}

impl UiEvent {
    /// The node the event came from.
    pub fn node(&self) -> u32 {
        match *self {
            UiEvent::Click(n) | UiEvent::Change(n, _) | UiEvent::Submit(n, _) | UiEvent::Toggle(n, _) | UiEvent::Select(n, _) => n,
            UiEvent::Key { node, .. } | UiEvent::Pointer { node, .. } => node,
            UiEvent::FocusOut(n) | UiEvent::NearEnd(n) | UiEvent::Link(n, _) | UiEvent::DoubleClick(n) => n,
        }
    }
}

#[derive(Debug)]
pub struct PatchError {
    pub offset: usize,
    pub msg: String,
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle {
    pub font: FontId,
    /// Logical px.
    pub size: f32,
    pub line_h: f32,
}

pub struct Ui {
    pub nodes: HashMap<u32, Node>,
    pub root: u32,
    pub theme: theme::Theme,
    pub fonts: Fonts,
    font_ids: HashMap<(String, bool), FontId>,
    pub scale: f32,
    /// Window size in logical px.
    pub win_w: f32,
    pub win_h: f32,
    pub focus: u32,
    pub hover: u32,
    pub press: u32,
    /// Show the focus ring (focus came from the keyboard).
    pub focus_visible: bool,
    pub layout_dirty: bool,
    /// Logical rects that need repainting.
    pub damage: Vec<RectF>,
    pub out: Vec<UiEvent>,
    pub cursor: Cursor,
    /// Text the runtime should put on the clipboard.
    pub clipboard_out: Option<String>,
    /// An input waiting for clipboard contents.
    pub paste_into: u32,
    pub quit: bool,
    /// The title bar's close button was clicked: ask the app to close.
    pub quit_requested: bool,
    pub mods: u8,
    pub ptr: (f32, f32),
    pub(crate) drag: Option<interact::Drag>,
    pub(crate) last_click: (u32, u64, u32),
    /// Selected text in a text node: (node, anchor, caret) as byte offsets.
    pub text_sel: Option<(u32, usize, usize)>,
    /// A press on selectable text that may turn into a selection drag:
    /// (node, offset, x, y).
    pub(crate) text_press: Option<(u32, usize, f32, f32)>,
    scroll_to: Vec<u32>,
    /// The select whose popup list is open.
    pub open_select: u32,
    pub popup_hl: i32,
    /// Clock for animations (indeterminate progress), ms.
    pub anim_ms: u64,
    /// Nodes that painted an animation frame and want another.
    pub animated: Vec<u32>,
    /// Skip the window background (overlays).
    pub transparent: bool,
    /// When the user last pressed a key or button.
    pub last_input: std::time::Instant,
    /// The app asked to read the clipboard (request ids).
    pub app_paste: Vec<u32>,
    /// Decoded image sizes in px (= logical px at natural size).
    pub image_sizes: HashMap<u32, (f32, f32)>,
    /// Image nodes whose `src` or `data` changed.
    pub image_dirty: Vec<u32>,
    /// Space reserved at the top for a runtime-drawn title bar, logical px.
    pub top_inset: f32,
}

fn is_paint_only(kind: u8, p: u16) -> bool {
    match p {
        prop::VARIANT | prop::TOOLTIP => kind != kind::TEXT,
        prop::TAG | prop::DISABLED => true,
        _ => match kind {
            kind::GRID => p == prop::grid::SELECTED || p == prop::grid::CLICKABLE,
            kind::TEXT => p == prop::text::SPANS,
            kind::BUTTON => p == prop::button::DISABLED,
            kind::CHECKBOX => p == prop::checkbox::CHECKED || p == prop::checkbox::DISABLED,
            kind::INPUT => p == prop::input::DISABLED || p == prop::input::PLACEHOLDER,
            kind::SELECT => p == prop::select::SELECTED || p == prop::select::DISABLED,
            kind::TABS => p == prop::tabs::SELECTED,
            kind::PROGRESS => p == prop::progress::VALUE,
            kind::SCROLL => p == prop::scroll::STICK_END,
            _ => false,
        },
    }
}

impl Ui {
    pub fn new(fonts: Fonts, theme: theme::Theme) -> Ui {
        Ui {
            nodes: HashMap::new(),
            root: 0,
            theme,
            fonts,
            font_ids: HashMap::new(),
            scale: 1.0,
            win_w: 0.0,
            win_h: 0.0,
            focus: 0,
            hover: 0,
            press: 0,
            focus_visible: false,
            layout_dirty: true,
            damage: Vec::new(),
            out: Vec::new(),
            cursor: Cursor::Default,
            clipboard_out: None,
            paste_into: 0,
            quit: false,
            quit_requested: false,
            mods: 0,
            ptr: (-1.0, -1.0),
            drag: None,
            last_click: (0, 0, 0),
            text_sel: None,
            text_press: None,
            scroll_to: Vec::new(),
            open_select: 0,
            popup_hl: -1,
            anim_ms: 0,
            animated: Vec::new(),
            transparent: false,
            last_input: std::time::Instant::now() - std::time::Duration::from_secs(60),
            app_paste: Vec::new(),
            image_sizes: HashMap::new(),
            image_dirty: Vec::new(),
            top_inset: 0.0,
        }
    }

    /// Forgets the whole tree (an app restart).
    pub fn reset(&mut self) {
        self.nodes.clear();
        self.root = 0;
        self.focus = 0;
        self.hover = 0;
        self.press = 0;
        self.drag = None;
        self.open_select = 0;
        self.paste_into = 0;
        self.scroll_to.clear();
        self.animated.clear();
        self.out.clear();
        self.layout_dirty = true;
        self.damage_all();
    }

    pub fn node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn set_window(&mut self, w_px: i32, h_px: i32, scale: f32) {
        let (w, h) = (w_px as f32 / scale, h_px as f32 / scale);
        if (w, h, scale) != (self.win_w, self.win_h, self.scale) {
            if scale != self.scale {
                // Text measurements do not depend on scale, but pixel snapping does.
                self.invalidate_all();
            }
            self.win_w = w;
            self.win_h = h;
            self.scale = scale;
            self.layout_dirty = true;
            if let Some(r) = self.nodes.get_mut(&self.root) {
                r.dirty = true;
            }
            self.damage_all();
        }
    }

    /// Matches another tree's window size and scale.
    pub fn match_window(&mut self, o: &Ui) {
        if self.top_inset != o.top_inset {
            self.top_inset = o.top_inset;
            self.layout_dirty = true;
            if let Some(r) = self.nodes.get_mut(&self.root) {
                r.dirty = true;
            }
        }
        if (self.win_w, self.win_h, self.scale) != (o.win_w, o.win_h, o.scale) {
            let s = o.scale;
            self.set_window((o.win_w * s).round() as i32, (o.win_h * s).round() as i32, s);
        }
    }

    pub fn damage_all(&mut self) {
        self.damage.clear();
        self.damage.push(RectF::new(0.0, 0.0, self.win_w.max(1.0), self.win_h.max(1.0)));
    }

    pub fn damage_rect(&mut self, r: RectF) {
        if r.w > 0.0 && r.h > 0.0 {
            // Room for focus rings and antialiasing.
            self.damage.push(RectF::new(r.x - 4.0, r.y - 4.0, r.w + 8.0, r.h + 8.0));
        }
    }

    pub fn damage_node(&mut self, id: u32) {
        if let Some(n) = self.nodes.get(&id) {
            let r = n.rect;
            self.damage_rect(r);
        }
    }

    /// Marks a node's size as changed: clears its measurements and those of its ancestors.
    pub fn invalidate(&mut self, id: u32) {
        let mut cur = id;
        while let Some(n) = self.nodes.get_mut(&cur) {
            n.m = Measure::default();
            n.dirty = true;
            cur = n.parent;
        }
        self.layout_dirty = true;
    }

    fn invalidate_subtree(&mut self, id: u32) {
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if let Some(n) = self.nodes.get_mut(&i) {
                n.m = Measure::default();
                n.dirty = true;
                stack.extend(n.children.iter().copied());
            }
        }
        self.invalidate(id);
    }

    pub fn invalidate_all(&mut self) {
        for n in self.nodes.values_mut() {
            n.m = Measure::default();
            n.dirty = true;
        }
        self.font_ids.clear();
        self.layout_dirty = true;
    }

    /// A node's `tag` prop (0 if unset).
    pub fn tag_of(&self, id: u32) -> i32 {
        self.nodes.get(&id).map_or(0, |n| n.i(prop::TAG, 0))
    }

    fn is_ancestor(&self, a: u32, mut b: u32) -> bool {
        while b != 0 {
            if a == b {
                return true;
            }
            b = self.nodes.get(&b).map(|n| n.parent).unwrap_or(0);
        }
        false
    }

    fn detach(&mut self, id: u32) {
        let Some(n) = self.nodes.get_mut(&id) else { return };
        let p = n.parent;
        if p == 0 {
            return;
        }
        n.parent = 0;
        let r = n.rect;
        if let Some(pn) = self.nodes.get_mut(&p) {
            pn.children.retain(|&c| c != id);
        }
        self.damage_rect(r);
        self.invalidate(p);
        self.blur_if_inside(id);
    }

    fn blur_if_inside(&mut self, id: u32) {
        if self.focus != 0 && self.is_ancestor(id, self.focus) {
            self.set_focus(0, false);
        }
        if self.hover != 0 && self.is_ancestor(id, self.hover) {
            self.hover = 0;
        }
        if self.press != 0 && self.is_ancestor(id, self.press) {
            self.press = 0;
        }
    }

    /// True if the node or one of its ancestors is disabled.
    pub fn is_disabled(&self, id: u32) -> bool {
        let mut cur = id;
        while let Some(n) = self.nodes.get(&cur) {
            if n.disabled() {
                return true;
            }
            cur = n.parent;
        }
        false
    }

    fn destroy(&mut self, id: u32) {
        // Focus inside the subtree moves to the next focusable node (or the
        // one before), so keyboard use can carry on after a row is deleted.
        let mut next = None;
        if self.focus != 0 && self.is_ancestor(id, self.focus) {
            let order = self.focus_order();
            if let Some(pos) = order.iter().position(|&f| f == self.focus) {
                let outside = |f: &&u32| !self.is_ancestor(id, **f);
                next = order[pos + 1..].iter().find(outside).or_else(|| order[..pos].iter().rev().find(outside)).map(|&f| (f, self.focus_visible));
            }
        }
        self.detach(id);
        self.blur_if_inside(id);
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if let Some(n) = self.nodes.remove(&i) {
                stack.extend(n.children);
            }
        }
        if let Some((f, visible)) = next {
            self.set_focus(f, visible);
        }
        if self.is_ancestor(id, self.root) || self.root == id {
            self.root = 0;
            self.damage_all();
        }
        if self.paste_into == id {
            self.paste_into = 0;
        }
    }

    fn attach(&mut self, parent: u32, child: u32, before: Option<u32>) -> Result<(), String> {
        let pk = self.nodes.get(&parent).ok_or("no such parent")?.kind;
        if !self.nodes.contains_key(&child) {
            return Err("no such child".into());
        }
        if pk != kind::GRID && pk != kind::SCROLL {
            return Err("parent is not a container".into());
        }
        if self.is_ancestor(child, parent) {
            return Err("would create a cycle".into());
        }
        if child == self.root {
            return Err("the root cannot be a child".into());
        }
        self.detach(child);
        let pn = self.nodes.get_mut(&parent).unwrap();
        if pk == kind::SCROLL && !pn.children.is_empty() {
            return Err("a scroll has exactly one child".into());
        }
        match before {
            Some(b) => {
                let i = pn.children.iter().position(|&c| c == b).ok_or("`before` is not a child of parent")?;
                pn.children.insert(i, child);
            }
            None => pn.children.push(child),
        }
        self.nodes.get_mut(&child).unwrap().parent = parent;
        self.invalidate_subtree(child);
        Ok(())
    }

    /// Applies a patch. On error, ops before the bad one stay applied.
    pub fn apply_patch(&mut self, b: &[u8]) -> Result<(), PatchError> {
        let mut r = Reader::new(b);
        while !r.done() {
            let at = r.pos;
            let fail = |msg: &str| PatchError { offset: at, msg: msg.to_string() };
            let opc = r.u8().ok_or_else(|| fail("truncated"))?;
            let res: Result<(), String> = (|| {
                let trunc = || "truncated op".to_string();
                match opc {
                    op::CREATE => {
                        let id = r.u32().ok_or_else(trunc)?;
                        let k = r.u8().ok_or_else(trunc)?;
                        if id == 0 || self.nodes.contains_key(&id) {
                            return Err(format!("node {id} already exists or is 0"));
                        }
                        if !kind::is_builtin(k) {
                            return Err(format!("unknown kind {k}"));
                        }
                        self.nodes.insert(id, Node::new(k));
                    }
                    op::DESTROY => {
                        let id = r.u32().ok_or_else(trunc)?;
                        if !self.nodes.contains_key(&id) {
                            return Err(format!("no node {id}"));
                        }
                        self.destroy(id);
                    }
                    op::APPEND => {
                        let (p, c) = (r.u32().ok_or_else(trunc)?, r.u32().ok_or_else(trunc)?);
                        self.attach(p, c, None)?;
                    }
                    op::INSERT => {
                        let (p, c, bf) = (r.u32().ok_or_else(trunc)?, r.u32().ok_or_else(trunc)?, r.u32().ok_or_else(trunc)?);
                        self.attach(p, c, Some(bf))?;
                    }
                    op::DETACH => {
                        let id = r.u32().ok_or_else(trunc)?;
                        if !self.nodes.contains_key(&id) {
                            return Err(format!("no node {id}"));
                        }
                        self.detach(id);
                    }
                    op::SET => {
                        let id = r.u32().ok_or_else(trunc)?;
                        let p = r.u16().ok_or_else(trunc)?;
                        let v = r.value().ok_or("bad value")?;
                        self.set_prop(id, p, Val::from_wire(&v))?;
                    }
                    op::CTX => {
                        let id = r.u32().ok_or_else(trunc)?;
                        let key = r.str().ok_or_else(trunc)?.to_string();
                        let v = r.value().ok_or("bad value")?;
                        self.set_ctx(id, key, Val::from_wire(&v))?;
                    }
                    op::ROOT => {
                        let id = r.u32().ok_or_else(trunc)?;
                        let n = self.nodes.get(&id).ok_or(format!("no node {id}"))?;
                        if n.parent != 0 {
                            return Err("the root must be detached".into());
                        }
                        self.root = id;
                        self.invalidate(id);
                        self.damage_all();
                    }
                    op::FOCUS => {
                        let id = r.u32().ok_or_else(trunc)?;
                        if id != 0 && !self.nodes.contains_key(&id) {
                            return Err(format!("no node {id}"));
                        }
                        self.set_focus(id, false);
                    }
                    op::REPLACE => {
                        let (old, new) = (r.u32().ok_or_else(trunc)?, r.u32().ok_or_else(trunc)?);
                        for id in [old, new] {
                            if !self.nodes.contains_key(&id) {
                                return Err(format!("no node {id}"));
                            }
                        }
                        if self.is_ancestor(old, new) {
                            return Err("the new node is inside the old one".into());
                        }
                        if new == self.root {
                            return Err("the root cannot replace a node".into());
                        }
                        self.detach(new);
                        let parent = self.nodes[&old].parent;
                        if parent != 0 {
                            let sibs = &self.nodes[&parent].children;
                            let next = sibs.iter().position(|&c| c == old).and_then(|i| sibs.get(i + 1).copied());
                            self.detach(old);
                            self.attach(parent, new, next)?;
                        } else if self.root == old {
                            self.root = new;
                            self.invalidate(new);
                            self.damage_all();
                        }
                        self.destroy(old);
                    }
                    op::SCROLL_TO => {
                        let id = r.u32().ok_or_else(trunc)?;
                        if !self.nodes.contains_key(&id) {
                            return Err(format!("no node {id}"));
                        }
                        self.scroll_to.push(id);
                        self.layout_dirty = true;
                    }
                    _ => return Err(format!("unknown op {opc}")),
                }
                Ok(())
            })();
            res.map_err(|m| fail(&m))?;
        }
        Ok(())
    }

    pub fn set_prop(&mut self, id: u32, p: u16, v: Val) -> Result<(), String> {
        let n = self.nodes.get_mut(&id).ok_or(format!("no node {id}"))?;
        let k = n.kind;
        let old = n.props.iter().position(|(q, _)| *q == p);
        match (&v, old) {
            (Val::None, None) => return Ok(()),
            (Val::None, Some(i)) => {
                n.props.remove(i);
            }
            (_, Some(i)) if n.props[i].1 == v => return Ok(()),
            (_, Some(i)) => n.props[i].1 = v.clone(),
            (_, None) => n.props.push((p, v.clone())),
        }
        if k == kind::INPUT && p == prop::input::VALUE {
            let s = v.as_str().unwrap_or("").to_string();
            if let Some(e) = n.edit() {
                e.set_text(&s);
            }
        }
        if k == kind::TEXT && p == prop::text::TEXT {
            n.m.lines = None;
        }
        if k == kind::IMAGE && (p == prop::image::SRC || p == prop::image::DATA) && !self.image_dirty.contains(&id) {
            self.image_dirty.push(id);
        }
        if p == prop::HIDDEN && v != Val::None && self.focus != 0 && self.is_ancestor(id, self.focus) {
            self.set_focus(0, false);
        }
        self.damage_node(id);
        if !is_paint_only(k, p) {
            self.invalidate(id);
        }
        Ok(())
    }

    fn set_ctx(&mut self, id: u32, key: String, v: Val) -> Result<(), String> {
        let n = self.nodes.get_mut(&id).ok_or(format!("no node {id}"))?;
        let old = n.ctx.iter().position(|(k, _)| *k == key);
        match (&v, old) {
            (Val::None, None) => return Ok(()),
            (Val::None, Some(i)) => {
                n.ctx.remove(i);
            }
            (_, Some(i)) if n.ctx[i].1 == v => return Ok(()),
            (_, Some(i)) => n.ctx[i].1 = v,
            (_, None) => n.ctx.push((key.clone(), v)),
        }
        if key.starts_with("font.") || key.starts_with("space.") || key == "line.height" {
            self.invalidate_subtree(id);
        }
        self.damage_node(id);
        Ok(())
    }

    pub fn take_scroll_to(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.scroll_to)
    }

    // ---- Tokens ----

    pub fn tok(&self, id: u32, key: &str) -> Option<&Val> {
        if let Some(v) = self.theme.locked(key) {
            return Some(v);
        }
        let mut cur = id;
        while let Some(n) = self.nodes.get(&cur) {
            if let Some((_, v)) = n.ctx.iter().find(|(k, _)| k == key) {
                return Some(v);
            }
            cur = n.parent;
        }
        self.theme.base(key)
    }

    /// The first of `keys` that is set, as a color.
    pub fn color(&self, id: u32, keys: &[&str]) -> Color {
        keys.iter().find_map(|k| self.tok(id, k).and_then(|v| v.as_color())).unwrap_or(Color::hex(0xff00ffff))
    }

    pub fn color_opt(&self, id: u32, keys: &[&str]) -> Option<Color> {
        keys.iter().find_map(|k| self.tok(id, k).and_then(|v| v.as_color()))
    }

    pub fn num(&self, id: u32, keys: &[&str], def: f32) -> f32 {
        keys.iter().find_map(|k| self.tok(id, k).and_then(|v| v.as_f32())).unwrap_or(def)
    }

    pub fn variant(&self, id: u32) -> &str {
        self.nodes.get(&id).map(|n| n.s(prop::VARIANT)).unwrap_or("")
    }

    fn load_family(&mut self, family: &str, bold: bool, defaults: &[&str], bold_defaults: &[&str]) -> FontId {
        let key = (family.to_string(), bold);
        if let Some(&f) = self.font_ids.get(&key) {
            return f;
        }
        if self.font_ids.is_empty() {
            match self.tok(0, "font.fallback").and_then(|v| v.as_str()).map(|s| s.to_string()) {
                Some(list) => self.fonts.set_fallback(&list.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>()),
                None => self.fonts.set_fallback(text::DEFAULT_FALLBACK),
            }
        }
        let regular = if family.is_empty() { self.fonts.find_first(defaults) } else { self.fonts.find(family) };
        let regular = regular.or_else(|| self.fonts.find_first(text::DEFAULT_SANS)).unwrap_or(0);
        let id = if bold {
            let b = if family.is_empty() { self.fonts.find_first(bold_defaults) } else { None };
            b.unwrap_or_else(|| self.fonts.synth_bold(regular))
        } else {
            regular
        };
        self.font_ids.insert(key, id);
        id
    }

    /// Font, size and line height for a text style (`body`, `title`, `heading`, `caption`, `mono`).
    pub fn text_style(&mut self, id: u32, style: &str) -> TextStyle {
        let style = if style.is_empty() { "body" } else { style };
        let size_key = format!("font.{style}.size");
        let weight_key = format!("font.{style}.weight");
        let fam_key = format!("font.{style}.family");
        let size = self.num(id, &[&size_key, "font.body.size"], 14.0) * self.num(id, &["font.scale"], 1.0);
        let weight = self.num(id, &[&weight_key], 400.0);
        let bold = weight >= 600.0;
        let mono = style == "mono";
        let (fkey, family) = if let Some(f) = self.tok(id, &fam_key).and_then(|v| v.as_str()) {
            (fam_key.clone(), f.to_string())
        } else if mono {
            ("font.mono.family".to_string(), String::new())
        } else {
            ("font.family".to_string(), self.tok(id, "font.family").and_then(|v| v.as_str()).unwrap_or("").to_string())
        };
        let font = if bold {
            let bold_key = format!("{fkey}.bold");
            match self.tok(id, &bold_key).and_then(|v| v.as_str()).map(|s| s.to_string()) {
                Some(b) => self.load_family(&b, false, text::DEFAULT_SANS, text::DEFAULT_SANS_BOLD),
                None if mono => self.load_family(&family, true, text::DEFAULT_MONO, &["DejaVuSansMono-Bold.ttf"]),
                None => self.load_family(&family, true, text::DEFAULT_SANS, text::DEFAULT_SANS_BOLD),
            }
        } else if mono {
            self.load_family(&family, false, text::DEFAULT_MONO, &[])
        } else {
            self.load_family(&family, false, text::DEFAULT_SANS, &[])
        };
        let lh = size * self.num(id, &["line.height"], 1.35);
        TextStyle { font, size, line_h: lh.round().max(1.0) }
    }

    /// Request that `id`'s nearest scroll ancestors show it.
    pub fn request_scroll_to(&mut self, id: u32) {
        self.scroll_to.push(id);
        self.layout_dirty = true;
    }

    /// Tree-order list of focusable, visible nodes.
    pub fn focus_order(&self) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            let Some(n) = self.nodes.get(&id) else { continue };
            if n.hidden() || n.disabled() {
                continue;
            }
            if n.focusable() {
                out.push(id);
            }
            stack.extend(n.children.iter().rev());
        }
        out
    }

    pub fn set_focus(&mut self, id: u32, visible: bool) {
        if id == self.focus {
            self.focus_visible = visible;
            self.damage_node(id);
            return;
        }
        let old = self.focus;
        if old != 0 {
            self.damage_node(old);
            if let Some(n) = self.nodes.get_mut(&old) {
                if n.kind == kind::INPUT {
                    if let Some(e) = n.edit() {
                        e.anchor = e.caret;
                    }
                    self.out.push(UiEvent::FocusOut(old));
                }
            }
        }
        self.focus = id;
        self.focus_visible = visible;
        if id != 0 {
            self.damage_node(id);
        }
    }

    pub fn emit(&mut self, e: UiEvent) {
        self.out.push(e);
    }

    pub fn take_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.out)
    }
}
