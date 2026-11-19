//! What the settings window is editing, independent of how it is drawn.

use crate::config::{Config, Detected, MonitorConfig, merge_detected, starter};
use crate::display::MonitorId;
use crate::input::InputCode;

/// The settings as saved on disk, and the copy being edited.
pub struct Model {
    /// `None` until the settings have been saved for the first time.
    saved: Option<Config>,
    /// Edits go here and reach the disk only on Save.
    pub draft: Config,
}

impl Model {
    pub fn new(saved: Option<Config>) -> Self {
        let draft = saved.clone().unwrap_or_else(|| starter(&[]));
        Self { saved, draft }
    }

    /// The settings as last saved, if they ever were.
    pub fn saved(&self) -> Option<&Config> {
        self.saved.as_ref()
    }

    pub fn is_first_run(&self) -> bool {
        self.saved.is_none()
    }

    pub fn is_dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.draft)
    }

    /// Folds freshly detected monitors into the draft. On a first run this also builds the
    /// starter layout, so the user never faces an empty window.
    pub fn merge(&mut self, detected: &[Detected]) {
        if self.saved.is_none() && self.draft == starter(&[]) {
            self.draft = starter(detected);
        } else {
            merge_detected(&mut self.draft, detected);
        }
    }

    pub fn mark_saved(&mut self) {
        self.saved = Some(self.draft.clone());
    }

    pub fn discard(&mut self) {
        if let Some(saved) = &self.saved {
            self.draft = saved.clone();
        }
    }

    /// The number shown for a monitor in the window and by Identify: its place in the list.
    pub fn monitor_number(&self, id: &MonitorId) -> Option<usize> {
        self.draft
            .monitors
            .iter()
            .position(|monitor| &monitor.id == id)
            .map(|index| index + 1)
    }

    pub fn forget_monitor(&mut self, id: &MonitorId) {
        self.draft.monitors.retain(|monitor| &monitor.id != id);
        for layout in &mut self.draft.layouts {
            layout.monitors.retain(|entry| &entry.id != id);
            if layout.primary.as_ref() == Some(id) {
                layout.primary = None;
            }
        }
    }

    /// Changes which input this PC is plugged into. A different value has not been checked by
    /// anyone yet, so the confirmation is withdrawn.
    pub fn set_pc_input(&mut self, id: &MonitorId, input: InputCode) {
        if let Some(monitor) = self
            .draft
            .monitors
            .iter_mut()
            .find(|monitor| &monitor.id == id)
            && monitor.pc_input != Some(input)
        {
            monitor.pc_input = Some(input);
            monitor.pc_input_confirmed = false;
        }
    }
}

/// The inputs to offer for a monitor: what it listed, plus the PC's input if that was entered
/// by hand and is not in the list.
pub fn input_choices(monitor: &MonitorConfig) -> Vec<InputCode> {
    let mut choices = monitor.inputs.clone();
    if let Some(input) = monitor.pc_input
        && !choices.contains(&input)
    {
        choices.push(input);
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Role;
    use crate::testing::{DP, HDMI1, HDMI2, desk, id};

    fn detected(name: &str) -> Detected {
        Detected {
            id: id(name),
            model: "MODEL".to_owned(),
            current_input: Some(DP),
            inputs: vec![HDMI1, DP],
        }
    }

    #[test]
    fn dirty_tracks_draft_changes() {
        let mut model = Model::new(Some(desk()));
        assert!(!model.is_dirty());

        model.draft.monitors[0].label = "Left".to_owned();
        assert!(model.is_dirty());

        model.draft.monitors[0].label = "L".to_owned();
        assert!(!model.is_dirty());
    }

    #[test]
    fn first_run_is_dirty_until_saved() {
        let mut model = Model::new(None);
        model.merge(&[detected("A")]);

        assert!(model.is_first_run());
        assert!(model.is_dirty());
        assert_eq!(model.draft.layouts[0].name, "All screens on this PC");

        model.mark_saved();

        assert!(!model.is_dirty());
        assert!(!model.is_first_run());
    }

    #[test]
    fn merging_a_new_monitor_into_saved_settings_makes_them_dirty() {
        let mut model = Model::new(Some(desk()));

        model.merge(&[detected("NEW")]);

        assert!(model.is_dirty());
        assert_eq!(model.draft.layouts.len(), 2);
        assert!(model.draft.monitor(&id("NEW")).is_some());
    }

    #[test]
    fn discard_returns_to_saved() {
        let mut model = Model::new(Some(desk()));
        model.draft.layouts.clear();

        model.discard();

        assert_eq!(model.draft, desk());
        assert!(!model.is_dirty());
    }

    #[test]
    fn monitor_number_is_config_order() {
        let model = Model::new(Some(desk()));

        assert_eq!(model.monitor_number(&id("L")), Some(1));
        assert_eq!(model.monitor_number(&id("r")), Some(2));
        assert_eq!(model.monitor_number(&id("X")), None);
    }

    #[test]
    fn forgetting_a_monitor_removes_it_from_layouts_too() {
        let mut model = Model::new(Some(desk()));
        model.draft.layouts[0].primary = Some(id("L"));

        model.forget_monitor(&id("L"));

        assert!(model.draft.monitor(&id("L")).is_none());
        for layout in &model.draft.layouts {
            assert!(layout.monitors.iter().all(|entry| entry.id != id("L")));
        }
        assert_eq!(model.draft.layouts[0].primary, None);
        assert_eq!(model.draft.layouts[1].monitors[0].role, Role::Pc);
    }

    #[test]
    fn changing_the_pc_input_withdraws_the_confirmation() {
        let mut model = Model::new(Some(desk()));

        model.set_pc_input(&id("L"), HDMI2);

        let left = model.draft.monitor(&id("L")).unwrap();
        assert_eq!(left.pc_input, Some(HDMI2));
        assert!(!left.pc_input_confirmed);
    }

    #[test]
    fn setting_the_same_pc_input_keeps_the_confirmation() {
        let mut model = Model::new(Some(desk()));

        model.set_pc_input(&id("L"), DP);

        assert!(model.draft.monitor(&id("L")).unwrap().pc_input_confirmed);
    }

    #[test]
    fn input_choices_include_an_unlisted_pc_input() {
        let mut config = desk();
        config.monitors[0].pc_input = Some(InputCode(0x1B));

        assert_eq!(
            input_choices(&config.monitors[0]),
            [HDMI1, HDMI2, DP, InputCode(0x1B)]
        );
        assert_eq!(input_choices(&config.monitors[1]), [HDMI1, HDMI2, DP]);
    }
}
