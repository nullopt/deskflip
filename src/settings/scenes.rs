//! Made-up window states: what the headless renderer draws and what the view tests press.
//! None of them reads the settings file or talks to a monitor.

use super::model::Model;
use super::{Page, Settings};
use crate::config::{Config, Layout, LayoutMonitor, MonitorConfig, Role, VERSION};
use crate::display::{ConnectedMonitor, MonitorId, Rect, Snapshot};
use crate::input::InputCode;

const DISPLAY_PORT: InputCode = InputCode(0x0F);
const HDMI_1: InputCode = InputCode(0x11);
const HDMI_2: InputCode = InputCode(0x12);

/// A scene's name and how it rewrites the state.
pub type Scene = (&'static str, fn(&mut Settings));

/// Scene names and how each rewrites the state, in the form the app specification takes.
pub const SCENES: &[Scene] = &[
    ("first-run", |s| *s = first_run()),
    ("desk", |s| *s = desk()),
    ("desk-monitors", |s| *s = on_page(desk(), Page::Monitors)),
    ("desk-general", |s| *s = on_page(desk(), Page::General)),
    ("unconfirmed", |s| *s = unconfirmed()),
    ("unsaved", |s| *s = unsaved()),
    ("unsaved-problem", |s| *s = unsaved_problem()),
    ("close-prompt", |s| *s = close_prompt()),
    ("no-monitors", |s| *s = no_monitors()),
    ("load-error", |s| *s = load_error()),
];

fn left() -> MonitorId {
    MonitorId::new("left")
}

fn right() -> MonitorId {
    MonitorId::new("right")
}

fn monitor(id: MonitorId, label: &str, confirmed: bool) -> MonitorConfig {
    MonitorConfig {
        id,
        model: "DELL G2725D".to_owned(),
        label: label.to_owned(),
        pc_input: Some(DISPLAY_PORT),
        pc_input_confirmed: confirmed,
        inputs: vec![HDMI_1, HDMI_2, DISPLAY_PORT],
    }
}

fn layout(name: &str, hotkey: &str, left_role: Role) -> Layout {
    Layout {
        name: name.to_owned(),
        hotkey: Some(hotkey.to_owned()),
        primary: Some(right()),
        monitors: vec![
            LayoutMonitor {
                id: left(),
                role: left_role,
            },
            LayoutMonitor {
                id: right(),
                role: Role::Pc,
            },
        ],
    }
}

fn desk_config(left_confirmed: bool) -> Config {
    Config {
        version: VERSION,
        cycle_hotkey: Some("Ctrl+Alt+M".to_owned()),
        monitors: vec![
            monitor(left(), "Left", left_confirmed),
            monitor(right(), "Right", true),
        ],
        layouts: vec![
            layout("Home PC", "Ctrl+Alt+1", Role::Pc),
            layout("Work laptop", "Ctrl+Alt+2", Role::Other { input: HDMI_2 }),
        ],
    }
}

/// Two 2560x1440 monitors side by side, both on this PC, the right one primary.
fn desk_snapshot() -> Snapshot {
    let connected = |id, x, primary| ConnectedMonitor {
        id,
        model: "DELL G2725D".to_owned(),
        active: true,
        primary,
        rect: Some(Rect {
            x,
            y: 0,
            width: 2560,
            height: 1440,
        }),
    };
    Snapshot {
        monitors: vec![connected(left(), -2560, false), connected(right(), 0, true)],
    }
}

fn on_page(mut settings: Settings, page: Page) -> Settings {
    settings.go_to(page);
    settings
}

/// The owner's desk, saved, showing the layout that hands the left monitor to a laptop.
pub fn desk() -> Settings {
    let mut settings = Settings::new(Model::new(Some(desk_config(true))), None);
    settings.snapshot = desk_snapshot();
    on_page(settings, Page::Layout(1))
}

/// Nothing saved yet: monitors just detected and the starter layout offered.
pub fn first_run() -> Settings {
    let mut model = Model::new(None);
    model.merge(
        &desk_config(true)
            .monitors
            .iter()
            .map(|monitor| crate::config::Detected {
                id: monitor.id.clone(),
                model: monitor.model.clone(),
                current_input: monitor.pc_input,
                inputs: monitor.inputs.clone(),
            })
            .collect::<Vec<_>>(),
    );
    let mut settings = Settings::new(model, None);
    settings.snapshot = desk_snapshot();
    on_page(settings, Page::Monitors)
}

/// The left monitor's PC input has not been confirmed.
pub fn unconfirmed() -> Settings {
    let mut settings = Settings::new(Model::new(Some(desk_config(false))), None);
    settings.snapshot = desk_snapshot();
    on_page(settings, Page::Monitors)
}

/// A change that can be saved.
pub fn unsaved() -> Settings {
    let mut settings = desk();
    settings.model.draft.monitors[0].label = "Left screen".to_owned();
    settings.sync_fields();
    settings
}

/// A change that cannot be saved: two layouts with one name.
pub fn unsaved_problem() -> Settings {
    let mut settings = desk();
    settings.model.draft.layouts[1].name = "Home PC".to_owned();
    settings.sync_fields();
    settings
}

/// The window was closed with unsaved changes.
pub fn close_prompt() -> Settings {
    let mut settings = unsaved();
    settings.close_prompt = true;
    settings
}

/// Saved settings that know no monitors, and none connected.
pub fn no_monitors() -> Settings {
    let empty = Config {
        version: VERSION,
        cycle_hotkey: None,
        monitors: Vec::new(),
        layouts: Vec::new(),
    };
    Settings::new(Model::new(Some(empty)), None)
}

/// The settings file exists but cannot be read.
pub fn load_error() -> Settings {
    Settings::new(
        Model::new(None),
        Some("TOML parse error at line 3, column 9: invalid type".to_owned()),
    )
}
