//! Command-line entry point.

use std::env;
use std::fmt::Write as _;
use std::io::{self, Write as _};
use std::process::ExitCode;

use crate::display::{Display, WindowsDisplay};
use crate::input::{input_name, parse_input_codes};
use crate::win;

const USAGE: &str = "\
Usage: deskflip <command>

Commands:
  monitors   List connected monitors and what they report (useful in bug reports)
";

pub fn run() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    win::attach_parent_console();

    match args.first().map(String::as_str) {
        Some("monitors") => {
            print(&monitors_report(&WindowsDisplay));
            ExitCode::SUCCESS
        }
        _ => {
            print(USAGE);
            ExitCode::from(2)
        }
    }
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
