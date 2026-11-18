//! Small Win32 helpers that are not about displays.

#![allow(unsafe_code)]

use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

/// Connects this process to the console it was launched from, if any.
///
/// Deskflip is built as a windowed program so that starting it does not flash a console. That
/// also means its command-line output goes nowhere unless it attaches to the caller's console.
pub fn attach_parent_console() {
    // SAFETY: AttachConsole takes no pointers. Failure means there is no parent console (or
    // output is already redirected), and either way there is nothing to do.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}
