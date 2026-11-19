//! Deskflip: named monitor layouts for Windows.
//!
//! A layout says, for each monitor, which input it should show and whether this PC's desktop
//! uses it. Applying a layout does both in one step.

#![deny(unsafe_code)]

pub mod apply;
pub mod autostart;
pub mod cli;
pub mod config;
pub mod display;
pub mod hotkey;
pub mod input;
pub mod logger;
pub mod notify;
pub mod paths;
pub mod plan;
pub mod settings;
pub mod state;
pub mod tray;
pub mod validate;
pub mod win;

#[cfg(test)]
mod testing;
