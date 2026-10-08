//! Synthetic Ctrl+V and Ctrl+Z: not available yet.

use tauri::AppHandle;

pub fn paste(_: &AppHandle) -> Result<(), String> {
    Err("typing into other apps is not available on Linux yet".into())
}

pub fn undo(_: &AppHandle) -> Result<(), String> {
    Err("typing into other apps is not available on Linux yet".into())
}
