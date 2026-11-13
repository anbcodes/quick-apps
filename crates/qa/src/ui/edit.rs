//! Text editing for `input`: caret, selection, readline keys, undo.

pub mod keys {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const RETURN: u32 = 0xff0d;
    pub const ESCAPE: u32 = 0xff1b;
    pub const DELETE: u32 = 0xffff;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    pub const SPACE: u32 = 0x20;
}

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;

#[derive(Default)]
pub struct Edit {
    pub text: String,
    /// Byte offsets.
    pub caret: usize,
    pub anchor: usize,
    /// Horizontal scroll of a single-line field, logical px.
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// Preferred x for up/down movement.
    pub goal_x: Option<f32>,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    /// Consecutive typing is merged into one undo step.
    typing: bool,
}

/// What a key did to an input.
pub enum EditResult {
    Ignored,
    Moved,
    Changed,
    Submit,
    Copy(String),
    Cut(String),
    Paste,
}

pub(crate) fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Edit {
    pub fn set_text(&mut self, s: &str) {
        if self.text != s {
            // A value put into an empty input leaves the caret after it,
            // ready for typing to carry on.
            let was_empty = self.text.is_empty();
            self.text = s.to_string();
            self.caret = if was_empty { self.text.len() } else { self.caret.min(self.text.len()) };
            while !self.text.is_char_boundary(self.caret) {
                self.caret -= 1;
            }
            self.anchor = self.caret;
            self.undo.clear();
            self.redo.clear();
        }
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected(&self) -> &str {
        let (a, b) = self.selection();
        &self.text[a..b]
    }

    fn snapshot(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push((self.text.clone(), self.caret));
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
    }

    fn prev(&self, i: usize) -> usize {
        self.text[..i].char_indices().next_back().map(|(j, _)| j).unwrap_or(0)
    }

    fn next(&self, i: usize) -> usize {
        self.text[i..].chars().next().map(|c| i + c.len_utf8()).unwrap_or(i)
    }

    fn word_left(&self, mut i: usize) -> usize {
        while i > 0 && !is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        while i > 0 && is_word(self.text[..i].chars().next_back().unwrap()) {
            i = self.prev(i);
        }
        i
    }

    fn word_right(&self, mut i: usize) -> usize {
        let n = self.text.len();
        while i < n && !is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        while i < n && is_word(self.text[i..].chars().next().unwrap()) {
            i = self.next(i);
        }
        i
    }

    pub fn line_start(&self, i: usize) -> usize {
        self.text[..i].rfind('\n').map(|j| j + 1).unwrap_or(0)
    }

    pub fn line_end(&self, i: usize) -> usize {
        self.text[i..].find('\n').map(|j| i + j).unwrap_or(self.text.len())
    }

    /// The word around byte `i`, for double-click selection.
    pub fn word_at(&self, i: usize) -> (usize, usize) {
        let mut a = i;
        while a > 0 && is_word(self.text[..a].chars().next_back().unwrap()) {
            a = self.prev(a);
        }
        let mut b = i;
        while b < self.text.len() && is_word(self.text[b..].chars().next().unwrap()) {
            b = self.next(b);
        }
        (a, b)
    }

    pub fn insert(&mut self, s: &str, typing: bool) {
        self.snapshot(typing && !self.has_selection());
        let (a, b) = self.selection();
        self.text.replace_range(a..b, s);
        self.caret = a + s.len();
        self.anchor = self.caret;
        self.goal_x = None;
    }

    fn delete_range(&mut self, a: usize, b: usize) {
        if a == b {
            return;
        }
        self.snapshot(false);
        self.text.replace_range(a..b, "");
        self.caret = a;
        self.anchor = a;
        self.goal_x = None;
    }

    fn move_to(&mut self, i: usize, select: bool) {
        self.caret = i;
        if !select {
            self.anchor = i;
        }
        self.typing = false;
        self.goal_x = None;
    }

    /// Handles a key. `multiline` changes Enter and Up/Down.
    /// Up/Down in multiline fields are handled by the caller, which knows the layout.
    pub fn key(&mut self, sym: u32, text: &str, mods: u8, multiline: bool) -> EditResult {
        use keys::*;
        let ctrl = mods & CTRL != 0;
        let shift = mods & SHIFT != 0;
        let alt = mods & ALT != 0;
        let before = (self.text.len(), self.caret, self.anchor);
        let changed_marker = self.undo.len();
        let lower = if ctrl && (0x41..=0x5a).contains(&sym) { sym + 0x20 } else { sym };
        match lower {
            RETURN | KP_ENTER => {
                if multiline && !ctrl {
                    self.insert("\n", false);
                    return EditResult::Changed;
                }
                return EditResult::Submit;
            }
            BACKSPACE => {
                if self.has_selection() {
                    let (a, b) = self.selection();
                    self.delete_range(a, b);
                } else if ctrl || alt {
                    let w = self.word_left(self.caret);
                    self.delete_range(w, self.caret);
                } else if self.caret > 0 {
                    let p = self.prev(self.caret);
                    self.delete_range(p, self.caret);
                }
            }
            DELETE => {
                if self.has_selection() {
                    let (a, b) = self.selection();
                    self.delete_range(a, b);
                } else if ctrl {
                    let w = self.word_right(self.caret);
                    self.delete_range(self.caret, w);
                } else if self.caret < self.text.len() {
                    let n = self.next(self.caret);
                    self.delete_range(self.caret, n);
                }
            }
            LEFT => {
                let i = if ctrl {
                    self.word_left(self.caret)
                } else if self.has_selection() && !shift {
                    self.selection().0
                } else {
                    self.prev(self.caret)
                };
                self.move_to(i, shift);
            }
            RIGHT => {
                let i = if ctrl {
                    self.word_right(self.caret)
                } else if self.has_selection() && !shift {
                    self.selection().1
                } else {
                    self.next(self.caret)
                };
                self.move_to(i, shift);
            }
            HOME => {
                let i = if ctrl || !multiline { 0 } else { self.line_start(self.caret) };
                self.move_to(i, shift);
            }
            END => {
                let i = if ctrl || !multiline { self.text.len() } else { self.line_end(self.caret) };
                self.move_to(i, shift);
            }
            _ if ctrl => match lower {
                0x61 => {
                    // Ctrl+A selects all, as in most GUI fields.
                    self.anchor = 0;
                    self.caret = self.text.len();
                }
                0x65 => {
                    let i = if multiline { self.line_end(self.caret) } else { self.text.len() };
                    self.move_to(i, shift);
                }
                0x6b => {
                    let e = if multiline { self.line_end(self.caret) } else { self.text.len() };
                    let e = if e == self.caret && e < self.text.len() { self.next(e) } else { e };
                    self.delete_range(self.caret, e);
                }
                0x75 => {
                    let s = if multiline { self.line_start(self.caret) } else { 0 };
                    self.delete_range(s, self.caret);
                }
                0x77 => {
                    let w = self.word_left(self.caret);
                    self.delete_range(w, self.caret);
                }
                0x63 => {
                    return if self.has_selection() { EditResult::Copy(self.selected().to_string()) } else { EditResult::Ignored };
                }
                0x78 => {
                    if !self.has_selection() {
                        return EditResult::Ignored;
                    }
                    let s = self.selected().to_string();
                    let (a, b) = self.selection();
                    self.delete_range(a, b);
                    return EditResult::Cut(s);
                }
                0x76 => return EditResult::Paste,
                0x7a => {
                    if shift {
                        self.redo();
                    } else {
                        self.undo();
                    }
                }
                0x79 => self.redo(),
                _ => return EditResult::Ignored,
            },
            _ => {
                if text.is_empty() || alt {
                    return EditResult::Ignored;
                }
                let t: String = text.chars().filter(|c| !c.is_control() || (*c == '\n' && multiline)).collect();
                if t.is_empty() {
                    return EditResult::Ignored;
                }
                self.insert(&t, true);
                return EditResult::Changed;
            }
        }
        if self.undo.len() != changed_marker || before.0 != self.text.len() {
            EditResult::Changed
        } else if before != (self.text.len(), self.caret, self.anchor) {
            EditResult::Moved
        } else {
            // Keys an input owns (arrows at the end, Home…) are still consumed.
            EditResult::Moved
        }
    }

    pub fn undo(&mut self) {
        if let Some((t, c)) = self.undo.pop() {
            self.redo.push((std::mem::replace(&mut self.text, t), self.caret));
            self.caret = c.min(self.text.len());
            self.anchor = self.caret;
        }
        self.typing = false;
    }

    pub fn redo(&mut self) {
        if let Some((t, c)) = self.redo.pop() {
            self.undo.push((std::mem::replace(&mut self.text, t), self.caret));
            self.caret = c.min(self.text.len());
            self.anchor = self.caret;
        }
        self.typing = false;
    }
}
