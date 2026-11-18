//! The user's settings: their monitors and layouts.
//!
//! Stored as TOML so it can be read, diffed and edited by hand.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::display::MonitorId;
use crate::input::InputCode;

/// The settings file format this build reads and writes.
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    /// Applies the next layout in order each time it is pressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_hotkey: Option<String>,
    #[serde(default, rename = "monitor", skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<MonitorConfig>,
    #[serde(default, rename = "layout", skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<Layout>,
}

/// What Deskflip knows about one physical monitor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub id: MonitorId,
    /// The model name the monitor reports.
    pub model: String,
    /// The user's own name for it, e.g. "Left".
    pub label: String,
    /// The input this PC is plugged into: what every "bring it back" path sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc_input: Option<InputCode>,
    /// `pc_input` starts as a guess read from the monitor. Until a person confirms it, no
    /// layout may send this monitor to another device, because a wrong value would leave no
    /// way to bring it back.
    #[serde(default)]
    pub pc_input_confirmed: bool,
    /// The inputs the monitor says it has. Empty means it cannot be switched (no DDC/CI).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputCode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hotkey: Option<String>,
    /// The main display for this layout. `None` keeps whichever is main already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<MonitorId>,
    /// Monitors not listed are left exactly as they are.
    #[serde(default, rename = "monitor", skip_serializing_if = "Vec::is_empty")]
    pub monitors: Vec<LayoutMonitor>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutMonitor {
    pub id: MonitorId,
    pub role: Role,
}

/// What a monitor does in a layout. Input and desktop membership are decided together so that
/// combinations nobody wants (the PC drawing to a screen that shows something else, by
/// accident) cannot be configured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Shows this PC: switched to the PC's input and part of the desktop.
    Pc,
    /// Shows another device on `input`, and is removed from the desktop.
    Other { input: InputCode },
    /// Removed from the desktop; its input is left alone.
    Off,
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("could not read or write the settings file: {0}")]
    Io(#[from] io::Error),
    #[error("the settings file has a mistake in it: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("the settings could not be written: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error(
        "the settings file was written by a newer Deskflip (format {0}); update Deskflip to use it"
    )]
    UnsupportedVersion(u32),
}

impl Config {
    /// Reads the settings file. A file that does not exist is `Ok(None)`: a first run, not an
    /// error. A file that cannot be understood is never modified.
    pub fn load(path: &Path) -> Result<Option<Self>, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };

        // Check the version before the full parse, so a newer file is reported as such rather
        // than as whatever field this build happens not to understand.
        #[derive(Deserialize)]
        struct Probe {
            version: u32,
        }
        let probe: Probe = toml::from_str(&text)?;
        if probe.version != VERSION {
            return Err(ConfigError::UnsupportedVersion(probe.version));
        }

        Ok(Some(toml::from_str(&text)?))
    }

    /// Writes the settings file so that a crash mid-write cannot leave it half-written: the
    /// new contents go to a temporary file that then replaces the old one.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut temp = path.as_os_str().to_owned();
        temp.push(".tmp");
        fs::write(&temp, text)?;
        fs::rename(&temp, path)?;
        Ok(())
    }

    pub fn monitor(&self, id: &MonitorId) -> Option<&MonitorConfig> {
        self.monitors.iter().find(|monitor| &monitor.id == id)
    }

    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts
            .iter()
            .find(|layout| layout.name.eq_ignore_ascii_case(name))
    }
}

/// What was learned about a connected monitor by asking it.
#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub id: MonitorId,
    pub model: String,
    /// The input it reported showing, if it answered.
    pub current_input: Option<InputCode>,
    /// The inputs it listed, if it answered.
    pub inputs: Vec<InputCode>,
}

/// Folds freshly detected monitors into the settings. Returns whether anything changed.
///
/// New monitors are added. For known monitors only machine-read facts are refreshed; anything
/// the user set or confirmed is kept, and a monitor that did not answer this time keeps what
/// it reported before.
pub fn merge_detected(config: &mut Config, detected: &[Detected]) -> bool {
    let mut changed = false;
    for found in detected {
        match config
            .monitors
            .iter_mut()
            .find(|monitor| monitor.id == found.id)
        {
            Some(known) => {
                if !found.inputs.is_empty() && known.inputs != found.inputs {
                    known.inputs.clone_from(&found.inputs);
                    changed = true;
                }
                if known.pc_input.is_none() && found.current_input.is_some() {
                    known.pc_input = found.current_input;
                    changed = true;
                }
            }
            None => {
                config.monitors.push(MonitorConfig {
                    id: found.id.clone(),
                    model: found.model.clone(),
                    label: format!("Monitor {}", config.monitors.len() + 1),
                    pc_input: found.current_input,
                    pc_input_confirmed: false,
                    inputs: found.inputs.clone(),
                });
                changed = true;
            }
        }
    }
    changed
}

/// Settings for a first run: the detected monitors and one layout that uses them all.
pub fn starter(detected: &[Detected]) -> Config {
    let mut config = Config {
        version: VERSION,
        cycle_hotkey: None,
        monitors: Vec::new(),
        layouts: Vec::new(),
    };
    merge_detected(&mut config, detected);
    config.layouts.push(Layout {
        name: "All screens on this PC".to_owned(),
        hotkey: None,
        primary: None,
        monitors: config
            .monitors
            .iter()
            .map(|monitor| LayoutMonitor {
                id: monitor.id.clone(),
                role: Role::Pc,
            })
            .collect(),
    });
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{DP, HDMI1, HDMI2, desk, id, monitor, temp_dir};
    use std::fs;

    const DOCUMENTED: &str = r#"
version = 1
cycle_hotkey = "Ctrl+Alt+M"

[[monitor]]
id = '\\?\DISPLAY#DEL2014#5&e6a3024&0&UID4353#{e6f07b5f}'
model = "DELL G2725D"
label = "Left"
pc_input = 0x0F
pc_input_confirmed = true
inputs = [0x11, 0x12, 0x0F]

[[monitor]]
id = '\\?\DISPLAY#DEL2014#5&e6a3024&0&UID4355#{e6f07b5f}'
model = "DELL G2725D"
label = "Right"

[[layout]]
name = "Work laptop"
hotkey = "Ctrl+Alt+2"
[[layout.monitor]]
id = '\\?\DISPLAY#DEL2014#5&e6a3024&0&UID4353#{e6f07b5f}'
role = { other = { input = 0x12 } }
[[layout.monitor]]
id = '\\?\DISPLAY#DEL2014#5&e6a3024&0&UID4355#{e6f07b5f}'
role = "pc"
"#;

    fn detected(name: &str, current: Option<InputCode>, inputs: &[InputCode]) -> Detected {
        Detected {
            id: id(name),
            model: "MODEL".to_owned(),
            current_input: current,
            inputs: inputs.to_vec(),
        }
    }

    #[test]
    fn parses_the_documented_example() {
        let dir = temp_dir("config-documented");
        let path = dir.join("config.toml");
        fs::write(&path, DOCUMENTED).unwrap();

        let config = Config::load(&path).unwrap().unwrap();

        assert_eq!(config.version, 1);
        assert_eq!(config.cycle_hotkey.as_deref(), Some("Ctrl+Alt+M"));
        assert_eq!(config.monitors.len(), 2);
        let left = &config.monitors[0];
        assert_eq!(left.label, "Left");
        assert_eq!(left.pc_input, Some(DP));
        assert!(left.pc_input_confirmed);
        assert_eq!(left.inputs, [HDMI1, HDMI2, DP]);
        let right = &config.monitors[1];
        assert_eq!(right.pc_input, None);
        assert!(!right.pc_input_confirmed);
        assert!(right.inputs.is_empty());

        let work = config.layout("Work laptop").unwrap();
        assert_eq!(work.hotkey.as_deref(), Some("Ctrl+Alt+2"));
        assert_eq!(work.primary, None);
        assert_eq!(work.monitors[0].role, Role::Other { input: HDMI2 });
        assert_eq!(work.monitors[1].role, Role::Pc);
        assert_eq!(work.monitors[1].id, right.id);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir("config-roundtrip");
        let path = dir.join("nested").join("config.toml");
        let mut config = desk();
        config.cycle_hotkey = Some("Ctrl+Alt+M".to_owned());
        config.layouts[0].primary = Some(id("R"));
        config.layouts[1].monitors.push(LayoutMonitor {
            id: id("X"),
            role: Role::Off,
        });

        config.save(&path).unwrap();

        assert_eq!(Config::load(&path).unwrap().unwrap(), config);
    }

    #[test]
    fn missing_file_is_none() {
        let dir = temp_dir("config-missing");
        assert!(Config::load(&dir.join("config.toml")).unwrap().is_none());
    }

    #[test]
    fn garbage_is_parse_error_and_file_is_untouched() {
        let dir = temp_dir("config-garbage");
        let path = dir.join("config.toml");
        fs::write(&path, "not = [toml").unwrap();

        assert!(matches!(Config::load(&path), Err(ConfigError::Parse(_))));
        assert_eq!(fs::read_to_string(&path).unwrap(), "not = [toml");
    }

    #[test]
    fn newer_version_is_refused() {
        let dir = temp_dir("config-version");
        let path = dir.join("config.toml");
        fs::write(&path, "version = 2\nsomething_new = true\n").unwrap();

        assert!(matches!(
            Config::load(&path),
            Err(ConfigError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let dir = temp_dir("config-temp");
        let path = dir.join("config.toml");

        desk().save(&path).unwrap();
        desk().save(&path).unwrap();

        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["config.toml"]);
    }

    #[test]
    fn lookups_ignore_case() {
        let config = desk();
        assert!(config.monitor(&id("l")).is_some());
        assert!(config.layout("HOME").is_some());
        assert!(config.layout("nope").is_none());
    }

    #[test]
    fn merge_adds_new_monitor_unconfirmed() {
        let mut config = desk();

        let changed = merge_detected(&mut config, &[detected("N", Some(HDMI1), &[HDMI1, DP])]);

        assert!(changed);
        let added = config.monitor(&id("N")).unwrap();
        assert_eq!(added.label, "Monitor 3");
        assert_eq!(added.model, "MODEL");
        assert_eq!(added.pc_input, Some(HDMI1));
        assert!(!added.pc_input_confirmed);
        assert_eq!(added.inputs, [HDMI1, DP]);
    }

    #[test]
    fn merge_keeps_user_label_and_confirmation() {
        let mut config = desk();
        config.monitors[0].label = "My left one".to_owned();

        let changed = merge_detected(
            &mut config,
            &[detected("L", Some(HDMI2), &[HDMI1, HDMI2, DP])],
        );

        assert!(!changed);
        let left = config.monitor(&id("L")).unwrap();
        assert_eq!(left.label, "My left one");
        assert_eq!(left.pc_input, Some(DP));
        assert!(left.pc_input_confirmed);
    }

    #[test]
    fn merge_refreshes_the_input_list() {
        let mut config = desk();

        let changed = merge_detected(&mut config, &[detected("L", Some(DP), &[DP])]);

        assert!(changed);
        assert_eq!(config.monitor(&id("L")).unwrap().inputs, [DP]);
    }

    #[test]
    fn merge_does_not_clear_inputs_when_detection_returns_none() {
        let mut config = desk();

        let changed = merge_detected(&mut config, &[detected("L", None, &[])]);

        assert!(!changed);
        assert_eq!(config.monitor(&id("L")).unwrap().inputs, [HDMI1, HDMI2, DP]);
    }

    #[test]
    fn merge_fills_a_missing_pc_input_guess() {
        let mut config = desk();
        config.monitors[0] = MonitorConfig {
            pc_input: None,
            pc_input_confirmed: false,
            ..monitor("L", false)
        };

        merge_detected(&mut config, &[detected("L", Some(HDMI1), &[])]);

        let left = config.monitor(&id("L")).unwrap();
        assert_eq!(left.pc_input, Some(HDMI1));
        assert!(!left.pc_input_confirmed);
    }

    #[test]
    fn starter_has_one_all_pc_layout() {
        let config = starter(&[
            detected("A", Some(DP), &[DP, HDMI1]),
            detected("B", None, &[]),
        ]);

        assert_eq!(config.version, VERSION);
        assert_eq!(config.monitors.len(), 2);
        assert_eq!(config.monitors[0].label, "Monitor 1");
        assert_eq!(config.monitors[1].label, "Monitor 2");
        assert_eq!(config.layouts.len(), 1);
        let layout = &config.layouts[0];
        assert_eq!(layout.name, "All screens on this PC");
        assert_eq!(layout.monitors.len(), 2);
        assert!(layout.monitors.iter().all(|m| m.role == Role::Pc));
    }
}
