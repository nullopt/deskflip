//! The real [`Display`]: this PC's monitors.

use std::collections::BTreeSet;

use super::topology::{self, SavedConfig};
use super::{Display, DisplayError, MonitorId, Snapshot, ddc};
use crate::input::InputCode;

/// This PC's monitors. Holds no state: every call asks Windows afresh.
#[derive(Debug, Default)]
pub struct WindowsDisplay;

impl WindowsDisplay {
    /// The input a monitor claims to be showing. See [`ddc::read_input`] for why this is only
    /// a hint.
    pub fn read_input(&self, monitor: &MonitorId) -> Result<InputCode, DisplayError> {
        ddc::read_input(&topology::gdi_name(monitor)?)
    }

    pub fn read_capabilities(&self, monitor: &MonitorId) -> Result<String, DisplayError> {
        ddc::read_capabilities(&topology::gdi_name(monitor)?)
    }
}

impl Display for WindowsDisplay {
    type Saved = SavedConfig;

    fn snapshot(&self) -> Result<Snapshot, DisplayError> {
        topology::snapshot()
    }

    fn extend_all(&mut self) -> Result<(), DisplayError> {
        topology::extend_all()
    }

    fn set_active(
        &mut self,
        keep: &BTreeSet<MonitorId>,
        primary: &MonitorId,
    ) -> Result<(), DisplayError> {
        topology::set_active(keep, primary)
    }

    fn save(&self) -> Result<SavedConfig, DisplayError> {
        topology::save()
    }

    fn restore(&mut self, saved: &SavedConfig) -> Result<(), DisplayError> {
        topology::restore(saved)
    }

    fn set_input(&mut self, monitor: &MonitorId, input: InputCode) -> Result<(), DisplayError> {
        ddc::set_input(&topology::gdi_name(monitor)?, input)
    }
}
