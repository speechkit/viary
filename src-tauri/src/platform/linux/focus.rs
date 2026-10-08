//! Whether the focused UI element can take typed text: unknown, so always
//! assumed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editable,
    NotEditable,
}

pub fn focused(_pid: i32) -> Focus {
    Focus::Editable
}
