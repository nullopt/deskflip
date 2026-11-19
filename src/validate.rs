//! Checks a configuration for mistakes that would make a layout unsafe or meaningless.
//!
//! Problems are reported, never fixed silently: the settings window shows them and blocks
//! saving, and a layout with a problem is refused when applied.

use std::collections::HashSet;
use std::fmt;

use crate::config::{Config, Layout, Role};
use crate::display::MonitorId;
use crate::hotkey;

#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    /// The layout the problem belongs to, or `None` for a general setting.
    pub layout: Option<String>,
    pub kind: ProblemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProblemKind {
    EmptyLayoutName,
    DuplicateLayoutName,
    /// Holds the text as the user wrote it.
    BadHotkey(String),
    /// Holds the hotkey in display form.
    DuplicateHotkey(String),
    UnknownMonitor(MonitorId),
    DuplicateMonitor(MonitorId),
    /// The monitor is sent to another device, but nobody has confirmed which input brings it
    /// back to this PC.
    PcInputUnconfirmed(MonitorId),
    /// The monitor is sent to the very input this PC is plugged into: it would go on showing
    /// this PC while Windows stops using it.
    SentToPcInput(MonitorId),
    PrimaryNotPc(MonitorId),
    /// No monitor would show this PC, and no hotkey leads back to a layout where one does.
    NoWayBack,
}

impl Problem {
    /// A sentence for the settings window, naming monitors by the user's labels.
    pub fn message(&self, config: &Config) -> String {
        let label = |id: &MonitorId| {
            config
                .monitor(id)
                .map_or_else(|| "A monitor".to_owned(), |monitor| monitor.label.clone())
        };
        match &self.kind {
            ProblemKind::UnknownMonitor(_) => {
                "This layout refers to a monitor that is no longer in Deskflip's list.".to_owned()
            }
            ProblemKind::DuplicateMonitor(id) => {
                format!("{} appears twice in this layout.", label(id))
            }
            ProblemKind::PcInputUnconfirmed(id) => format!(
                "{} cannot be handed to another device until you confirm which input this PC is plugged into (Monitors page).",
                label(id)
            ),
            ProblemKind::SentToPcInput(id) => format!(
                "{} would be switched to the input this PC is plugged into. Choose the input the other device uses.",
                label(id)
            ),
            ProblemKind::PrimaryNotPc(id) => format!(
                "{} is set as the main display but does not show this PC in this layout.",
                label(id)
            ),
            other => other.to_string(),
        }
    }
}

impl fmt::Display for ProblemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyLayoutName => f.write_str("A layout needs a name."),
            Self::DuplicateLayoutName => f.write_str("Another layout already has this name."),
            Self::BadHotkey(text) => write!(
                f,
                "\"{text}\" is not a usable hotkey. Use Ctrl, Alt or Win with another key."
            ),
            Self::DuplicateHotkey(text) => write!(f, "{text} is already used by another hotkey."),
            Self::UnknownMonitor(id) => write!(f, "Unknown monitor {id}."),
            Self::DuplicateMonitor(id) => write!(f, "Monitor {id} is listed twice."),
            Self::PcInputUnconfirmed(id) => write!(
                f,
                "Monitor {id} cannot be handed to another device until this PC's input on it is confirmed."
            ),
            Self::SentToPcInput(id) => write!(
                f,
                "Monitor {id} would be switched to the input this PC is plugged into."
            ),
            Self::PrimaryNotPc(id) => write!(
                f,
                "Monitor {id} is set as the main display but does not show this PC."
            ),
            Self::NoWayBack => f.write_str(
                "This PC would not be visible, and no hotkey brings it back. Give a hotkey to a layout that shows this PC, or set a cycle hotkey.",
            ),
        }
    }
}

pub fn validate(config: &Config) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut names: HashSet<String> = HashSet::new();
    let mut hotkeys: HashSet<u32> = HashSet::new();
    let cycle_works = config
        .cycle_hotkey
        .as_deref()
        .is_some_and(|text| hotkey::parse(text).is_ok());

    for (index, layout) in config.layouts.iter().enumerate() {
        let mut report = |kind| {
            problems.push(Problem {
                layout: Some(layout.name.clone()),
                kind,
            });
        };

        if layout.name.trim().is_empty() {
            report(ProblemKind::EmptyLayoutName);
        } else if !names.insert(layout.name.trim().to_lowercase()) {
            report(ProblemKind::DuplicateLayoutName);
        }

        if let Some(text) = &layout.hotkey {
            check_hotkey(text, &mut hotkeys, &mut report);
        }

        let mut seen: HashSet<&MonitorId> = HashSet::new();
        for entry in &layout.monitors {
            if !seen.insert(&entry.id) {
                report(ProblemKind::DuplicateMonitor(entry.id.clone()));
                continue;
            }
            let Some(monitor) = config.monitor(&entry.id) else {
                report(ProblemKind::UnknownMonitor(entry.id.clone()));
                continue;
            };
            if let Role::Other { input } = entry.role {
                if !(monitor.pc_input.is_some() && monitor.pc_input_confirmed) {
                    report(ProblemKind::PcInputUnconfirmed(entry.id.clone()));
                } else if monitor.pc_input == Some(input) {
                    report(ProblemKind::SentToPcInput(entry.id.clone()));
                }
            }
        }

        if let Some(primary) = &layout.primary
            && !layout
                .monitors
                .iter()
                .any(|entry| &entry.id == primary && entry.role == Role::Pc)
        {
            report(ProblemKind::PrimaryNotPc(primary.clone()));
        }

        if hides_pc(layout) && !has_way_back(config, index, cycle_works) {
            report(ProblemKind::NoWayBack);
        }
    }

    if let Some(text) = &config.cycle_hotkey {
        check_hotkey(text, &mut hotkeys, &mut |kind| {
            problems.push(Problem { layout: None, kind });
        });
    }

    problems
}

/// Whether nothing stands in the way of applying this layout.
pub fn layout_is_usable(config: &Config, layout: &str) -> bool {
    validate(config).iter().all(|problem| {
        problem
            .layout
            .as_deref()
            .is_none_or(|name| !name.eq_ignore_ascii_case(layout))
    })
}

fn check_hotkey(text: &str, taken: &mut HashSet<u32>, report: &mut impl FnMut(ProblemKind)) {
    match hotkey::parse(text) {
        Err(_) => report(ProblemKind::BadHotkey(text.to_owned())),
        Ok(parsed) => {
            if !taken.insert(parsed.id()) {
                report(ProblemKind::DuplicateHotkey(hotkey::format(&parsed)));
            }
        }
    }
}

fn shows_pc(layout: &Layout) -> bool {
    layout.monitors.iter().any(|entry| entry.role == Role::Pc)
}

/// A layout that sends monitors to another device without saying which monitor keeps showing
/// this PC. On a one-monitor desk that leaves the PC running but out of sight.
fn hides_pc(layout: &Layout) -> bool {
    !shows_pc(layout)
        && layout
            .monitors
            .iter()
            .any(|entry| matches!(entry.role, Role::Other { .. }))
}

/// Whether a hotkey, usable without seeing the screen, leads from the layout at `hidden` to a
/// layout that shows this PC.
fn has_way_back(config: &Config, hidden: usize, cycle_works: bool) -> bool {
    config.layouts.iter().enumerate().any(|(index, layout)| {
        index != hidden
            && shows_pc(layout)
            && (cycle_works
                || layout
                    .hotkey
                    .as_deref()
                    .is_some_and(|text| hotkey::parse(text).is_ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;
    use crate::testing::{HDMI1, HDMI2, config, desk, id, layout, monitor};

    fn kinds(config: &Config) -> Vec<ProblemKind> {
        validate(config)
            .into_iter()
            .map(|problem| problem.kind)
            .collect()
    }

    #[test]
    fn the_reference_desk_is_valid() {
        assert_eq!(kinds(&desk()), []);
        assert!(layout_is_usable(&desk(), "work"));
    }

    #[test]
    fn other_role_requires_confirmed_pc_input() {
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;

        let problems = validate(&config);

        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].layout.as_deref(), Some("work"));
        assert_eq!(problems[0].kind, ProblemKind::PcInputUnconfirmed(id("L")));
        assert!(!layout_is_usable(&config, "work"));
        assert!(layout_is_usable(&config, "home"));
    }

    #[test]
    fn a_monitor_cannot_be_handed_to_the_input_this_pc_is_on() {
        let mut config = desk();
        config.monitors[0].pc_input = Some(HDMI2);

        let problems = validate(&config);

        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::SentToPcInput(id("L")));
        assert!(problems[0].message(&config).starts_with("L "));
        assert!(!layout_is_usable(&config, "work"));
        assert!(layout_is_usable(&config, "home"));
    }

    #[test]
    fn other_role_requires_a_pc_input_at_all() {
        let mut config = desk();
        config.monitors[0].pc_input = None;

        assert_eq!(kinds(&config), [ProblemKind::PcInputUnconfirmed(id("L"))]);
    }

    #[test]
    fn hidden_layout_without_any_hotkey_has_no_way_back() {
        let config = config(
            vec![monitor("S", true)],
            vec![
                layout("pc", None, &[("S", Role::Pc)]),
                layout(
                    "laptop",
                    Some("Ctrl+Alt+2"),
                    &[("S", Role::Other { input: HDMI1 })],
                ),
            ],
        );

        let problems = validate(&config);

        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].layout.as_deref(), Some("laptop"));
        assert_eq!(problems[0].kind, ProblemKind::NoWayBack);
    }

    #[test]
    fn hidden_layout_is_fine_when_a_pc_layout_has_a_hotkey() {
        let config = config(
            vec![monitor("S", true)],
            vec![
                layout("pc", Some("Ctrl+Alt+1"), &[("S", Role::Pc)]),
                layout("laptop", None, &[("S", Role::Other { input: HDMI1 })]),
            ],
        );

        assert_eq!(kinds(&config), []);
    }

    #[test]
    fn hidden_layout_is_fine_when_cycle_hotkey_is_set() {
        let mut config = config(
            vec![monitor("S", true)],
            vec![
                layout("pc", None, &[("S", Role::Pc)]),
                layout("laptop", None, &[("S", Role::Other { input: HDMI1 })]),
            ],
        );
        config.cycle_hotkey = Some("Ctrl+Alt+M".to_owned());

        assert_eq!(kinds(&config), []);
    }

    #[test]
    fn hidden_layout_is_not_rescued_by_its_own_hotkey() {
        let config = config(
            vec![monitor("S", true)],
            vec![layout(
                "laptop",
                Some("Ctrl+Alt+2"),
                &[("S", Role::Other { input: HDMI1 })],
            )],
        );

        assert_eq!(kinds(&config), [ProblemKind::NoWayBack]);
    }

    #[test]
    fn cycle_hotkey_does_not_help_when_no_layout_shows_the_pc() {
        let mut config = config(
            vec![monitor("S", true)],
            vec![layout(
                "laptop",
                None,
                &[("S", Role::Other { input: HDMI1 })],
            )],
        );
        config.cycle_hotkey = Some("Ctrl+Alt+M".to_owned());

        assert_eq!(kinds(&config), [ProblemKind::NoWayBack]);
    }

    #[test]
    fn duplicate_hotkeys_across_layouts_and_cycle() {
        let mut config = desk();
        config.layouts[1].hotkey = Some("alt+ctrl+1".to_owned());
        config.cycle_hotkey = Some("Control+Alt+Digit1".to_owned());

        let problems = validate(&config);

        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].layout.as_deref(), Some("work"));
        assert_eq!(
            problems[0].kind,
            ProblemKind::DuplicateHotkey("Ctrl+Alt+1".to_owned())
        );
        assert_eq!(problems[1].layout, None);
        assert_eq!(
            problems[1].kind,
            ProblemKind::DuplicateHotkey("Ctrl+Alt+1".to_owned())
        );
    }

    #[test]
    fn bad_hotkey_string() {
        let mut config = desk();
        config.layouts[0].hotkey = Some("Ctrl+Nope".to_owned());
        config.cycle_hotkey = Some("M".to_owned());

        let problems = validate(&config);

        assert_eq!(problems.len(), 2);
        assert_eq!(problems[0].layout.as_deref(), Some("home"));
        assert_eq!(
            problems[0].kind,
            ProblemKind::BadHotkey("Ctrl+Nope".to_owned())
        );
        assert_eq!(problems[1].layout, None);
        assert_eq!(problems[1].kind, ProblemKind::BadHotkey("M".to_owned()));
    }

    #[test]
    fn unknown_monitor_reference() {
        let mut config = desk();
        config.layouts[0].monitors[0].id = id("GONE");

        assert_eq!(kinds(&config), [ProblemKind::UnknownMonitor(id("GONE"))]);
    }

    #[test]
    fn monitor_listed_twice_in_a_layout() {
        let mut config = desk();
        config.layouts[0].monitors[1].id = id("l");

        assert_eq!(kinds(&config), [ProblemKind::DuplicateMonitor(id("L"))]);
    }

    #[test]
    fn duplicate_layout_name_ignores_case() {
        let mut config = desk();
        config.layouts[1].name = "HOME".to_owned();
        config.layouts[1].hotkey = None;

        let problems = validate(&config);

        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::DuplicateLayoutName);
    }

    #[test]
    fn empty_layout_name() {
        let mut config = desk();
        config.layouts[0].name = "  ".to_owned();

        assert_eq!(kinds(&config), [ProblemKind::EmptyLayoutName]);
    }

    #[test]
    fn primary_must_be_a_pc_monitor() {
        let mut config = desk();
        config.layouts[1].primary = Some(id("L"));

        assert_eq!(kinds(&config), [ProblemKind::PrimaryNotPc(id("L"))]);
    }

    #[test]
    fn problems_read_as_plain_sentences() {
        assert_eq!(
            ProblemKind::NoWayBack.to_string(),
            "This PC would not be visible, and no hotkey brings it back. Give a hotkey to a layout that shows this PC, or set a cycle hotkey."
        );
        assert_eq!(
            ProblemKind::BadHotkey("M".to_owned()).to_string(),
            "\"M\" is not a usable hotkey. Use Ctrl, Alt or Win with another key."
        );
    }

    #[test]
    fn messages_name_monitors_by_label() {
        let mut config = desk();
        config.monitors[0].label = "Left".to_owned();
        config.monitors[0].pc_input_confirmed = false;

        let problems = validate(&config);

        assert_eq!(
            problems[0].message(&config),
            "Left cannot be handed to another device until you confirm which input this PC is plugged into (Monitors page)."
        );
    }
}
