//! Whether the focused UI element can take typed text. Wayland does not
//! say, and AT-SPI is not asked yet: anything gets the paste, which is
//! harmless where nothing takes text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Editable,
    NotEditable,
}

pub fn focused(_pid: i32) -> Focus {
    Focus::Editable
}
