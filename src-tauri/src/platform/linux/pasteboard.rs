//! The clipboard: not connected yet.

pub struct Saved;

pub fn save() -> Saved {
    Saved
}

pub fn set_text(_text: &str) -> isize {
    0
}

pub fn set_transient_text(_text: &str) -> isize {
    0
}

pub fn restore(_saved: Saved, _ours: isize) {}
