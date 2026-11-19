//! Small Win32 helpers that are not about displays: the console, and the named kernel objects
//! Deskflip's processes use to coordinate.

#![allow(unsafe_code)]

use std::io;
use std::marker::PhantomData;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, ReleaseMutex, SetEvent, WaitForSingleObject,
};
use windows::core::HSTRING;

/// Held by the tray process, so that only one runs.
pub const TRAY_MUTEX: &str = "Local\\Deskflip.Tray";
/// Held by the settings window, so that only one is open.
pub const SETTINGS_MUTEX: &str = "Local\\Deskflip.Settings";
/// Held while a layout is being applied, by whichever process is applying it.
pub const APPLY_MUTEX: &str = "Local\\Deskflip.Apply";
/// Signalled whenever the settings or the current layout change, to wake the tray process.
pub const CHANGED_EVENT: &str = "Local\\Deskflip.Changed";

/// Connects this process to the console it was launched from, if any.
///
/// Deskflip is built as a windowed program so that starting it does not flash a console. That
/// also means its command-line output goes nowhere unless it attaches to the caller's console.
pub fn attach_parent_console() {
    // SAFETY: AttachConsole takes no pointers. Failure means there is no parent console (or
    // output is already redirected), and either way there is nothing to do.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

/// Ownership of a system-wide named mutex, released when dropped.
///
/// Windows ties mutex ownership to the acquiring thread, so the guard must be dropped on the
/// thread that created it; it is deliberately not `Send`.
pub struct NamedMutex {
    handle: HANDLE,
    _not_send: PhantomData<*const ()>,
}

impl NamedMutex {
    /// Takes the mutex if no other thread or process holds it. Returns `None` if it is held
    /// elsewhere, or if the mutex could not be created at all.
    pub fn try_acquire(name: &str) -> Option<Self> {
        // SAFETY: the name is a valid NUL-terminated string for the duration of the call.
        let handle = unsafe { CreateMutexW(None, false, &HSTRING::from(name)) }.ok()?;
        // SAFETY: `handle` was just returned by CreateMutexW. A zero timeout makes this a
        // non-blocking attempt.
        let wait = unsafe { WaitForSingleObject(handle, 0) };
        // "Abandoned" means the previous owner exited without releasing; we own it now.
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            Some(Self {
                handle,
                _not_send: PhantomData,
            })
        } else {
            // SAFETY: `handle` is ours and is not used again.
            let _ = unsafe { CloseHandle(handle) };
            None
        }
    }
}

impl Drop for NamedMutex {
    fn drop(&mut self) {
        // SAFETY: this thread owns the mutex (see `try_acquire`; the type is not `Send`), and
        // the handle is closed exactly once, here.
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

/// A system-wide named event that resets itself after waking one waiter.
pub struct NamedEvent {
    handle: HANDLE,
}

impl NamedEvent {
    pub fn open_or_create(name: &str) -> io::Result<Self> {
        // SAFETY: the name is a valid NUL-terminated string for the duration of the call.
        let handle = unsafe { CreateEventW(None, false, false, &HSTRING::from(name)) }
            .map_err(io::Error::other)?;
        Ok(Self { handle })
    }

    pub fn signal(&self) {
        // SAFETY: `handle` is a live event handle. SetEvent fails only for an invalid handle.
        let _ = unsafe { SetEvent(self.handle) };
    }

    /// Waits up to `milliseconds` for a signal, consuming it. Returns whether one arrived.
    pub fn wait(&self, milliseconds: u32) -> bool {
        // SAFETY: `handle` is a live event handle.
        unsafe { WaitForSingleObject(self.handle, milliseconds) == WAIT_OBJECT_0 }
    }
}

impl Drop for NamedEvent {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and is closed exactly once, here.
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;

    #[test]
    fn second_acquire_from_another_thread_is_refused() {
        let name = format!("Local\\Deskflip.Test.Mutex.{}", std::process::id());
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        let holder = thread::spawn({
            let name = name.clone();
            move || {
                let guard = NamedMutex::try_acquire(&name);
                acquired_tx.send(guard.is_some()).unwrap();
                release_rx.recv().unwrap();
                drop(guard);
            }
        });

        assert!(acquired_rx.recv().unwrap());
        assert!(NamedMutex::try_acquire(&name).is_none());

        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert!(NamedMutex::try_acquire(&name).is_some());
    }

    #[test]
    fn event_stays_signalled_until_one_wait_consumes_it() {
        let name = format!("Local\\Deskflip.Test.Event.{}", std::process::id());
        let event = NamedEvent::open_or_create(&name).unwrap();
        let same_event = NamedEvent::open_or_create(&name).unwrap();

        assert!(!event.wait(0));
        same_event.signal();
        assert!(event.wait(0));
        assert!(!event.wait(0));
    }
}
