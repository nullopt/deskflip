//! The monitors connected to this PC, and the operations Deskflip performs on them.
//!
//! Everything above this module works in terms of [`Snapshot`] and the [`Display`] trait, so the
//! decision logic can be tested without touching real hardware.

mod ddc;
mod topology;
mod windows;

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};

use crate::input::InputCode;

pub use windows::WindowsDisplay;

/// Identifies a physical monitor by its Windows device path, for example
/// `\\?\DISPLAY#DEL2014#5&e6a3024&0&UID4353#{e6f07b5f-...}`.
///
/// The path is tied to the graphics-card connector, so two identical monitors get different
/// ids, and an id stays the same across reboots. Windows is not consistent about the letter
/// case of these paths, so ids compare case-insensitively.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorId(String);

impl MonitorId {
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn folded(&self) -> impl Iterator<Item = u8> + '_ {
        self.0.bytes().map(|byte| byte.to_ascii_lowercase())
    }
}

impl PartialEq for MonitorId {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for MonitorId {}

impl Hash for MonitorId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for byte in self.folded() {
            state.write_u8(byte);
        }
        state.write_u8(0xFF);
    }
}

impl Ord for MonitorId {
    fn cmp(&self, other: &Self) -> Ordering {
        self.folded().cmp(other.folded())
    }
}

impl PartialOrd for MonitorId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A monitor's place on the Windows desktop, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// A monitor that is plugged in, whether or not the desktop currently uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectedMonitor {
    pub id: MonitorId,
    /// The model name the monitor reports, e.g. "DELL G2725D".
    pub model: String,
    /// Whether the Windows desktop extends onto this monitor.
    pub active: bool,
    /// Whether this is the main display. Only an active monitor can be.
    pub primary: bool,
    /// Known only while the monitor is active.
    pub rect: Option<Rect>,
}

/// The state of all connected monitors at one moment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub monitors: Vec<ConnectedMonitor>,
}

impl Snapshot {
    pub fn active_ids(&self) -> BTreeSet<MonitorId> {
        self.monitors
            .iter()
            .filter(|monitor| monitor.active)
            .map(|monitor| monitor.id.clone())
            .collect()
    }

    pub fn primary(&self) -> Option<&MonitorId> {
        self.monitors
            .iter()
            .find(|monitor| monitor.active && monitor.primary)
            .map(|monitor| &monitor.id)
    }

    pub fn get(&self, id: &MonitorId) -> Option<&ConnectedMonitor> {
        self.monitors.iter().find(|monitor| &monitor.id == id)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DisplayError {
    #[error("Windows display call {call} failed with code {code}")]
    Os { call: &'static str, code: i32 },
    #[error("monitor is not connected")]
    NotConnected,
    #[error("monitor is not on the desktop, so it cannot be sent commands")]
    NotActive,
    #[error("monitor did not accept the command (DDC/CI may be off or unsupported)")]
    DdcRejected,
}

/// What Deskflip needs from the display hardware.
pub trait Display {
    /// An opaque copy of the desktop arrangement, used to undo a failed change.
    type Saved;

    fn snapshot(&self) -> Result<Snapshot, DisplayError>;

    /// Extends the desktop onto every connected monitor, in the arrangement Windows remembers.
    fn extend_all(&mut self) -> Result<(), DisplayError>;

    /// Removes every monitor not in `keep` from the desktop and makes `primary` the main display.
    ///
    /// `keep` must be a non-empty subset of the currently active monitors and contain `primary`.
    /// This can only take monitors away; use [`Display::extend_all`] first to bring one back.
    fn set_active(
        &mut self,
        keep: &BTreeSet<MonitorId>,
        primary: &MonitorId,
    ) -> Result<(), DisplayError>;

    fn save(&self) -> Result<Self::Saved, DisplayError>;

    fn restore(&mut self, saved: &Self::Saved) -> Result<(), DisplayError>;

    /// Tells a monitor to show a different input. The monitor must be active.
    ///
    /// Success means the monitor acknowledged the command. Monitors cannot be relied on to
    /// report which input they are showing, so there is no way to confirm that it switched.
    fn set_input(&mut self, monitor: &MonitorId, input: InputCode) -> Result<(), DisplayError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeSet, HashSet};

    fn monitor(id: &str, active: bool, primary: bool) -> ConnectedMonitor {
        ConnectedMonitor {
            id: MonitorId::new(id),
            model: "TEST".to_owned(),
            active,
            primary,
            rect: None,
        }
    }

    #[test]
    fn monitor_id_ignores_ascii_case() {
        let upper = MonitorId::new(r"\\?\DISPLAY#DEL2014#A");
        let lower = MonitorId::new(r"\\?\display#del2014#a");

        assert_eq!(upper, lower);
        assert_eq!(BTreeSet::from([upper.clone(), lower.clone()]).len(), 1);
        assert_eq!(HashSet::from([upper, lower]).len(), 1);
    }

    #[test]
    fn monitor_id_keeps_its_original_spelling() {
        assert_eq!(MonitorId::new("AbC").as_str(), "AbC");
    }

    #[test]
    fn different_monitors_are_different() {
        assert_ne!(MonitorId::new("UID4353"), MonitorId::new("UID4355"));
    }

    #[test]
    fn snapshot_active_ids_and_primary() {
        let snapshot = Snapshot {
            monitors: vec![
                monitor("L", false, false),
                monitor("R", true, true),
                monitor("M", true, false),
            ],
        };

        assert_eq!(
            snapshot.active_ids(),
            BTreeSet::from([MonitorId::new("R"), MonitorId::new("M")])
        );
        assert_eq!(snapshot.primary(), Some(&MonitorId::new("R")));
        assert!(snapshot.get(&MonitorId::new("l")).is_some());
        assert!(snapshot.get(&MonitorId::new("X")).is_none());
    }

    #[test]
    fn snapshot_without_active_monitors_has_no_primary() {
        let snapshot = Snapshot {
            monitors: vec![monitor("L", false, false)],
        };

        assert_eq!(snapshot.primary(), None);
        assert!(snapshot.active_ids().is_empty());
    }

    #[test]
    fn monitor_id_serializes_as_a_string() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Wrapper {
            id: MonitorId,
        }
        let text = toml::to_string(&Wrapper {
            id: MonitorId::new(r"\\?\DISPLAY#X"),
        })
        .unwrap();
        let back: Wrapper = toml::from_str(&text).unwrap();
        assert_eq!(back.id.as_str(), r"\\?\DISPLAY#X");
    }
}
