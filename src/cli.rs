//! Command-line entry point.

use std::env;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::process::ExitCode;

use crate::apply;
use crate::config::Config;
use crate::display::{Display, Snapshot, WindowsDisplay};
use crate::hotkey;
use crate::input::{input_name, parse_input_codes};
use crate::logger;
use crate::paths;
use crate::state::{State, current_layout};
use crate::tray;
use crate::validate::validate;
use crate::win;

const USAGE: &str = "\
Usage: deskflip [command]

With no command, Deskflip runs in the system tray.

Commands:
  apply <layout>   Switch to a layout
  list             List layouts; * marks the one in effect
  monitors         List connected monitors and what they report (useful in bug reports)
";

pub fn run() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    win::attach_parent_console();
    logger::init(&paths::log_file());

    match args.split_first() {
        None => tray::run(),
        Some((command, rest)) if command == "apply" && !rest.is_empty() => apply(&rest.join(" ")),
        Some((command, [])) if command == "list" => list(),
        Some((command, [])) if command == "monitors" => {
            print(&monitors_report(&WindowsDisplay));
            ExitCode::SUCCESS
        }
        _ => {
            print(USAGE);
            ExitCode::from(2)
        }
    }
}

fn apply(layout: &str) -> ExitCode {
    match apply::run(layout) {
        Ok(outcome) => {
            let mut message = format!("Switched to {layout}.\n");
            if !outcome.skipped.is_empty() {
                let config = Config::load(&paths::config_file()).ok().flatten();
                let labels: Vec<String> = outcome
                    .skipped
                    .iter()
                    .map(|id| {
                        config
                            .as_ref()
                            .and_then(|config| config.monitor(id))
                            .map_or_else(|| id.to_string(), |monitor| monitor.label.clone())
                    })
                    .collect();
                let _ = writeln!(
                    message,
                    "Not connected, so left out: {}.",
                    labels.join(", ")
                );
            }
            print(&message);
            ExitCode::SUCCESS
        }
        Err(error) => {
            print(&format!("Could not switch to {layout}: {error}.\n"));
            ExitCode::FAILURE
        }
    }
}

fn list() -> ExitCode {
    let config = match Config::load(&paths::config_file()) {
        Ok(Some(config)) => config,
        Ok(None) => {
            print("No layouts yet. Run \"deskflip settings\" to create one.\n");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            print(&format!("{error}\n"));
            return ExitCode::FAILURE;
        }
    };
    let snapshot = WindowsDisplay.snapshot().unwrap_or_default();
    let state = State::load(&paths::state_file());
    print(&list_report(&config, &state, &snapshot));
    ExitCode::SUCCESS
}

fn list_report(config: &Config, state: &State, snapshot: &Snapshot) -> String {
    if config.layouts.is_empty() {
        return "No layouts yet. Run \"deskflip settings\" to create one.\n".to_owned();
    }
    let current = current_layout(config, state, snapshot).map(|layout| layout.name.as_str());
    let problems = validate(config);

    let mut out = String::new();
    for layout in &config.layouts {
        let marker = if current == Some(layout.name.as_str()) {
            '*'
        } else {
            ' '
        };
        // Writing to a String cannot fail.
        let _ = write!(out, "{marker} {}", layout.name);
        if let Some(text) = &layout.hotkey {
            let _ = write!(out, "  {}", hotkey::pretty(text));
        }
        out.push('\n');
        for problem in &problems {
            if problem.layout.as_deref() == Some(layout.name.as_str()) {
                let _ = writeln!(out, "      ! {}", problem.message(config));
            }
        }
    }
    for problem in problems.iter().filter(|problem| problem.layout.is_none()) {
        let _ = writeln!(out, "! {}", problem.message(config));
    }
    out
}

/// Writes to stdout, ignoring failure: there may be no console to write to.
fn print(text: &str) {
    let mut stdout = io::stdout();
    let _ = stdout.write_all(text.as_bytes());
    let _ = stdout.flush();
}

/// Everything Deskflip can learn about the connected monitors. A failure to read one detail is
/// shown in place so the rest of the report is still useful.
fn monitors_report(display: &WindowsDisplay) -> String {
    let snapshot = match display.snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => return format!("Could not list monitors: {error}\n"),
    };

    let mut out = String::new();
    for monitor in &snapshot.monitors {
        // Writing to a String cannot fail.
        let _ = writeln!(out, "{}", monitor.model);
        let _ = writeln!(out, "  id:       {}", monitor.id);
        let _ = writeln!(
            out,
            "  desktop:  {}",
            match (monitor.active, monitor.primary) {
                (true, true) => "in use, main display",
                (true, false) => "in use",
                (false, _) => "not in use",
            }
        );
        if let Some(rect) = monitor.rect {
            let _ = writeln!(
                out,
                "  position: {}x{} at ({}, {})",
                rect.width, rect.height, rect.x, rect.y
            );
        }
        if !monitor.active {
            let _ = writeln!(out, "  inputs:   unknown while the monitor is not in use");
            continue;
        }
        match display.read_input(&monitor.id) {
            Ok(input) => {
                let _ = writeln!(out, "  reports:  {} ({:#04X})", input_name(input), input.0);
            }
            Err(error) => {
                let _ = writeln!(out, "  reports:  {error}");
            }
        }
        match display.read_capabilities(&monitor.id) {
            Ok(capabilities) => {
                let names: Vec<String> = parse_input_codes(&capabilities)
                    .into_iter()
                    .map(|input| format!("{} ({:#04X})", input_name(input), input.0))
                    .collect();
                let _ = writeln!(out, "  inputs:   {}", names.join(", "));
                let _ = writeln!(out, "  raw:      {capabilities}");
            }
            Err(error) => {
                let _ = writeln!(out, "  inputs:   {error}");
            }
        }
    }
    if snapshot.monitors.is_empty() {
        out.push_str("No monitors found.\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{desk, snap};

    #[test]
    fn list_marks_the_current_layout_and_shows_hotkeys() {
        let at_work = snap(&[("L", false, false), ("R", true, true)]);
        let state = State {
            last_applied: Some("work".to_owned()),
        };

        let report = list_report(&desk(), &state, &at_work);

        assert_eq!(report, "  home  Ctrl+Alt+1\n* work  Ctrl+Alt+2\n");
    }

    #[test]
    fn list_shows_problems_under_the_layout() {
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;
        config.layouts[0].hotkey = None;
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let report = list_report(&config, &State::default(), &both);

        assert_eq!(
            report,
            "  home\n  work  Ctrl+Alt+2\n      ! L cannot be handed to another device until you confirm which input this PC is plugged into (Monitors page).\n"
        );
    }

    #[test]
    fn list_shows_general_problems_last() {
        let mut config = desk();
        config.cycle_hotkey = Some("M".to_owned());
        let both = snap(&[("L", true, false), ("R", true, true)]);

        let report = list_report(&config, &State::default(), &both);

        assert!(
            report.ends_with(
                "! \"M\" is not a usable hotkey. Use Ctrl, Alt or Win with another key.\n"
            )
        );
    }

    #[test]
    fn list_without_layouts_says_so() {
        let mut config = desk();
        config.layouts.clear();

        let report = list_report(&config, &State::default(), &snap(&[("R", true, true)]));

        assert_eq!(
            report,
            "No layouts yet. Run \"deskflip settings\" to create one.\n"
        );
    }
}
