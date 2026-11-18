//! Deskflip: named monitor layouts for Windows.
//!
//! A layout says, for each monitor, which input it should show and whether this PC's desktop
//! uses it. Applying a layout does both in one step.

#![deny(unsafe_code)]

pub mod cli;
pub mod display;
pub mod input;
pub mod logger;
pub mod paths;
pub mod win;
