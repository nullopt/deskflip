//! Works out the steps that take the monitors from where they are to what a layout asks for.
//!
//! This module only decides; it never touches hardware. The order of steps follows from one
//! fact about monitors: a monitor can only be sent a command while it is part of the desktop.
//! So a monitor that is leaving is told to switch input *before* it is removed, and a monitor
//! that is coming back is added *before* it is told to switch.

use std::collections::BTreeSet;

use crate::config::{Config, Layout, Role};
use crate::display::{MonitorId, Snapshot};
use crate::input::InputCode;

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Put every connected monitor on the desktop.
    ExtendAll,
    SetInput {
        monitor: MonitorId,
        input: InputCode,
    },
    /// Reduce the desktop to exactly these monitors, with `primary` as the main display.
    SetActive {
        monitors: BTreeSet<MonitorId>,
        primary: MonitorId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub steps: Vec<Step>,
    /// The monitors that should be on the desktop once the steps have run.
    pub expected_active: BTreeSet<MonitorId>,
    /// Monitors the layout names that are not plugged in.
    pub skipped: Vec<MonitorId>,
    /// Monitors being sent to another device, with the input that brings each back to this PC.
    /// Used to undo the switch if a later step fails.
    pub return_inputs: Vec<(MonitorId, InputCode)>,
}

#[derive(thiserror::Error, Clone, Debug, PartialEq)]
pub enum PlanError {
    #[error("none of the monitors this layout uses for this PC are connected")]
    NoPcMonitorConnected,
    #[error("this layout would leave this PC with no monitor")]
    NothingActive,
}

pub fn plan(snapshot: &Snapshot, config: &Config, layout: &Layout) -> Result<Plan, PlanError> {
    let before = snapshot.active_ids();

    let mut skipped = Vec::new();
    let mut inputs: Vec<(MonitorId, InputCode)> = Vec::new();
    let mut return_inputs = Vec::new();
    let mut joining: BTreeSet<MonitorId> = BTreeSet::new();
    let mut handed_over: BTreeSet<MonitorId> = BTreeSet::new();
    let mut turned_off: BTreeSet<MonitorId> = BTreeSet::new();

    for entry in &layout.monitors {
        if snapshot.get(&entry.id).is_none() {
            skipped.push(entry.id.clone());
            continue;
        }
        let known = config.monitor(&entry.id);
        match entry.role {
            Role::Pc => {
                joining.insert(entry.id.clone());
                // An unconfirmed input is only a guess; sending a wrong one would switch the
                // monitor away from this PC.
                if let Some(monitor) = known
                    && monitor.pc_input_confirmed
                    && let Some(input) = monitor.pc_input
                {
                    inputs.push((entry.id.clone(), input));
                }
            }
            Role::Other { input } => {
                handed_over.insert(entry.id.clone());
                inputs.push((entry.id.clone(), input));
                if let Some(pc_input) = known.and_then(|monitor| monitor.pc_input) {
                    return_inputs.push((entry.id.clone(), pc_input));
                }
            }
            Role::Off => {
                turned_off.insert(entry.id.clone());
            }
        }
    }

    let wants_pc_monitor = layout.monitors.iter().any(|entry| entry.role == Role::Pc);
    if wants_pc_monitor && joining.is_empty() {
        return Err(PlanError::NoPcMonitorConnected);
    }

    // Monitors the layout does not mention stay as they are.
    let mut expected: BTreeSet<MonitorId> = before
        .iter()
        .filter(|id| !handed_over.contains(id) && !turned_off.contains(id))
        .cloned()
        .chain(joining)
        .collect();
    if expected.is_empty() {
        // Windows needs at least one display. If every monitor is being handed to another
        // device (one shared monitor is the common case), the desktop stays where it is, out
        // of sight, and only the inputs change.
        expected = before
            .iter()
            .filter(|id| handed_over.contains(id))
            .cloned()
            .collect();
    }
    if expected.is_empty() {
        return Err(PlanError::NothingActive);
    }

    let config_order = |id: &MonitorId| {
        config
            .monitors
            .iter()
            .position(|monitor| &monitor.id == id)
            .unwrap_or(usize::MAX)
    };
    inputs.sort_by_key(|(id, _)| config_order(id));

    let is_inactive = |id: &MonitorId| snapshot.get(id).is_some_and(|monitor| !monitor.active);
    let extend = inputs.iter().any(|(id, _)| is_inactive(id)) || expected.iter().any(is_inactive);

    let primary = layout
        .primary
        .as_ref()
        .filter(|id| expected.contains(id))
        .or_else(|| snapshot.primary().filter(|id| expected.contains(id)))
        .or_else(|| expected.iter().min_by_key(|id| config_order(id)))
        .cloned()
        .ok_or(PlanError::NothingActive)?;

    let mut steps = Vec::new();
    if extend {
        steps.push(Step::ExtendAll);
    }
    steps.extend(
        inputs
            .into_iter()
            .map(|(monitor, input)| Step::SetInput { monitor, input }),
    );
    // After extending, Windows decides the arrangement, so the target is always re-stated.
    if extend || expected != before || snapshot.primary() != Some(&primary) {
        steps.push(Step::SetActive {
            monitors: expected.clone(),
            primary,
        });
    }

    Ok(Plan {
        steps,
        expected_active: expected,
        skipped,
        return_inputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MonitorConfig, Role};
    use crate::testing::{DP, HDMI1, HDMI2, config, desk, id, layout, monitor, snap};

    fn set(names: &[&str]) -> BTreeSet<MonitorId> {
        names.iter().map(|name| id(name)).collect()
    }

    fn input(name: &str, input: InputCode) -> Step {
        Step::SetInput {
            monitor: id(name),
            input,
        }
    }

    fn active(names: &[&str], primary: &str) -> Step {
        Step::SetActive {
            monitors: set(names),
            primary: id(primary),
        }
    }

    fn plan_for(
        config: &Config,
        layout_name: &str,
        snapshot: &Snapshot,
    ) -> Result<Plan, PlanError> {
        plan(snapshot, config, config.layout(layout_name).unwrap())
    }

    #[test]
    fn work_from_home() {
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&desk(), "work", &both).unwrap();

        assert_eq!(
            plan.steps,
            [input("L", HDMI2), input("R", DP), active(&["R"], "R")]
        );
        assert_eq!(plan.expected_active, set(&["R"]));
        assert_eq!(plan.return_inputs, [(id("L"), DP)]);
        assert!(plan.skipped.is_empty());
    }

    #[test]
    fn home_from_work() {
        let only_right = snap(&[("L", false, false), ("R", true, true)]);

        let plan = plan_for(&desk(), "home", &only_right).unwrap();

        assert_eq!(
            plan.steps,
            [
                Step::ExtendAll,
                input("L", DP),
                input("R", DP),
                active(&["L", "R"], "R")
            ]
        );
        assert_eq!(plan.expected_active, set(&["L", "R"]));
        assert!(plan.return_inputs.is_empty());
    }

    #[test]
    fn home_when_already_home() {
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&desk(), "home", &both).unwrap();

        assert_eq!(plan.steps, [input("L", DP), input("R", DP)]);
    }

    #[test]
    fn unconfirmed_pc_input_is_never_sent() {
        let mut config = desk();
        config.monitors[1].pc_input_confirmed = false;
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&config, "home", &both).unwrap();

        assert_eq!(plan.steps, [input("L", DP)]);
    }

    #[test]
    fn off_role_sends_no_input() {
        let mut config = desk();
        config
            .layouts
            .push(layout("solo", None, &[("L", Role::Off), ("R", Role::Pc)]));
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&config, "solo", &both).unwrap();

        assert_eq!(plan.steps, [input("R", DP), active(&["R"], "R")]);
        assert!(plan.return_inputs.is_empty());
    }

    #[test]
    fn unlisted_monitor_keeps_its_state() {
        let three = snap(&[("L", true, false), ("M", true, false), ("R", true, true)]);

        let plan = plan_for(&desk(), "work", &three).unwrap();

        assert_eq!(plan.expected_active, set(&["M", "R"]));
        assert_eq!(plan.steps.last(), Some(&active(&["M", "R"], "R")));
    }

    #[test]
    fn unlisted_inactive_monitor_stays_off() {
        let three = snap(&[("L", true, false), ("M", false, false), ("R", true, true)]);

        let plan = plan_for(&desk(), "work", &three).unwrap();

        assert_eq!(plan.expected_active, set(&["R"]));
        assert!(!plan.steps.contains(&Step::ExtendAll));
    }

    #[test]
    fn hidden_layout_leaves_desktop_alone() {
        let config = config(
            vec![monitor("S", true)],
            vec![layout(
                "laptop",
                None,
                &[("S", Role::Other { input: HDMI1 })],
            )],
        );
        let single = snap(&[("S", true, true)]);

        let plan = plan_for(&config, "laptop", &single).unwrap();

        assert_eq!(plan.steps, [input("S", HDMI1)]);
        assert_eq!(plan.expected_active, set(&["S"]));
        assert_eq!(plan.return_inputs, [(id("S"), DP)]);
    }

    #[test]
    fn hidden_layout_on_inactive_monitor_restores_previous_set() {
        let mut config = desk();
        config.layouts.push(layout(
            "left away",
            None,
            &[("L", Role::Other { input: HDMI2 })],
        ));
        let only_right = snap(&[("L", false, false), ("R", true, true)]);

        let plan = plan_for(&config, "left away", &only_right).unwrap();

        assert_eq!(
            plan.steps,
            [Step::ExtendAll, input("L", HDMI2), active(&["R"], "R")]
        );
        assert_eq!(plan.expected_active, set(&["R"]));
    }

    #[test]
    fn handed_over_monitor_leaves_the_desktop_when_another_remains() {
        let mut config = desk();
        config.layouts.push(layout(
            "left away",
            None,
            &[("L", Role::Other { input: HDMI2 })],
        ));
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&config, "left away", &both).unwrap();

        assert_eq!(plan.steps, [input("L", HDMI2), active(&["R"], "R")]);
    }

    #[test]
    fn primary_moves_when_current_primary_leaves() {
        let left_is_main = snap(&[("L", true, true), ("R", true, false)]);

        let plan = plan_for(&desk(), "work", &left_is_main).unwrap();

        assert_eq!(plan.steps.last(), Some(&active(&["R"], "R")));
    }

    #[test]
    fn new_primary_follows_config_order() {
        let mut config = config(
            vec![monitor("A", true), monitor("B", true), monitor("C", true)],
            vec![layout(
                "x",
                None,
                &[
                    ("C", Role::Pc),
                    ("A", Role::Other { input: HDMI1 }),
                    ("B", Role::Pc),
                ],
            )],
        );
        config.monitors[2].pc_input = None;
        let all = snap(&[("A", true, true), ("B", true, false), ("C", true, false)]);

        let plan = plan_for(&config, "x", &all).unwrap();

        assert_eq!(plan.steps.last(), Some(&active(&["B", "C"], "B")));
    }

    #[test]
    fn layout_primary_wins() {
        let mut config = desk();
        config.layouts[0].primary = Some(id("L"));
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let plan = plan_for(&config, "home", &both).unwrap();

        assert_eq!(
            plan.steps,
            [input("L", DP), input("R", DP), active(&["L", "R"], "L")]
        );
    }

    #[test]
    fn pc_monitor_without_ddc_is_still_brought_back() {
        let mut config = desk();
        config.monitors[0] = MonitorConfig {
            pc_input: None,
            pc_input_confirmed: false,
            inputs: Vec::new(),
            ..monitor("L", false)
        };
        let only_right = snap(&[("L", false, false), ("R", true, true)]);

        let plan = plan_for(&config, "home", &only_right).unwrap();

        assert_eq!(
            plan.steps,
            [Step::ExtendAll, input("R", DP), active(&["L", "R"], "R")]
        );
    }

    #[test]
    fn disconnected_monitor_is_skipped() {
        let left_unplugged = snap(&[("R", true, true)]);

        let plan = plan_for(&desk(), "work", &left_unplugged).unwrap();

        assert_eq!(plan.skipped, [id("L")]);
        assert_eq!(plan.steps, [input("R", DP)]);
        assert_eq!(plan.expected_active, set(&["R"]));
        assert!(plan.return_inputs.is_empty());
    }

    #[test]
    fn refused_when_no_pc_monitor_connected() {
        let right_unplugged = snap(&[("L", true, true)]);

        assert_eq!(
            plan_for(&desk(), "work", &right_unplugged),
            Err(PlanError::NoPcMonitorConnected)
        );
    }

    #[test]
    fn refused_when_everything_would_be_off() {
        let mut config = desk();
        config
            .layouts
            .push(layout("dark", None, &[("L", Role::Off), ("R", Role::Off)]));
        let both = snap(&[("L", true, false), ("R", true, true)]);

        assert_eq!(
            plan_for(&config, "dark", &both),
            Err(PlanError::NothingActive)
        );
    }
}
