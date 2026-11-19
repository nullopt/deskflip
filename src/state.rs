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
use crate::display::Snapshot;
use crate::plan::plan;
use crate::validate::layout_is_usable;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_applied: Option<String>,
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
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(io::Error::other)?;
        fs::write(path, text)
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
    use crate::testing::{desk, snap, temp_dir};

    fn applied(name: &str) -> State {
        State {
            last_applied: Some(name.to_owned()),
        }
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
