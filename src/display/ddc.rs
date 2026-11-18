//! Commands sent to a monitor over DDC/CI, the control channel that runs alongside the picture.
//!
//! Windows only hands out a DDC/CI handle for a monitor that is part of the desktop, and the
//! lookup fails intermittently on real hardware, so every operation here retries.

#![allow(unsafe_code)]

use std::mem;
use std::thread;
use std::time::Duration;

use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitors, GetCapabilitiesStringLength,
    GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR,
    GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR, SetVCPFeature,
};
use windows::Win32::Foundation::{HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::core::BOOL;

use super::DisplayError;
use super::topology::wide_to_string;
use crate::input::{INPUT_SOURCE_FEATURE, InputCode};

/// Switching inputs is the operation users wait on, so it gets the full budget: 5 seconds.
const WRITE_ATTEMPTS: u32 = 10;
/// Reads only inform the settings window; a shorter wait keeps it responsive.
const READ_ATTEMPTS: u32 = 3;
const PAUSE: Duration = Duration::from_millis(500);

pub fn set_input(gdi_name: &str, input: InputCode) -> Result<(), DisplayError> {
    retry(WRITE_ATTEMPTS, || {
        with_monitor(gdi_name, |handle| {
            // SAFETY: `handle` is a live physical-monitor handle for the duration of the closure.
            let ok = unsafe { SetVCPFeature(handle, INPUT_SOURCE_FEATURE, u32::from(input.0)) };
            (ok != 0).then_some(())
        })
    })
}

/// The input the monitor claims to be showing.
///
/// Treat the answer as a hint: some monitors report the input the request arrived on rather
/// than the one on screen.
pub fn read_input(gdi_name: &str) -> Result<InputCode, DisplayError> {
    retry(READ_ATTEMPTS, || {
        with_monitor(gdi_name, |handle| {
            let mut current = 0u32;
            // SAFETY: `handle` is live and `current` outlives the call.
            let ok = unsafe {
                GetVCPFeatureAndVCPFeatureReply(
                    handle,
                    INPUT_SOURCE_FEATURE,
                    None,
                    &mut current,
                    None,
                )
            };
            // The value is 16 bits wide; the input code is its low byte.
            (ok != 0).then_some(InputCode((current & 0xFF) as u8))
        })
    })
}

/// The monitor's MCCS capabilities string. This is slow: monitors take a second or more.
pub fn read_capabilities(gdi_name: &str) -> Result<String, DisplayError> {
    retry(READ_ATTEMPTS, || {
        with_monitor(gdi_name, |handle| {
            let mut length = 0u32;
            // SAFETY: `handle` is live and `length` outlives the call.
            if unsafe { GetCapabilitiesStringLength(handle, &mut length) } == 0 || length == 0 {
                return None;
            }
            let mut buffer = vec![0u8; length as usize];
            // SAFETY: `handle` is live; the wrapper passes the buffer's own length, so the
            // monitor's reply cannot be written past its end.
            if unsafe { CapabilitiesRequestAndCapabilitiesReply(handle, &mut buffer) } == 0 {
                return None;
            }
            let end = buffer
                .iter()
                .position(|&byte| byte == 0)
                .unwrap_or(buffer.len());
            Some(String::from_utf8_lossy(&buffer[..end]).into_owned())
        })
    })
}

/// Runs `attempt` until it yields a value, pausing between tries.
fn retry<T>(attempts: u32, mut attempt: impl FnMut() -> Option<T>) -> Result<T, DisplayError> {
    for remaining in (0..attempts).rev() {
        if let Some(value) = attempt() {
            return Ok(value);
        }
        if remaining > 0 {
            thread::sleep(PAUSE);
        }
    }
    Err(DisplayError::DdcRejected)
}

/// Runs `f` with the DDC/CI handle of the monitor showing the desktop region `gdi_name`.
/// Returns `None` if no handle could be obtained or `f` itself returns `None`.
fn with_monitor<T>(gdi_name: &str, f: impl FnOnce(HANDLE) -> Option<T>) -> Option<T> {
    let monitor = find_hmonitor(gdi_name)?;
    let physical = PhysicalMonitors::open(monitor)?;
    // One desktop region shows on several physical monitors only when they are duplicated;
    // Deskflip does not manage duplicated displays, so the first is the one we mean.
    f(physical.0.first()?.hPhysicalMonitor)
}

/// Physical-monitor handles, destroyed when dropped.
struct PhysicalMonitors(Vec<PHYSICAL_MONITOR>);

impl PhysicalMonitors {
    fn open(monitor: HMONITOR) -> Option<Self> {
        let mut count = 0u32;
        // SAFETY: `count` outlives the call.
        unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(monitor, &mut count) }.ok()?;
        if count == 0 {
            return None;
        }
        let mut handles = vec![PHYSICAL_MONITOR::default(); count as usize];
        // SAFETY: the wrapper passes the slice's own length as the array size.
        unsafe { GetPhysicalMonitorsFromHMONITOR(monitor, &mut handles) }.ok()?;
        Some(Self(handles))
    }
}

impl Drop for PhysicalMonitors {
    fn drop(&mut self) {
        // SAFETY: these handles came from GetPhysicalMonitorsFromHMONITOR and are destroyed
        // exactly once, here. A failure leaves nothing to clean up.
        let _ = unsafe { DestroyPhysicalMonitors(&self.0) };
    }
}

fn find_hmonitor(gdi_name: &str) -> Option<HMONITOR> {
    unsafe extern "system" fn collect(
        monitor: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` is the address of the `Vec` passed to EnumDisplayMonitors below, which
        // is alive and not otherwise borrowed while the enumeration runs.
        let monitors = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        monitors.push(monitor);
        true.into()
    }

    let mut monitors: Vec<HMONITOR> = Vec::new();
    // SAFETY: the callback only uses `data` as the `Vec` pointer passed here, and the
    // enumeration is finished when the call returns.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&raw mut monitors as isize),
        )
    };

    monitors.into_iter().find(|&monitor| {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `cbSize` tells Windows the pointer refers to the extended struct, which it is.
        let ok = unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast()) };
        ok.as_bool() && wide_to_string(&info.szDevice).eq_ignore_ascii_case(gdi_name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn retry_returns_first_success_without_further_attempts() {
        let calls = Cell::new(0);
        let result = retry(3, || {
            calls.set(calls.get() + 1);
            Some(7)
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retry_gives_up_after_all_attempts() {
        let calls = Cell::new(0);
        let result: Result<(), _> = retry(2, || {
            calls.set(calls.get() + 1);
            None
        });
        assert!(matches!(result, Err(DisplayError::DdcRejected)));
        assert_eq!(calls.get(), 2);
    }
}
