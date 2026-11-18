// Release builds are windowed so starting from the Start menu shows no console. Debug builds
// keep the console, which makes `cargo run` output visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    deskflip::cli::run()
}
