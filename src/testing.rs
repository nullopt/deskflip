//! Fixtures shared by unit tests.

use std::fs;
use std::path::PathBuf;

use crate::config::{Config, Layout, LayoutMonitor, MonitorConfig, Role, VERSION};
use crate::display::MonitorId;
use crate::input::InputCode;

pub const DP: InputCode = InputCode(0x0F);
pub const HDMI1: InputCode = InputCode(0x11);
pub const HDMI2: InputCode = InputCode(0x12);

pub fn id(name: &str) -> MonitorId {
    MonitorId::new(name)
}

/// A fresh, empty directory unique to this test.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("deskflip-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A monitor on DisplayPort with the Dell's three inputs.
pub fn monitor(name: &str, confirmed: bool) -> MonitorConfig {
    MonitorConfig {
        id: id(name),
        model: "TEST".to_owned(),
        label: name.to_owned(),
        pc_input: Some(DP),
        pc_input_confirmed: confirmed,
        inputs: vec![HDMI1, HDMI2, DP],
    }
}

pub fn layout(name: &str, hotkey: Option<&str>, monitors: &[(&str, Role)]) -> Layout {
    Layout {
        name: name.to_owned(),
        hotkey: hotkey.map(str::to_owned),
        primary: None,
        monitors: monitors
            .iter()
            .map(|(monitor, role)| LayoutMonitor {
                id: id(monitor),
                role: role.clone(),
            })
            .collect(),
    }
}

pub fn config(monitors: Vec<MonitorConfig>, layouts: Vec<Layout>) -> Config {
    Config {
        version: VERSION,
        cycle_hotkey: None,
        monitors,
        layouts,
    }
}

/// The owner's desk: `L` and `R`, both confirmed, with layouts `home` (both on this PC) and
/// `work` (`L` handed to HDMI 2).
pub fn desk() -> Config {
    config(
        vec![monitor("L", true), monitor("R", true)],
        vec![
            layout(
                "home",
                Some("Ctrl+Alt+1"),
                &[("L", Role::Pc), ("R", Role::Pc)],
            ),
            layout(
                "work",
                Some("Ctrl+Alt+2"),
                &[("L", Role::Other { input: HDMI2 }), ("R", Role::Pc)],
            ),
        ],
    )
}
