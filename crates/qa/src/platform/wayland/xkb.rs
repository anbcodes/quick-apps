//! libxkbcommon, loaded with dlopen so the binary has no link-time dependency.

use std::ffi::{c_char, c_int, c_void, CStr};

type P = *mut c_void;

struct Fns {
    context_new: unsafe extern "C" fn(c_int) -> P,
    context_unref: unsafe extern "C" fn(P),
    keymap_new_from_string: unsafe extern "C" fn(P, *const c_char, c_int, c_int) -> P,
    keymap_unref: unsafe extern "C" fn(P),
    keymap_key_repeats: unsafe extern "C" fn(P, u32) -> c_int,
    state_new: unsafe extern "C" fn(P) -> P,
    state_unref: unsafe extern "C" fn(P),
    state_update_mask: unsafe extern "C" fn(P, u32, u32, u32, u32, u32, u32) -> c_int,
    state_key_get_one_sym: unsafe extern "C" fn(P, u32) -> u32,
    state_key_get_utf8: unsafe extern "C" fn(P, u32, *mut c_char, usize) -> c_int,
    state_mod_name_is_active: unsafe extern "C" fn(P, *const c_char, c_int) -> c_int,
}

pub struct Xkb {
    f: Fns,
    ctx: P,
    keymap: P,
    state: P,
}

unsafe fn sym<T>(lib: P, name: &CStr) -> Option<T> {
    let p = libc::dlsym(lib, name.as_ptr());
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute_copy(&p))
    }
}

impl Xkb {
    pub fn load() -> Option<Xkb> {
        unsafe {
            let mut lib = libc::dlopen(c"libxkbcommon.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            if lib.is_null() {
                lib = libc::dlopen(c"libxkbcommon.so".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
            }
            if lib.is_null() {
                return None;
            }
            let f = Fns {
                context_new: sym(lib, c"xkb_context_new")?,
                context_unref: sym(lib, c"xkb_context_unref")?,
                keymap_new_from_string: sym(lib, c"xkb_keymap_new_from_string")?,
                keymap_unref: sym(lib, c"xkb_keymap_unref")?,
                keymap_key_repeats: sym(lib, c"xkb_keymap_key_repeats")?,
                state_new: sym(lib, c"xkb_state_new")?,
                state_unref: sym(lib, c"xkb_state_unref")?,
                state_update_mask: sym(lib, c"xkb_state_update_mask")?,
                state_key_get_one_sym: sym(lib, c"xkb_state_key_get_one_sym")?,
                state_key_get_utf8: sym(lib, c"xkb_state_key_get_utf8")?,
                state_mod_name_is_active: sym(lib, c"xkb_state_mod_name_is_active")?,
            };
            let ctx = (f.context_new)(0);
            if ctx.is_null() {
                return None;
            }
            Some(Xkb { f, ctx, keymap: std::ptr::null_mut(), state: std::ptr::null_mut() })
        }
    }

    /// `text` is the NUL-terminated keymap the compositor sent.
    pub fn set_keymap(&mut self, text: *const c_char) {
        unsafe {
            let km = (self.f.keymap_new_from_string)(self.ctx, text, 1, 0);
            if km.is_null() {
                return;
            }
            let st = (self.f.state_new)(km);
            if st.is_null() {
                (self.f.keymap_unref)(km);
                return;
            }
            self.drop_map();
            self.keymap = km;
            self.state = st;
        }
    }

    fn drop_map(&mut self) {
        unsafe {
            if !self.state.is_null() {
                (self.f.state_unref)(self.state);
            }
            if !self.keymap.is_null() {
                (self.f.keymap_unref)(self.keymap);
            }
        }
    }

    pub fn ready(&self) -> bool {
        !self.state.is_null()
    }

    pub fn update_mask(&mut self, dep: u32, lat: u32, lock: u32, group: u32) {
        if self.ready() {
            unsafe { (self.f.state_update_mask)(self.state, dep, lat, lock, 0, 0, group) };
        }
    }

    /// Modifier bits: 1 Shift, 2 Ctrl, 4 Alt, 8 Super.
    pub fn mods(&self) -> u8 {
        if !self.ready() {
            return 0;
        }
        let mut m = 0;
        for (bit, name) in [(1, c"Shift"), (2, c"Control"), (4, c"Mod1"), (8, c"Mod4")] {
            if unsafe { (self.f.state_mod_name_is_active)(self.state, name.as_ptr(), 8) } > 0 {
                m |= bit;
            }
        }
        m
    }

    /// `code` is an evdev key code (the Wayland one); xkb adds 8.
    pub fn sym(&self, code: u32) -> u32 {
        if !self.ready() {
            return 0;
        }
        unsafe { (self.f.state_key_get_one_sym)(self.state, code + 8) }
    }

    pub fn text(&self, code: u32) -> String {
        if !self.ready() {
            return String::new();
        }
        let mut buf = [0 as c_char; 64];
        let n = unsafe { (self.f.state_key_get_utf8)(self.state, code + 8, buf.as_mut_ptr(), buf.len()) };
        if n <= 0 {
            return String::new();
        }
        let s = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
        // Control characters are not text.
        if s.chars().all(|c| c.is_control()) {
            String::new()
        } else {
            s
        }
    }

    pub fn repeats(&self, code: u32) -> bool {
        self.ready() && unsafe { (self.f.keymap_key_repeats)(self.keymap, code + 8) } != 0
    }
}

impl Drop for Xkb {
    fn drop(&mut self) {
        self.drop_map();
        unsafe { (self.f.context_unref)(self.ctx) };
    }
}
