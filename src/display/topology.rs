//! Which monitors the Windows desktop uses, through the display-configuration (CCD) API.
//!
//! Windows describes the desktop as a list of *paths*, each joining a *source* (a region of the
//! desktop) to a *target* (a monitor connector), plus a table of *modes* the paths index into.
//! A source mode carries the region's size and position; the main display is the one at (0, 0).
//!
//! Deskflip never invents an arrangement. It either asks Windows to extend onto everything, in
//! the arrangement Windows itself remembers, or takes the current arrangement and drops paths.

#![allow(unsafe_code)]

use std::collections::{BTreeSet, HashMap};
use std::mem;

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_SOURCE_MODE,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes,
    QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS, QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig,
    SDC_ALLOW_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_TOPOLOGY_EXTEND,
    SDC_USE_SUPPLIED_DISPLAY_CONFIG, SET_DISPLAY_CONFIG_FLAGS, SetDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

use super::{ConnectedMonitor, DisplayError, MonitorId, Rect, Snapshot};

/// A desktop arrangement captured by [`save`], valid until the next reboot or driver restart.
pub struct SavedConfig {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
    modes: Vec<DISPLAYCONFIG_MODE_INFO>,
}

/// Apply exactly the supplied arrangement, remember it, and let Windows adjust details (such as
/// closing a gap left by a removed monitor) rather than reject it.
const APPLY_SUPPLIED: SET_DISPLAY_CONFIG_FLAGS = SET_DISPLAY_CONFIG_FLAGS(
    SDC_APPLY.0 | SDC_USE_SUPPLIED_DISPLAY_CONFIG.0 | SDC_SAVE_TO_DATABASE.0 | SDC_ALLOW_CHANGES.0,
);

pub fn snapshot() -> Result<Snapshot, DisplayError> {
    let (active_paths, modes) = query(QDC_ONLY_ACTIVE_PATHS)?;
    let mut monitors = Vec::new();

    for path in &active_paths {
        let Some((id, model)) = target_name(path) else {
            continue;
        };
        let source = source_mode(path, &modes);
        monitors.push(ConnectedMonitor {
            id,
            model,
            active: true,
            primary: source.is_some_and(|mode| mode.position.x == 0 && mode.position.y == 0),
            rect: source.map(|mode| Rect {
                x: mode.position.x,
                y: mode.position.y,
                width: mode.width,
                height: mode.height,
            }),
        });
    }

    // The full path list pairs every source with every target, so each connected monitor shows
    // up many times; anything not already listed as active is connected but unused.
    let (all_paths, _) = query(QDC_ALL_PATHS)?;
    for path in &all_paths {
        if !path.targetInfo.targetAvailable.as_bool() {
            continue;
        }
        let Some((id, model)) = target_name(path) else {
            continue;
        };
        if monitors.iter().all(|monitor| monitor.id != id) {
            monitors.push(ConnectedMonitor {
                id,
                model,
                active: false,
                primary: false,
                rect: None,
            });
        }
    }

    monitors.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Snapshot { monitors })
}

pub fn extend_all() -> Result<(), DisplayError> {
    // SAFETY: with no path or mode arrays, SetDisplayConfig reads nothing from us; it applies
    // the "extend" arrangement from Windows' own database.
    let code = unsafe {
        SetDisplayConfig(
            None,
            None,
            SET_DISPLAY_CONFIG_FLAGS(SDC_APPLY.0 | SDC_TOPOLOGY_EXTEND.0),
        )
    };
    check("SetDisplayConfig(extend)", code)
}

pub fn set_active(keep: &BTreeSet<MonitorId>, primary: &MonitorId) -> Result<(), DisplayError> {
    let (active_paths, modes) = query(QDC_ONLY_ACTIVE_PATHS)?;

    let mut kept: Vec<(MonitorId, DISPLAYCONFIG_PATH_INFO)> = Vec::new();
    for path in &active_paths {
        if let Some((id, _)) = target_name(path)
            && keep.contains(&id)
        {
            kept.push((id, *path));
        }
    }
    if keep
        .iter()
        .any(|id| kept.iter().all(|(kept_id, _)| kept_id != id))
    {
        return Err(DisplayError::NotActive);
    }

    // The main display is whichever sits at the origin, so shift everything to put it there.
    let origin = kept
        .iter()
        .find(|(id, _)| id == primary)
        .and_then(|(_, path)| source_mode(path, &modes))
        .map(|mode| mode.position)
        .ok_or(DisplayError::NotActive)?;

    // Rebuild the mode table with only the entries the kept paths use.
    let mut new_modes: Vec<DISPLAYCONFIG_MODE_INFO> = Vec::new();
    let mut remapped: HashMap<u32, u32> = HashMap::new();
    let mut remap = |old_index: u32, new_modes: &mut Vec<DISPLAYCONFIG_MODE_INFO>| -> Option<u32> {
        if let Some(&new_index) = remapped.get(&old_index) {
            return Some(new_index);
        }
        let mut mode = *modes.get(old_index as usize)?;
        if mode.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
            // SAFETY: `infoType` says the union holds a source mode.
            let source = unsafe { &mut mode.Anonymous.sourceMode };
            source.position.x -= origin.x;
            source.position.y -= origin.y;
        }
        let new_index = u32::try_from(new_modes.len()).ok()?;
        new_modes.push(mode);
        remapped.insert(old_index, new_index);
        Some(new_index)
    };

    let mut new_paths = Vec::with_capacity(kept.len());
    for (_, mut path) in kept {
        // SAFETY: without QDC_VIRTUAL_MODE_AWARE the union is the plain `modeInfoIdx` field.
        let (source_index, target_index) = unsafe {
            (
                path.sourceInfo.Anonymous.modeInfoIdx,
                path.targetInfo.Anonymous.modeInfoIdx,
            )
        };
        let invalid = DisplayError::Os {
            call: "QueryDisplayConfig(mode index)",
            code: 0,
        };
        let Some(new_source) = remap(source_index, &mut new_modes) else {
            return Err(invalid);
        };
        let Some(new_target) = remap(target_index, &mut new_modes) else {
            return Err(invalid);
        };
        path.sourceInfo.Anonymous.modeInfoIdx = new_source;
        path.targetInfo.Anonymous.modeInfoIdx = new_target;
        new_paths.push(path);
    }

    apply_supplied("SetDisplayConfig(subset)", &new_paths, &new_modes)
}

pub fn save() -> Result<SavedConfig, DisplayError> {
    let (paths, modes) = query(QDC_ONLY_ACTIVE_PATHS)?;
    Ok(SavedConfig { paths, modes })
}

pub fn restore(saved: &SavedConfig) -> Result<(), DisplayError> {
    apply_supplied("SetDisplayConfig(restore)", &saved.paths, &saved.modes)
}

/// The GDI name (`\\.\DISPLAY1`) of the desktop region shown on an active monitor.
pub fn gdi_name(monitor: &MonitorId) -> Result<String, DisplayError> {
    let (active_paths, _) = query(QDC_ONLY_ACTIVE_PATHS)?;
    for path in &active_paths {
        if target_name(path).is_some_and(|(id, _)| &id == monitor) {
            return source_name(path).ok_or(DisplayError::Os {
                call: "DisplayConfigGetDeviceInfo(source name)",
                code: 0,
            });
        }
    }
    let connected = snapshot()?.get(monitor).is_some();
    Err(if connected {
        DisplayError::NotActive
    } else {
        DisplayError::NotConnected
    })
}

fn apply_supplied(
    call: &'static str,
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Result<(), DisplayError> {
    // SAFETY: both slices are valid for the duration of the call and SetDisplayConfig only
    // reads them.
    let code = unsafe { SetDisplayConfig(Some(paths), Some(modes), APPLY_SUPPLIED) };
    check(call, code)
}

fn check(call: &'static str, code: i32) -> Result<(), DisplayError> {
    if code == 0 {
        Ok(())
    } else {
        Err(DisplayError::Os { call, code })
    }
}

fn query(
    flags: QUERY_DISPLAY_CONFIG_FLAGS,
) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), DisplayError> {
    // The required sizes can change between the two calls if a monitor is plugged in, so retry.
    for _ in 0..4 {
        let mut path_count = 0u32;
        let mut mode_count = 0u32;
        // SAFETY: both pointers refer to live locals.
        let sizes = unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) };
        if sizes != ERROR_SUCCESS {
            return Err(os_error("GetDisplayConfigBufferSizes", sizes.0));
        }

        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
        // SAFETY: the buffers hold `path_count` and `mode_count` elements, as the counts passed
        // alongside them say; Windows writes at most that many and updates the counts.
        let result = unsafe {
            QueryDisplayConfig(
                flags,
                &mut path_count,
                paths.as_mut_ptr(),
                &mut mode_count,
                modes.as_mut_ptr(),
                None,
            )
        };
        if result == ERROR_SUCCESS {
            paths.truncate(path_count as usize);
            modes.truncate(mode_count as usize);
            return Ok((paths, modes));
        }
        if result != ERROR_INSUFFICIENT_BUFFER {
            return Err(os_error("QueryDisplayConfig", result.0));
        }
    }
    Err(os_error("QueryDisplayConfig", ERROR_INSUFFICIENT_BUFFER.0))
}

fn os_error(call: &'static str, code: u32) -> DisplayError {
    DisplayError::Os {
        call,
        code: i32::try_from(code).unwrap_or(i32::MAX),
    }
}

fn source_mode(
    path: &DISPLAYCONFIG_PATH_INFO,
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Option<DISPLAYCONFIG_SOURCE_MODE> {
    // SAFETY: without QDC_VIRTUAL_MODE_AWARE the union is the plain `modeInfoIdx` field.
    let index = unsafe { path.sourceInfo.Anonymous.modeInfoIdx };
    let mode = modes.get(index as usize)?;
    if mode.infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE {
        return None;
    }
    // SAFETY: `infoType` says the union holds a source mode.
    Some(unsafe { mode.Anonymous.sourceMode })
}

/// The device path and model name of the monitor at the end of a path.
fn target_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<(MonitorId, String)> {
    let mut request = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id,
        },
        ..Default::default()
    };
    // SAFETY: `header.size` matches the struct the header sits at the start of, so Windows
    // writes only within `request`.
    let code = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) };
    if code != 0 {
        return None;
    }
    let device_path = wide_to_string(&request.monitorDevicePath);
    if device_path.is_empty() {
        return None;
    }
    Some((
        MonitorId::new(device_path),
        wide_to_string(&request.monitorFriendlyDeviceName),
    ))
}

fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Option<String> {
    let mut request = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };
    // SAFETY: as in `target_name`, `header.size` bounds what Windows writes.
    let code = unsafe { DisplayConfigGetDeviceInfo(&mut request.header) };
    (code == 0).then(|| wide_to_string(&request.viewGdiDeviceName))
}

/// Converts a NUL-terminated UTF-16 buffer to a `String`.
pub(super) fn wide_to_string(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_string_stops_at_nul() {
        let buffer: Vec<u16> = "DELL\0junk".encode_utf16().collect();
        assert_eq!(wide_to_string(&buffer), "DELL");
    }

    #[test]
    fn wide_string_without_nul_uses_whole_buffer() {
        let buffer: Vec<u16> = "AB".encode_utf16().collect();
        assert_eq!(wide_to_string(&buffer), "AB");
    }
}
