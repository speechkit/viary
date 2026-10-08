// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // `viary --toggle`, from the GNOME custom shortcut: tell the running
    // Viary and stop. With none running, start one.
    #[cfg(target_os = "linux")]
    if std::env::args().any(|a| a == "--toggle") && viary_lib::send_toggle() {
        return;
    }
    viary_lib::run();
}
