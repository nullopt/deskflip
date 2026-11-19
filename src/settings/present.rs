//! What the pages say and offer, decided apart from how it is drawn.

use rune::Shortcut;

use super::model::input_choices;
use crate::config::{Config, Layout, LayoutMonitor, Role};
use crate::display::{MonitorId, Rect, Snapshot};
use crate::hotkey::{self, HotkeyError};
use crate::input::{InputCode, input_name};
use crate::state::State;

/// How a monitor relates to this PC right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorStatus {
    InUse,
    ConnectedNotInUse,
    NotConnected,
}

impl MonitorStatus {
    pub fn text(self) -> &'static str {
        match self {
            Self::InUse => "in use by this PC",
            Self::ConnectedNotInUse => "connected, not in use by this PC right now",
            Self::NotConnected => "not connected",
        }
    }
}

pub fn monitor_status(snapshot: &Snapshot, id: &MonitorId) -> MonitorStatus {
    match snapshot.get(id) {
        Some(found) if found.active => MonitorStatus::InUse,
        Some(_) => MonitorStatus::ConnectedNotInUse,
        None => MonitorStatus::NotConnected,
    }
}

/// What a monitor shows in a layout, as the four choices the page offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoleChoice {
    ThisPc,
    AnotherDevice,
    Off,
    DontChange,
}

impl RoleChoice {
    pub const ALL: [RoleChoice; 4] = [
        Self::ThisPc,
        Self::AnotherDevice,
        Self::Off,
        Self::DontChange,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ThisPc => "This PC",
            Self::AnotherDevice => "Another device",
            Self::Off => "Off",
            Self::DontChange => "Don't change",
        }
    }
}

pub fn role_choice(layout: &Layout, id: &MonitorId) -> RoleChoice {
    match layout.monitors.iter().find(|entry| &entry.id == id) {
        Some(entry) => match entry.role {
            Role::Pc => RoleChoice::ThisPc,
            Role::Other { .. } => RoleChoice::AnotherDevice,
            Role::Off => RoleChoice::Off,
        },
        None => RoleChoice::DontChange,
    }
}

/// Why this monitor cannot be handed to another device, if it cannot.
pub fn another_device_blocked(config: &Config, id: &MonitorId) -> Option<&'static str> {
    let Some(monitor) = config.monitor(id) else {
        return Some("Deskflip does not know this monitor any more.");
    };
    if !monitor.pc_input_confirmed {
        return Some(
            "To hand this monitor to another device, first confirm on the Monitors page which \
             input this PC is plugged into.",
        );
    }
    if other_inputs(config, id).is_empty() {
        return Some("This monitor has no other input to switch to.");
    }
    None
}

/// The inputs of a monitor that this PC is not plugged into.
pub fn other_inputs(config: &Config, id: &MonitorId) -> Vec<InputCode> {
    config.monitor(id).map_or_else(Vec::new, |monitor| {
        input_choices(monitor)
            .into_iter()
            .filter(|&input| Some(input) != monitor.pc_input)
            .collect()
    })
}

/// Gives a monitor a role in a layout. A choice the monitor cannot take changes nothing.
pub fn set_role(config: &mut Config, layout: usize, id: &MonitorId, choice: RoleChoice) {
    if config.monitor(id).is_none() {
        return;
    }
    let role = match choice {
        RoleChoice::ThisPc => Some(Role::Pc),
        RoleChoice::Off => Some(Role::Off),
        RoleChoice::DontChange => None,
        RoleChoice::AnotherDevice => {
            if another_device_blocked(config, id).is_some() {
                return;
            }
            let Some(&first) = other_inputs(config, id).first() else {
                return;
            };
            Some(Role::Other { input: first })
        }
    };
    let Some(layout) = config.layouts.get_mut(layout) else {
        return;
    };
    let entry = layout.monitors.iter().position(|entry| &entry.id == id);
    match (entry, role) {
        // Choosing "Another device" again must not reset the input already picked.
        (Some(index), Some(Role::Other { .. }))
            if matches!(layout.monitors[index].role, Role::Other { .. }) => {}
        (Some(index), Some(role)) => layout.monitors[index].role = role,
        (Some(index), None) => {
            layout.monitors.remove(index);
        }
        (None, Some(role)) => layout.monitors.push(LayoutMonitor {
            id: id.clone(),
            role,
        }),
        (None, None) => {}
    }
    if choice != RoleChoice::ThisPc && layout.primary.as_ref() == Some(id) {
        layout.primary = None;
    }
}

/// One sentence saying what a layout does, in the user's own monitor names.
pub fn layout_summary(config: &Config, layout: &Layout) -> String {
    let mut kept = Vec::new();
    let mut handed = Vec::new();
    let mut off = Vec::new();
    let mut unchanged = Vec::new();
    for monitor in &config.monitors {
        let name = monitor.label.as_str();
        match layout.monitors.iter().find(|entry| entry.id == monitor.id) {
            Some(entry) => match entry.role {
                Role::Pc => kept.push(name),
                Role::Other { input } => {
                    handed.push(format!("{name} goes to {}", input_name(input)));
                }
                Role::Off => off.push(name),
            },
            None => unchanged.push(name),
        }
    }

    let mut clauses = handed.clone();
    if !kept.is_empty() {
        let verb = if handed.is_empty() { "uses" } else { "keeps" };
        clauses.push(format!("this PC {verb} {}", list(&kept)));
    }
    if !off.is_empty() {
        let verb = if off.len() == 1 { "is" } else { "are" };
        clauses.push(format!("{} {verb} turned off for this PC", list(&off)));
    }
    match unchanged.as_slice() {
        [] => {}
        [only] => clauses.push(format!("{only} is left as it is")),
        several => clauses.push(format!("{} are left as they are", list(several))),
    }
    if clauses.is_empty() {
        return "This layout has no monitors yet.".to_owned();
    }
    let sentence = clauses.join("; ");
    let mut letters = sentence.chars();
    let capitalised: String = letters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(letters)
        .collect();
    format!("{capitalised}.")
}

/// "A", "A and B", "A, B and C".
fn list(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Where a monitor is drawn in the desk picture, in points from the picture's top left.
#[derive(Clone, Debug, PartialEq)]
pub struct DeskRect {
    pub id: MonitorId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Lays the known monitors out as they sit on the desk, scaled to fit `area`. A monitor
/// Windows gives no position for (not in use, or not connected) is drawn where it was last
/// seen; one never seen on the desktop stands in a row to the right.
pub fn desk_rects(
    snapshot: &Snapshot,
    remembered: &State,
    config: &Config,
    area: (f32, f32),
) -> Vec<DeskRect> {
    const INSET: f32 = 16.0;
    const GAP: f32 = 12.0;
    /// The shape drawn for a monitor when no monitor's real size is known.
    const FALLBACK: (f32, f32) = (2560.0, 1440.0);

    let rect_of = |id: &MonitorId| {
        snapshot
            .get(id)
            .and_then(|found| found.rect)
            .or_else(|| remembered.position(id))
    };
    let placed: Vec<(&MonitorId, Rect)> = config
        .monitors
        .iter()
        .filter_map(|monitor| Some((&monitor.id, rect_of(&monitor.id)?)))
        .collect();
    let unplaced: Vec<&MonitorId> = config
        .monitors
        .iter()
        .map(|monitor| &monitor.id)
        .filter(|id| rect_of(id).is_none())
        .collect();

    // Measured in desktop pixels first, then scaled into the picture.
    let left = placed.iter().map(|(_, rect)| rect.x).min().unwrap_or(0) as f32;
    let top = placed.iter().map(|(_, rect)| rect.y).min().unwrap_or(0) as f32;
    let desk_width = placed
        .iter()
        .map(|(_, rect)| rect.x as f32 + rect.width as f32 - left)
        .fold(0.0, f32::max);
    let desk_height = placed
        .iter()
        .map(|(_, rect)| rect.y as f32 + rect.height as f32 - top)
        .fold(0.0, f32::max);
    let stand_in = placed.first().map_or(FALLBACK, |(_, rect)| {
        (rect.width as f32, rect.height as f32)
    });

    // Stand-ins line up to the right, each one gap away from what is before it. The gap is
    // fixed in points, so it is left out of the scaled width.
    let gaps = if placed.is_empty() {
        unplaced.len().saturating_sub(1)
    } else {
        unplaced.len()
    } as f32;
    let width = (desk_width + unplaced.len() as f32 * stand_in.0).max(1.0);
    let height = desk_height.max(stand_in.1);
    let scale = ((area.0 - 2.0 * INSET - gaps * GAP) / width)
        .min((area.1 - 2.0 * INSET) / height)
        .max(0.0);
    let origin_x = (area.0 - (width * scale + gaps * GAP)) / 2.0;
    let origin_y = (area.1 - height * scale) / 2.0;

    let mut rects: Vec<DeskRect> = placed
        .iter()
        .map(|(id, rect)| DeskRect {
            id: (*id).clone(),
            x: origin_x + (rect.x as f32 - left) * scale,
            y: origin_y + (rect.y as f32 - top) * scale,
            w: rect.width as f32 * scale,
            h: rect.height as f32 * scale,
        })
        .collect();
    let first_gap = if placed.is_empty() { 0.0 } else { GAP };
    for (index, id) in unplaced.into_iter().enumerate() {
        let index = index as f32;
        rects.push(DeskRect {
            id: id.clone(),
            x: origin_x + (desk_width + index * stand_in.0) * scale + first_gap + index * GAP,
            y: origin_y,
            w: stand_in.0 * scale,
            h: stand_in.1 * scale,
        });
    }
    // In the order the monitors are numbered.
    rects.sort_by_key(|rect| {
        config
            .monitors
            .iter()
            .position(|monitor| monitor.id == rect.id)
    });
    rects
}

/// The form a recorded hotkey is stored in ("Ctrl+Alt+2"), or why it cannot be used.
pub fn stored_hotkey(shortcut: &Shortcut) -> Result<String, HotkeyError> {
    // The field and the tray name keys differently; the tray's reading is the one that counts.
    hotkey::parse(&shortcut.to_string()).map(|parsed| hotkey::format(&parsed))
}

/// A stored hotkey as the hotkey field's value.
pub fn shortcut_of(stored: &str) -> Option<Shortcut> {
    stored.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MonitorConfig;
    use crate::display::ConnectedMonitor;
    use crate::testing::{DP, HDMI1, HDMI2, config, desk, id, layout, monitor, snap};

    fn named(name: &str, label: &str) -> MonitorConfig {
        MonitorConfig {
            label: label.to_owned(),
            ..monitor(name, true)
        }
    }

    fn desk_named() -> Config {
        let mut config = desk();
        config.monitors = vec![named("L", "Left"), named("R", "Right")];
        config
    }

    #[test]
    fn summary_names_handover_and_what_this_pc_keeps() {
        let config = desk_named();
        assert_eq!(
            layout_summary(&config, &config.layouts[1]),
            "Left goes to HDMI 2; this PC keeps Right."
        );
    }

    #[test]
    fn summary_for_all_pc() {
        let config = desk_named();
        assert_eq!(
            layout_summary(&config, &config.layouts[0]),
            "This PC uses Left and Right."
        );
    }

    #[test]
    fn summary_mentions_off_and_unchanged() {
        let mut config = desk_named();
        config.layouts = vec![layout("x", None, &[("L", Role::Off)])];
        assert_eq!(
            layout_summary(&config, &config.layouts[0]),
            "Left is turned off for this PC; Right is left as it is."
        );
    }

    #[test]
    fn summary_lists_three_monitors_with_commas() {
        let mut config = desk_named();
        config.monitors.push(named("T", "Top"));
        config.layouts = vec![layout(
            "x",
            None,
            &[("L", Role::Pc), ("R", Role::Pc), ("T", Role::Pc)],
        )];
        assert_eq!(
            layout_summary(&config, &config.layouts[0]),
            "This PC uses Left, Right and Top."
        );
    }

    #[test]
    fn summary_of_a_layout_that_changes_nothing() {
        let mut config = desk_named();
        config.layouts = vec![layout("x", None, &[])];
        assert_eq!(
            layout_summary(&config, &config.layouts[0]),
            "Left and Right are left as they are."
        );
    }

    #[test]
    fn monitor_status_in_use() {
        let snapshot = snap(&[("L", true, true)]);
        assert_eq!(monitor_status(&snapshot, &id("L")), MonitorStatus::InUse);
    }

    #[test]
    fn monitor_status_connected_not_in_use() {
        let snapshot = snap(&[("L", false, false)]);
        assert_eq!(
            monitor_status(&snapshot, &id("L")),
            MonitorStatus::ConnectedNotInUse
        );
    }

    #[test]
    fn monitor_status_not_connected() {
        let snapshot = snap(&[("L", true, true)]);
        assert_eq!(
            monitor_status(&snapshot, &id("R")),
            MonitorStatus::NotConnected
        );
    }

    #[test]
    fn role_choice_follows_the_layout() {
        let config = desk();
        assert_eq!(
            role_choice(&config.layouts[1], &id("L")),
            RoleChoice::AnotherDevice
        );
        assert_eq!(
            role_choice(&config.layouts[1], &id("R")),
            RoleChoice::ThisPc
        );
    }

    #[test]
    fn role_choice_of_unlisted_monitor_is_dont_change() {
        let config = desk();
        assert_eq!(
            role_choice(&config.layouts[0], &id("elsewhere")),
            RoleChoice::DontChange
        );
    }

    #[test]
    fn another_device_blocked_until_confirmed() {
        let config = config(vec![monitor("L", false), monitor("R", true)], Vec::new());
        assert!(another_device_blocked(&config, &id("L")).is_some());
        assert!(another_device_blocked(&config, &id("R")).is_none());
    }

    #[test]
    fn another_device_blocked_without_another_input() {
        let mut only_dp = monitor("L", true);
        only_dp.inputs = vec![DP];
        let config = config(vec![only_dp], Vec::new());
        assert!(another_device_blocked(&config, &id("L")).is_some());
    }

    #[test]
    fn role_options_for_unknown_monitor() {
        let mut config = desk();
        let before = config.clone();
        assert!(another_device_blocked(&config, &id("gone")).is_some());
        set_role(&mut config, 0, &id("gone"), RoleChoice::AnotherDevice);
        assert_eq!(config, before);
    }

    #[test]
    fn set_role_ignores_a_layout_that_does_not_exist() {
        let mut config = desk();
        let before = config.clone();
        set_role(&mut config, 9, &id("L"), RoleChoice::Off);
        assert_eq!(config, before);
    }

    #[test]
    fn set_role_another_device_picks_first_non_pc_input() {
        let mut config = desk();
        set_role(&mut config, 0, &id("L"), RoleChoice::AnotherDevice);
        assert_eq!(
            config.layouts[0].monitors[0].role,
            Role::Other { input: HDMI1 }
        );
    }

    #[test]
    fn set_role_another_device_keeps_an_input_already_chosen() {
        let mut config = desk();
        set_role(&mut config, 1, &id("L"), RoleChoice::AnotherDevice);
        assert_eq!(
            config.layouts[1].monitors[0].role,
            Role::Other { input: HDMI2 }
        );
    }

    #[test]
    fn set_role_dont_change_removes_the_entry_and_this_pc_adds_it_back() {
        let mut config = desk();
        set_role(&mut config, 0, &id("L"), RoleChoice::DontChange);
        assert_eq!(
            role_choice(&config.layouts[0], &id("L")),
            RoleChoice::DontChange
        );
        assert_eq!(config.layouts[0].monitors.len(), 1);
        set_role(&mut config, 0, &id("L"), RoleChoice::ThisPc);
        assert_eq!(
            role_choice(&config.layouts[0], &id("L")),
            RoleChoice::ThisPc
        );
    }

    #[test]
    fn set_role_away_from_pc_clears_primary() {
        let mut config = desk();
        config.layouts[0].primary = Some(id("L"));
        set_role(&mut config, 0, &id("L"), RoleChoice::Off);
        assert_eq!(config.layouts[0].primary, None);
    }

    fn placed(name: &str, x: i32) -> ConnectedMonitor {
        ConnectedMonitor {
            id: id(name),
            model: "TEST".to_owned(),
            active: true,
            primary: x == 0,
            rect: Some(Rect {
                x,
                y: 0,
                width: 2560,
                height: 1440,
            }),
        }
    }

    #[test]
    fn desk_rects_keep_relative_positions_and_fit() {
        let snapshot = Snapshot {
            monitors: vec![placed("L", -2560), placed("R", 0)],
        };
        let rects = desk_rects(&snapshot, &State::default(), &desk(), (400.0, 200.0));

        assert_eq!(rects.len(), 2);
        let (left, right) = (&rects[0], &rects[1]);
        assert_eq!(left.id, id("L"));
        assert!((left.w - right.w).abs() < 0.01 && (left.h - right.h).abs() < 0.01);
        assert!((left.w / left.h - 2560.0 / 1440.0).abs() < 0.01);
        assert!((right.x - (left.x + left.w)).abs() < 0.01);
        for rect in &rects {
            assert!(rect.x >= 0.0 && rect.y >= 0.0);
            assert!(rect.x + rect.w <= 400.0 && rect.y + rect.h <= 200.0);
        }
        // Centred: the same margin on both sides.
        assert!((left.x - (400.0 - (right.x + right.w))).abs() < 0.01);
    }

    #[test]
    fn desk_rects_place_inactive_monitors_beside_the_desk() {
        let snapshot = Snapshot {
            monitors: vec![placed("R", 0)],
        };
        let rects = desk_rects(&snapshot, &State::default(), &desk(), (400.0, 200.0));

        assert_eq!(rects.len(), 2);
        let left = rects.iter().find(|rect| rect.id == id("L")).unwrap();
        let right = rects.iter().find(|rect| rect.id == id("R")).unwrap();
        assert!(left.x >= right.x + right.w);
        assert!((left.w - right.w).abs() < 0.01);
        assert!(left.x + left.w <= 400.0);
    }

    #[test]
    fn desk_rects_put_a_monitor_not_in_use_where_it_was_last_seen() {
        let mut remembered = State::default();
        remembered.remember(&Snapshot {
            monitors: vec![placed("L", -2560), placed("R", 0)],
        });
        let now = Snapshot {
            monitors: vec![placed("R", 0)],
        };
        let rects = desk_rects(&now, &remembered, &desk(), (400.0, 200.0));

        let left = rects.iter().find(|rect| rect.id == id("L")).unwrap();
        let right = rects.iter().find(|rect| rect.id == id("R")).unwrap();
        assert!(
            (left.x + left.w - right.x).abs() < 0.01,
            "left sits left of right"
        );
    }

    #[test]
    fn desk_rects_without_any_position_still_show_every_monitor() {
        let rects = desk_rects(
            &Snapshot::default(),
            &State::default(),
            &desk(),
            (400.0, 200.0),
        );
        assert_eq!(rects.len(), 2);
        assert!(rects.iter().all(|rect| rect.w > 0.0 && rect.h > 0.0));
        assert!(rects[1].x >= rects[0].x + rects[0].w);
    }

    #[test]
    fn stored_hotkey_round_trips() {
        for stored in ["Ctrl+Alt+2", "Ctrl+Alt+M", "Ctrl+Shift+F5"] {
            let shortcut = shortcut_of(stored).unwrap_or_else(|| panic!("{stored} did not parse"));
            assert_eq!(stored_hotkey(&shortcut).as_deref(), Ok(stored));
        }
    }

    #[test]
    fn stored_hotkey_rejects_shift_only() {
        let shortcut = shortcut_of("Shift+A").expect("the field itself accepts Shift+A");
        assert_eq!(stored_hotkey(&shortcut), Err(HotkeyError::NeedsModifier));
    }

    #[test]
    fn shortcut_of_nonsense_is_none() {
        assert!(shortcut_of("not a hotkey").is_none());
    }
}
