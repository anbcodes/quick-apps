//! Event encoding (docs/events.md).

use crate::ui::UiEvent;
use qa_proto::{ev, Sink};

/// `tag` (the node's tag prop) is appended to click, change, submit,
/// toggle, select and link events.
pub fn ui_event(e: &UiEvent, tag: i32) -> Vec<u8> {
    let mut b = Vec::with_capacity(16);
    match e {
        UiEvent::Click(n) => {
            b.u8(ev::CLICK);
            b.u32(*n);
        }
        UiEvent::Change(n, s) => {
            b.u8(ev::CHANGE);
            b.u32(*n);
            b.str(s);
        }
        UiEvent::Submit(n, s) => {
            b.u8(ev::SUBMIT);
            b.u32(*n);
            b.str(s);
        }
        UiEvent::Toggle(n, on) => {
            b.u8(ev::TOGGLE);
            b.u32(*n);
            b.u8(*on as u8);
        }
        UiEvent::Select(n, i) => {
            b.u8(ev::SELECT);
            b.u32(*n);
            b.i32(*i);
        }
        UiEvent::Key { node, sym, mods, text } => {
            b.u8(ev::KEY);
            b.u32(*node);
            b.u32(*sym);
            b.u8(*mods);
            b.str(text);
        }
        UiEvent::Pointer { node, kind, x, y, buttons, dx, dy } => {
            b.u8(ev::POINTER);
            b.u32(*node);
            b.u8(*kind);
            b.f32(*x);
            b.f32(*y);
            b.u8(*buttons);
            b.f32(*dx);
            b.f32(*dy);
        }
        UiEvent::FocusOut(n) => {
            b.u8(ev::FOCUS_OUT);
            b.u32(*n);
        }
        UiEvent::NearEnd(n) => {
            b.u8(ev::NEAR_END);
            b.u32(*n);
        }
        UiEvent::Link(n, i) => {
            b.u8(ev::LINK);
            b.u32(*n);
            b.u32(*i);
        }
    }
    if matches!(e, UiEvent::Click(_) | UiEvent::Change(..) | UiEvent::Submit(..) | UiEvent::Toggle(..) | UiEvent::Select(..) | UiEvent::Link(..)) {
        b.i32(tag);
    }
    b
}

pub fn resize(w: f32, h: f32, scale: f32) -> Vec<u8> {
    let mut b = vec![ev::RESIZE];
    b.f32(w);
    b.f32(h);
    b.f32(scale);
    b
}

pub fn timer(id: u32) -> Vec<u8> {
    let mut b = vec![ev::TIMER];
    b.u32(id);
    b
}

pub fn simple(t: u8) -> Vec<u8> {
    vec![t]
}

pub fn request_done(req: u32, result: i32, info: &str) -> Vec<u8> {
    let mut b = vec![ev::REQUEST_DONE];
    b.u32(req);
    b.i32(result);
    b.str(info);
    b
}
