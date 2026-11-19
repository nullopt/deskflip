//! Carries out a layout change, and undoes it if anything goes wrong.
//!
//! The promise to the user is that a failed change never leaves them worse off: either the
//! monitors end up as the layout describes, or the desktop is put back as it was and every
//! monitor that was sent away is told to come back.

use crate::config::{Config, ConfigError};
use crate::display::{Display, DisplayError, MonitorId, WindowsDisplay};
use crate::paths;
use crate::plan::{Plan, PlanError, Step, plan};
use crate::state::State;
use crate::validate::layout_is_usable;
use crate::win::{APPLY_MUTEX, CHANGED_EVENT, NamedEvent, NamedMutex};

/// A layout change that succeeded.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    /// Monitors the layout names that were not plugged in, and so were left out.
    pub skipped: Vec<MonitorId>,
}

#[derive(thiserror::Error, Debug)]
pub enum ApplyError {
    #[error("there is no layout named \"{0}\"")]
    UnknownLayout(String),
    #[error("the layout \"{0}\" has a problem; open Settings to fix it")]
    InvalidLayout(String),
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// Nothing had been changed yet when this failed.
    #[error(transparent)]
    Display(#[from] DisplayError),
    #[error("{cause}; your previous display setup was restored")]
    RolledBack { cause: String },
    #[error("{cause}; restoring your previous display setup also failed: {rollback}")]
    RollbackFailed { cause: String, rollback: String },
    #[error("another layout change is still in progress")]
    Busy,
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// Applies a layout on this PC's real monitors, using the saved settings.
///
/// This is the one entry point every trigger uses (tray, hotkey, command line, settings
/// window). It makes sure only one change runs at a time across processes, remembers the
/// layout on success, and wakes the tray so its menu reflects the result.
pub fn run(layout_name: &str) -> Result<Outcome, ApplyError> {
    let Some(_lock) = NamedMutex::try_acquire(APPLY_MUTEX) else {
        return Err(ApplyError::Busy);
    };

    let result = Config::load(&paths::config_file())
        .map_err(ApplyError::from)
        .and_then(|config| {
            let config = config.ok_or_else(|| ApplyError::UnknownLayout(layout_name.to_owned()))?;
            // A monitor about to be handed over is on the desktop now: note where.
            let before = WindowsDisplay.snapshot();
            let outcome = apply_layout(&mut WindowsDisplay, &config, layout_name);
            // Record under the name as the settings spell it, not as the caller typed it.
            let name = config
                .layout(layout_name)
                .map_or(layout_name, |layout| layout.name.as_str());
            let state_file = paths::state_file();
            let mut state = State::load(&state_file);
            // Before and after: together they cover every monitor either layout uses.
            for seen in [before, WindowsDisplay.snapshot()].iter().flatten() {
                state.remember(seen);
            }
            record(&outcome, name, &mut state);
            if let Err(error) = state.save(&state_file) {
                log::warn!("could not save state: {error}");
            }
            outcome
        });

    match &result {
        Ok(outcome) => log::info!(
            "applied \"{layout_name}\" ({} monitor(s) skipped)",
            outcome.skipped.len()
        ),
        Err(error) => log::error!("applying \"{layout_name}\" failed: {error}"),
    }
    if let Ok(event) = NamedEvent::open_or_create(CHANGED_EVENT) {
        event.signal();
    }
    result
}

/// Updates the remembered layout after an attempt. A failed attempt was rolled back (or left
/// the monitors in an unknown arrangement), so what was remembered before stays.
fn record(result: &Result<Outcome, ApplyError>, layout_name: &str, state: &mut State) {
    if result.is_ok() {
        state.last_applied = Some(layout_name.to_owned());
    }
}

/// Applies the layout named `layout_name` (matched without regard to case).
pub fn apply_layout<D: Display>(
    display: &mut D,
    config: &Config,
    layout_name: &str,
) -> Result<Outcome, ApplyError> {
    let layout = config
        .layout(layout_name)
        .ok_or_else(|| ApplyError::UnknownLayout(layout_name.to_owned()))?;
    if !layout_is_usable(config, &layout.name) {
        return Err(ApplyError::InvalidLayout(layout.name.clone()));
    }

    let before = display.snapshot()?;
    let plan = plan(&before, config, layout)?;
    let saved = display.save()?;

    match execute(display, &plan) {
        Ok(()) => Ok(Outcome {
            skipped: plan.skipped,
        }),
        Err(cause) => Err(roll_back(display, &plan, &saved, cause)),
    }
}

fn execute<D: Display>(display: &mut D, plan: &Plan) -> Result<(), String> {
    for step in &plan.steps {
        match step {
            Step::ExtendAll => display.extend_all(),
            Step::SetInput { monitor, input } => display.set_input(monitor, *input),
            Step::SetActive { monitors, primary } => {
                // Extending often lands exactly on the target; changing the display mode again
                // would only make the screens flicker a second time.
                let now = display.snapshot().map_err(|error| error.to_string())?;
                if &now.active_ids() == monitors && now.primary() == Some(primary) {
                    Ok(())
                } else {
                    display.set_active(monitors, primary)
                }
            }
        }
        .map_err(|error| error.to_string())?;
    }

    let after = display.snapshot().map_err(|error| error.to_string())?;
    if after.active_ids() != plan.expected_active {
        return Err("Windows did not end up using the monitors this layout asks for".to_owned());
    }
    Ok(())
}

fn roll_back<D: Display>(
    display: &mut D,
    plan: &Plan,
    saved: &D::Saved,
    cause: String,
) -> ApplyError {
    if let Err(error) = display.restore(saved) {
        return ApplyError::RollbackFailed {
            cause,
            rollback: error.to_string(),
        };
    }

    // Bring back every monitor that was sent to another device. A monitor that is not on the
    // desktop after the restore was not on it before either, so it was never switched.
    let active = display
        .snapshot()
        .map(|snapshot| snapshot.active_ids())
        .unwrap_or_default();
    for (monitor, pc_input) in &plan.return_inputs {
        if active.contains(monitor)
            && let Err(error) = display.set_input(monitor, *pc_input)
        {
            log::warn!("could not switch {monitor} back to this PC during rollback: {error}");
        }
    }
    ApplyError::RolledBack { cause }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::fake::FakeDisplay;
    use crate::state::State;
    use crate::testing::{DP, HDMI2, desk, id, snap};

    fn at_home() -> FakeDisplay {
        FakeDisplay::new(snap(&[("L", true, false), ("R", true, true)]))
    }

    fn at_work() -> FakeDisplay {
        FakeDisplay::new(snap(&[("L", false, false), ("R", true, true)]))
    }

    fn position(calls: &[String], prefix: &str) -> usize {
        calls
            .iter()
            .position(|call| call.starts_with(prefix))
            .unwrap_or_else(|| panic!("no call starting with {prefix} in {calls:?}"))
    }

    #[test]
    fn applies_work_layout() {
        let mut display = at_home();

        let outcome = apply_layout(&mut display, &desk(), "work").unwrap();

        assert!(outcome.skipped.is_empty());
        assert_eq!(
            display.state,
            snap(&[("L", false, false), ("R", true, true)])
        );
        assert_eq!(display.inputs[&id("L")], HDMI2);
    }

    #[test]
    fn applies_home_layout_from_work() {
        let mut display = at_work();

        apply_layout(&mut display, &desk(), "home").unwrap();

        assert_eq!(
            display.state,
            snap(&[("L", true, false), ("R", true, true)])
        );
        assert_eq!(display.inputs[&id("L")], DP);
    }

    #[test]
    fn call_order_sends_inputs_before_removing_the_monitor() {
        let mut display = at_home();

        apply_layout(&mut display, &desk(), "work").unwrap();

        let calls = display.calls();
        assert!(position(&calls, "set_input(L)") < position(&calls, "set_active"));
    }

    #[test]
    fn call_order_brings_a_monitor_back_before_switching_it() {
        let mut display = at_work();

        apply_layout(&mut display, &desk(), "home").unwrap();

        let calls = display.calls();
        assert!(position(&calls, "extend_all") < position(&calls, "set_input(L)"));
    }

    #[test]
    fn set_input_failure_rolls_back() {
        let mut display = at_home();
        display.fail_on = vec!["set_input"];

        let error = apply_layout(&mut display, &desk(), "work").unwrap_err();

        assert!(matches!(error, ApplyError::RolledBack { .. }), "{error:?}");
        assert_eq!(
            display.state,
            snap(&[("L", true, false), ("R", true, true)])
        );
    }

    #[test]
    fn verify_mismatch_rolls_back() {
        let mut display = at_home();
        display.ignore_set_active = true;

        let error = apply_layout(&mut display, &desk(), "work").unwrap_err();

        assert!(matches!(error, ApplyError::RolledBack { .. }), "{error:?}");
        assert_eq!(display.inputs[&id("L")], DP);
        assert_eq!(
            display.state,
            snap(&[("L", true, false), ("R", true, true)])
        );
    }

    #[test]
    fn rollback_failure_is_reported() {
        let mut display = at_home();
        display.fail_on = vec!["set_active", "restore"];

        let error = apply_layout(&mut display, &desk(), "work").unwrap_err();

        assert!(
            matches!(error, ApplyError::RollbackFailed { .. }),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("set_active"), "{message}");
        assert!(message.contains("restore"), "{message}");
    }

    #[test]
    fn refused_plan_makes_no_calls() {
        let mut display = FakeDisplay::new(snap(&[("L", true, true)]));

        let error = apply_layout(&mut display, &desk(), "work").unwrap_err();

        assert!(matches!(
            error,
            ApplyError::Plan(PlanError::NoPcMonitorConnected)
        ));
        assert_eq!(display.calls(), ["snapshot"]);
    }

    #[test]
    fn disconnected_monitor_is_reported_as_skipped() {
        let mut display = FakeDisplay::new(snap(&[("R", true, true)]));

        let outcome = apply_layout(&mut display, &desk(), "work").unwrap();

        assert_eq!(outcome.skipped, [id("L")]);
    }

    #[test]
    fn unknown_layout() {
        let mut display = at_home();

        let error = apply_layout(&mut display, &desk(), "nope").unwrap_err();

        assert!(matches!(error, ApplyError::UnknownLayout(name) if name == "nope"));
        assert!(display.calls().is_empty());
    }

    #[test]
    fn invalid_layout_is_refused() {
        let mut display = at_home();
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;

        let error = apply_layout(&mut display, &config, "work").unwrap_err();

        assert!(matches!(error, ApplyError::InvalidLayout(name) if name == "work"));
        assert!(display.calls().is_empty());
    }

    #[test]
    fn redundant_set_active_is_skipped() {
        let mut display = at_work();

        apply_layout(&mut display, &desk(), "home").unwrap();

        assert!(!display.calls().iter().any(|call| call == "set_active"));
    }

    #[test]
    fn layout_names_are_matched_without_regard_to_case() {
        let mut display = at_home();

        assert!(apply_layout(&mut display, &desk(), "WORK").is_ok());
    }

    #[test]
    fn state_updated_on_success() {
        let mut state = State::default();

        record(
            &Ok(Outcome {
                skipped: Vec::new(),
            }),
            "work",
            &mut state,
        );

        assert_eq!(state.last_applied.as_deref(), Some("work"));
    }

    #[test]
    fn state_unchanged_on_rollback() {
        let mut state = State {
            last_applied: Some("home".to_owned()),
            ..State::default()
        };
        let failed = Err(ApplyError::RolledBack {
            cause: "x".to_owned(),
        });

        record(&failed, "work", &mut state);

        assert_eq!(state.last_applied.as_deref(), Some("home"));
    }

    #[test]
    fn state_unchanged_on_rollback_failure() {
        let mut state = State {
            last_applied: Some("home".to_owned()),
            ..State::default()
        };
        let failed = Err(ApplyError::RollbackFailed {
            cause: "x".to_owned(),
            rollback: "y".to_owned(),
        });

        record(&failed, "work", &mut state);

        assert_eq!(state.last_applied.as_deref(), Some("home"));
    }
}
