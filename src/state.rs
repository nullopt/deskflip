//! Which layout is in effect.
//!
//! Monitors cannot be asked which input they are showing, so Deskflip remembers the last
//! layout it applied. That memory is only trusted while the desktop still looks the way that
//! layout left it: the user can change displays through Windows at any time.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Layout};
use crate::display::{MonitorId, Rect, Snapshot};
use crate::paths;
use crate::plan::plan;
use crate::validate::layout_is_usable;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied: Option<String>,
    /// Where each monitor was on the desktop when last seen there. A monitor handed to
    /// another device has no place on the desktop, but the settings window still draws it.
    #[serde(default, rename = "position", skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<Position>,
}

/// A monitor's last known place on the desktop, in pixels.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub id: MonitorId,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl State {
    /// Reads the state file. Anything wrong with it just means "nothing remembered".
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let text = toml::to_string(self).map_err(io::Error::other)?;
        paths::replace_file(path, &text)
    }
}

impl State {
    /// Notes where the monitors in `snapshot` are. Returns whether anything changed.
    pub fn remember(&mut self, snapshot: &Snapshot) -> bool {
        let on_desktop: Vec<Position> = snapshot
            .monitors
            .iter()
            .filter_map(|monitor| {
                let rect = monitor.rect?;
                Some(Position {
                    id: monitor.id.clone(),
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                })
            })
            .collect();
        let mut changed = false;

        // Windows measures positions from the main monitor. When another monitor becomes the
        // main one, every monitor on the desktop moves by the same amount, and the ones off
        // the desktop have to move with them to keep their place beside the others.
        if let Some((dx, dy)) = self.shift_of_origin(&on_desktop) {
            for known in &mut self.positions {
                if !on_desktop.iter().any(|seen| seen.id == known.id) {
                    known.x += dx;
                    known.y += dy;
                    changed = true;
                }
            }
        }

        for seen in on_desktop {
            match self.positions.iter_mut().find(|known| known.id == seen.id) {
                Some(known) if *known == seen => {}
                Some(known) => {
                    *known = seen;
                    changed = true;
                }
                None => {
                    self.positions.push(seen);
                    changed = true;
                }
            }
        }
        changed
    }

    /// How far the monitors remembered and still on the desktop have all moved together, if
    /// they have. Monitors that moved by different amounts were rearranged by the user, which
    /// says nothing about the others.
    fn shift_of_origin(&self, on_desktop: &[Position]) -> Option<(i32, i32)> {
        let mut shifts = on_desktop.iter().filter_map(|seen| {
            let known = self.positions.iter().find(|known| known.id == seen.id)?;
            Some((seen.x - known.x, seen.y - known.y))
        });
        let first = shifts.next()?;
        (first != (0, 0) && shifts.all(|shift| shift == first)).then_some(first)
    }

    /// Where a monitor was when last seen on the desktop.
    pub fn position(&self, id: &MonitorId) -> Option<Rect> {
        self.positions
            .iter()
            .find(|known| &known.id == id)
            .map(|known| Rect {
                x: known.x,
                y: known.y,
                width: known.width,
                height: known.height,
            })
    }
}

/// The remembered layout, if the desktop still uses the monitors that layout leaves it with.
pub fn current_layout<'a>(
    config: &'a Config,
    state: &State,
    snapshot: &Snapshot,
) -> Option<&'a Layout> {
    let layout = config.layout(state.last_applied.as_deref()?)?;
    let expected = plan(snapshot, config, layout).ok()?.expected_active;
    (expected == snapshot.active_ids()).then_some(layout)
}

/// The layout the cycle hotkey applies next: the usable layout after the remembered one,
/// wrapping around, or the first usable layout if nothing is remembered.
pub fn next_layout<'a>(config: &'a Config, state: &State) -> Option<&'a Layout> {
    let count = config.layouts.len();
    let start = state
        .last_applied
        .as_deref()
        .and_then(|name| {
            config
                .layouts
                .iter()
                .position(|layout| layout.name.eq_ignore_ascii_case(name))
        })
        .map_or(0, |index| index + 1);
    (0..count)
        .map(|offset| &config.layouts[(start + offset) % count])
        .find(|layout| layout_is_usable(config, &layout.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::{ConnectedMonitor, Rect};
    use crate::testing::{desk, id, snap, temp_dir};

    fn applied(name: &str) -> State {
        State {
            last_applied: Some(name.to_owned()),
            ..State::default()
        }
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
    fn remember_keeps_where_each_monitor_was_last_seen() {
        let mut state = State::default();
        let both = Snapshot {
            monitors: vec![placed("L", -2560), placed("R", 0)],
        };
        assert!(state.remember(&both));
        assert!(!state.remember(&both), "nothing new the second time");

        // The left monitor leaves the desktop; its place is still known.
        let mut only_right = both.clone();
        only_right.monitors[0].active = false;
        only_right.monitors[0].rect = None;
        assert!(!state.remember(&only_right));
        assert_eq!(state.position(&id("L")).map(|rect| rect.x), Some(-2560));
        assert_eq!(state.position(&id("elsewhere")), None);
    }

    #[test]
    fn remember_follows_a_monitor_that_moved() {
        let mut state = State::default();
        state.remember(&Snapshot {
            monitors: vec![placed("L", -2560)],
        });
        assert!(state.remember(&Snapshot {
            monitors: vec![placed("L", 2560)],
        }));
        assert_eq!(state.position(&id("L")).map(|rect| rect.x), Some(2560));
        assert_eq!(state.positions.len(), 1);
    }

    #[test]
    fn a_monitor_off_the_desktop_keeps_its_place_when_the_main_monitor_changes() {
        // Positions are measured from the main monitor: L, with R to its right.
        let mut state = State::default();
        state.remember(&Snapshot {
            monitors: vec![placed("L", 0), placed("R", 2560)],
        });

        // L is handed over, so R becomes the main monitor and is now at 0.
        assert!(state.remember(&Snapshot {
            monitors: vec![placed("R", 0)],
        }));

        assert_eq!(state.position(&id("R")).map(|rect| rect.x), Some(0));
        assert_eq!(state.position(&id("L")).map(|rect| rect.x), Some(-2560));
    }

    #[test]
    fn dragging_one_monitor_does_not_move_the_ones_off_the_desktop() {
        let mut state = State::default();
        state.remember(&Snapshot {
            monitors: vec![placed("A", 0), placed("B", 2560), placed("C", 5120)],
        });

        // C is off the desktop and B was dragged to the other side of A.
        state.remember(&Snapshot {
            monitors: vec![placed("A", 0), placed("B", -2560)],
        });

        assert_eq!(state.position(&id("C")).map(|rect| rect.x), Some(5120));
    }

    #[test]
    fn saving_replaces_the_file_and_leaves_nothing_else_behind() {
        let dir = temp_dir("state-replace");
        let path = dir.join("state.toml");
        applied("home").save(&path).unwrap();
        applied("work").save(&path).unwrap();

        assert_eq!(State::load(&path), applied("work"));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[test]
    fn positions_survive_a_round_trip_and_old_files_still_load() {
        let dir = temp_dir("state-positions");
        let path = dir.join("state.toml");
        let mut state = applied("work");
        state.remember(&Snapshot {
            monitors: vec![placed("L", -2560)],
        });
        state.save(&path).unwrap();
        assert_eq!(State::load(&path), state);

        fs::write(&path, "last_applied = \"home\"\n").unwrap();
        assert_eq!(State::load(&path), applied("home"));
    }

    #[test]
    fn missing_state_file_is_default() {
        let dir = temp_dir("state-missing");
        assert_eq!(State::load(&dir.join("state.toml")), State::default());
    }

    #[test]
    fn unreadable_state_file_is_default() {
        let dir = temp_dir("state-garbage");
        let path = dir.join("state.toml");
        std::fs::write(&path, "][").unwrap();
        assert_eq!(State::load(&path), State::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("state-roundtrip");
        let path = dir.join("nested").join("state.toml");

        applied("work").save(&path).unwrap();

        assert_eq!(State::load(&path), applied("work"));
    }

    #[test]
    fn current_matches() {
        let at_work = snap(&[("L", false, false), ("R", true, true)]);
        let config = desk();

        let current = current_layout(&config, &applied("work"), &at_work);

        assert_eq!(current.map(|layout| layout.name.as_str()), Some("work"));
    }

    #[test]
    fn current_is_none_when_displays_changed_since() {
        let both = snap(&[("L", true, false), ("R", true, true)]);

        assert!(current_layout(&desk(), &applied("work"), &both).is_none());
    }

    #[test]
    fn current_is_none_when_layout_was_deleted() {
        let both = snap(&[("L", true, false), ("R", true, true)]);

        assert!(current_layout(&desk(), &applied("gone"), &both).is_none());
        assert!(current_layout(&desk(), &State::default(), &both).is_none());
    }

    #[test]
    fn next_wraps_around() {
        let config = desk();
        let name = |state: &State| next_layout(&config, state).map(|layout| layout.name.clone());

        assert_eq!(name(&applied("home")).as_deref(), Some("work"));
        assert_eq!(name(&applied("work")).as_deref(), Some("home"));
    }

    #[test]
    fn next_skips_unusable_layouts() {
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;

        let next = next_layout(&config, &applied("home"));

        assert_eq!(next.map(|layout| layout.name.as_str()), Some("home"));
    }

    #[test]
    fn next_is_first_when_nothing_applied() {
        let config = desk();

        let next = next_layout(&config, &State::default());

        assert_eq!(next.map(|layout| layout.name.as_str()), Some("home"));
    }

    #[test]
    fn next_when_last_applied_was_deleted() {
        let config = desk();

        let next = next_layout(&config, &applied("gone"));

        assert_eq!(next.map(|layout| layout.name.as_str()), Some("home"));
    }

    #[test]
    fn next_is_none_without_usable_layouts() {
        let mut config = desk();
        config.layouts.clear();

        assert!(next_layout(&config, &State::default()).is_none());
    }
}
