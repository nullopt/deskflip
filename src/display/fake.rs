//! An in-memory [`Display`] for tests: it behaves like well-behaved hardware unless told to fail.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use super::{Display, DisplayError, MonitorId, Snapshot};
use crate::input::InputCode;

pub struct FakeDisplay {
    pub state: Snapshot,
    /// The input each monitor was last told to show.
    pub inputs: HashMap<MonitorId, InputCode>,
    /// Calls named here (`"extend_all"`, `"set_active"`, `"set_input"`, `"restore"`) fail.
    pub fail_on: Vec<&'static str>,
    /// Makes `set_active` report success without changing anything, like a driver that
    /// silently refuses.
    pub ignore_set_active: bool,
    calls: RefCell<Vec<String>>,
}

impl FakeDisplay {
    pub fn new(state: Snapshot) -> Self {
        Self {
            state,
            inputs: HashMap::new(),
            fail_on: Vec::new(),
            ignore_set_active: false,
            calls: RefCell::new(Vec::new()),
        }
    }

    /// Every call made so far, in order, e.g. `"set_input(L)"`.
    pub fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }

    fn record(&self, name: &'static str, detail: &str) -> Result<(), DisplayError> {
        self.calls.borrow_mut().push(if detail.is_empty() {
            name.to_owned()
        } else {
            format!("{name}({detail})")
        });
        if self.fail_on.contains(&name) {
            return Err(DisplayError::Os {
                call: name,
                code: 1,
            });
        }
        Ok(())
    }
}

impl Display for FakeDisplay {
    type Saved = Snapshot;

    fn snapshot(&self) -> Result<Snapshot, DisplayError> {
        self.record("snapshot", "")?;
        Ok(self.state.clone())
    }

    fn extend_all(&mut self) -> Result<(), DisplayError> {
        self.record("extend_all", "")?;
        let has_primary = self.state.primary().is_some();
        for (index, monitor) in self.state.monitors.iter_mut().enumerate() {
            monitor.active = true;
            if !has_primary {
                monitor.primary = index == 0;
            }
        }
        Ok(())
    }

    fn set_active(
        &mut self,
        keep: &BTreeSet<MonitorId>,
        primary: &MonitorId,
    ) -> Result<(), DisplayError> {
        self.record("set_active", "")?;
        assert!(keep.contains(primary), "primary must be kept");
        assert!(
            keep.is_subset(&self.state.active_ids()),
            "set_active can only keep monitors that are already active"
        );
        if !self.ignore_set_active {
            for monitor in &mut self.state.monitors {
                monitor.active = keep.contains(&monitor.id);
                monitor.primary = &monitor.id == primary;
            }
        }
        Ok(())
    }

    fn save(&self) -> Result<Snapshot, DisplayError> {
        self.record("save", "")?;
        Ok(self.state.clone())
    }

    fn restore(&mut self, saved: &Snapshot) -> Result<(), DisplayError> {
        self.record("restore", "")?;
        self.state = saved.clone();
        Ok(())
    }

    fn set_input(&mut self, monitor: &MonitorId, input: InputCode) -> Result<(), DisplayError> {
        self.record("set_input", monitor.as_str())?;
        match self.state.get(monitor) {
            None => Err(DisplayError::NotConnected),
            Some(connected) if !connected.active => Err(DisplayError::NotActive),
            Some(_) => {
                self.inputs.insert(monitor.clone(), input);
                Ok(())
            }
        }
    }
}
