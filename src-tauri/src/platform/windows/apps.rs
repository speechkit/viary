//! The app the user is dictating into: the foreground window and its
//! process.

use std::{
    ffi::c_void,
    path::Path,
    time::{Duration, Instant},
};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE, HWND},
        Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::Threading::{
            AttachThreadInput, GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        UI::WindowsAndMessaging::{
            BringWindowToTop, GW_HWNDNEXT, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow,
            GetTopWindow, GetWindow, GetWindowLongW, GetWindowTextLengthW,
            GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, SW_RESTORE,
            SetForegroundWindow, ShowWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        },
    },
    core::{PCWSTR, PWSTR},
};

use super::wide;

/// The foreground window when a dictation started.
#[derive(Debug, Clone, Default)]
pub struct TargetApp {
    pub pid: i32,
    pub name: String,
    /// The window's handle, as an integer so the target can cross threads.
    pub window: isize,
}

impl TargetApp {
    pub fn is_self(&self) -> bool {
        self.pid == std::process::id().cast_signed()
    }

    fn hwnd(&self) -> HWND {
        HWND(self.window as *mut c_void)
    }
}

/// Windows that are never the app being dictated into: the taskbar, the
/// tray overflow, the desktop, and Start.
const SHELL: &[&str] = &[
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Progman",
    "WorkerW",
    "Windows.UI.Core.CoreWindow",
];

/// Whether `window` is an app window someone could be typing into, and
/// not a tray menu's hidden owner, the pill, or the shell.
fn is_app_window(window: HWND) -> bool {
    // SAFETY: plain queries on a window handle; the class buffer is a local.
    unsafe {
        if window.is_invalid() || !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() {
            return false;
        }
        let style = GetWindowLongW(window, GWL_EXSTYLE).cast_unsigned();
        if style & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 || GetWindowTextLengthW(window) == 0 {
            return false;
        }
        let mut class = [0_u16; 64];
        let len = GetClassNameW(window, &mut class);
        let class = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
        !SHELL.contains(&class.as_str())
    }
}

/// The foreground window; while the taskbar or a tray menu has the
/// foreground, the app window just below them.
fn front_window() -> Option<HWND> {
    // SAFETY: plain queries walking the top-level windows in z-order.
    unsafe {
        let foreground = GetForegroundWindow();
        if is_app_window(foreground) {
            return Some(foreground);
        }
        let mut window = GetTopWindow(None).ok()?;
        loop {
            if is_app_window(window) {
                return Some(window);
            }
            window = GetWindow(window, GW_HWNDNEXT).ok()?;
        }
    }
}

pub fn frontmost() -> Option<TargetApp> {
    let window = front_window()?;
    // SAFETY: plain Win32 queries; the out pointer is a local.
    unsafe {
        let mut pid = 0_u32;
        GetWindowThreadProcessId(window, Some(&raw mut pid));
        Some(TargetApp {
            pid: pid.cast_signed(),
            name: process_name(pid).unwrap_or_default(),
            window: window.0 as isize,
        })
    }
}

/// The app's name as Windows shows it in Task Manager: the executable's
/// FileDescription ("Microsoft Outlook"), else its file name ("OUTLOOK").
fn process_name(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed below; the buffer outlives the call.
    let path = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0_u16; 1024];
        let mut len = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &raw mut len,
        );
        let _ = CloseHandle(process);
        queried.ok()?;
        String::from_utf16_lossy(&buffer[..len as usize])
    };
    description(&path).or_else(|| {
        Path::new(&path)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
    })
}

fn description(path: &str) -> Option<String> {
    let path = wide(path);
    // SAFETY: the version block is read into a buffer of the size Windows
    // reports, and the value pointer points into that buffer.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0_u8; size as usize];
        GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, block.as_mut_ptr().cast()).ok()?;

        // The first language and code page the file lists.
        let mut value: *mut c_void = std::ptr::null_mut();
        let mut len = 0_u32;
        let query = wide(r"\VarFileInfo\Translation");
        if !VerQueryValueW(block.as_ptr().cast(), PCWSTR(query.as_ptr()), &raw mut value, &raw mut len)
            .as_bool()
            || len < 4
        {
            return None;
        }
        let [language, code_page] = *value.cast::<[u16; 2]>();
        let query = wide(&format!(
            r"\StringFileInfo\{language:04x}{code_page:04x}\FileDescription"
        ));
        if !VerQueryValueW(block.as_ptr().cast(), PCWSTR(query.as_ptr()), &raw mut value, &raw mut len)
            .as_bool()
            || len == 0
        {
            return None;
        }
        let text = std::slice::from_raw_parts(value.cast::<u16>(), len as usize);
        let text = String::from_utf16_lossy(text);
        let text = text.trim_end_matches('\0').trim();
        (!text.is_empty()).then(|| text.to_owned())
    }
}

/// Whether `process` runs as administrator. A process Viary may not even
/// look at counts as one.
fn elevated(process: HANDLE) -> bool {
    let mut token = HANDLE::default();
    // SAFETY: the token is closed below; the out value is a local of the
    // size passed.
    unsafe {
        if let Err(error) = OpenProcessToken(process, TOKEN_QUERY, &raw mut token) {
            return error.code() == ERROR_ACCESS_DENIED.to_hresult();
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0_u32;
        let read = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &raw mut len,
        );
        let _ = CloseHandle(token);
        read.is_ok() && elevation.TokenIsElevated != 0
    }
}

/// Whether Windows keeps Viary's keystrokes from `target`: an app running
/// as administrator ignores input from one that is not (UIPI), without
/// telling the sender.
pub fn is_protected(target: &TargetApp) -> bool {
    if target.pid <= 0 || target.is_self() {
        return false;
    }
    // SAFETY: the pseudo handle needs no closing; the target's handle is
    // closed below.
    unsafe {
        if elevated(GetCurrentProcess()) {
            return false;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, target.pid.cast_unsigned()) else {
            return false;
        };
        let protected = elevated(process);
        let _ = CloseHandle(process);
        protected
    }
}

/// Brings `target`'s window back to the front, for example after a click
/// on the pill, and waits until it is. Returns whether it is.
pub fn activate(target: &TargetApp) -> bool {
    let window = target.hwnd();
    // SAFETY: plain Win32 calls on a window handle that may have closed;
    // IsWindow checks it first.
    unsafe {
        if GetForegroundWindow() == window {
            return true;
        }
        if !IsWindow(Some(window)).as_bool() {
            return false;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        // Windows lets only the foreground thread hand the foreground
        // over. Attaching to it borrows that right for a moment.
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let current = GetCurrentThreadId();
        let attached = foreground != 0
            && foreground != current
            && AttachThreadInput(current, foreground, true).as_bool();
        let _ = BringWindowToTop(window);
        let _ = SetForegroundWindow(window);
        if attached {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
    let deadline = Instant::now() + Duration::from_millis(600);
    while Instant::now() < deadline {
        // SAFETY: a plain query.
        if unsafe { GetForegroundWindow() } == window {
            // Give the window a moment to put its caret back.
            std::thread::sleep(Duration::from_millis(60));
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    false
}
