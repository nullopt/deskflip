//! The Monitors page: naming monitors, and establishing which input this PC is plugged into.

use std::thread;
use std::time::Duration;

use rune::El;
use rune::tok::space;
use rune::{col, el, row};
use rune_kit::{ButtonVariant, Kit, TextInput};

use super::model::input_choices;
use super::present::{self, MonitorStatus, monitor_status};
use super::ui::{self, Dropdown, Paint};
use super::{Settings, in_background};
use crate::config::{Detected, MonitorConfig};
use crate::display::{Display, DisplayError, Snapshot, WindowsDisplay};
use crate::input::{InputCode, input_name, parse_input_codes};
use crate::paths;
use crate::state::State;

const FIELD_WIDTH: f32 = 220.0;

/// How long a test shows the other input before switching back.
pub const TEST_DURATION: Duration = Duration::from_secs(5);

/// The "Try another input" test. Only one runs at a time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TestState {
    #[default]
    Idle,
    /// A monitor is showing another input and will be switched back.
    Running,
    /// How the last test ended, shown until the next one starts.
    Finished(Result<(), String>),
}

/// Asks Windows which monitors are connected and asks each active one about its inputs.
/// Slow (monitors take a second or more to answer), so it runs off the UI thread.
pub fn detect() -> (Snapshot, Vec<Detected>) {
    let display = WindowsDisplay;
    let snapshot = display.snapshot().unwrap_or_default();
    let detected = snapshot
        .monitors
        .iter()
        .map(|monitor| {
            // A monitor that is not on the desktop cannot be asked anything.
            let (current_input, inputs) = if monitor.active {
                (
                    display.read_input(&monitor.id).ok(),
                    display
                        .read_capabilities(&monitor.id)
                        .map(|capabilities| parse_input_codes(&capabilities))
                        .unwrap_or_default(),
                )
            } else {
                (None, Vec::new())
            };
            Detected {
                id: monitor.id.clone(),
                model: monitor.model.clone(),
                current_input,
                inputs,
            }
        })
        .collect();
    (snapshot, detected)
}

/// Shows `target` for `hold`, then returns to `pc_input`. The return is attempted even when
/// the switch away reported failure: a monitor can obey a command it claims to have rejected.
pub fn run_input_test(
    mut set_input: impl FnMut(InputCode) -> Result<(), DisplayError>,
    target: InputCode,
    pc_input: InputCode,
    hold: Duration,
) -> Result<(), String> {
    let away = set_input(target);
    if away.is_ok() {
        thread::sleep(hold);
    }
    let back = set_input(pc_input);
    match (away, back) {
        (_, Err(error)) => Err(format!("Could not switch back: {error}")),
        (Err(error), Ok(())) => Err(error.to_string()),
        (Ok(()), Ok(())) => Ok(()),
    }
}

/// Why a test cannot start now, if it cannot.
pub fn test_blocked(
    monitor: &MonitorConfig,
    status: MonitorStatus,
    test: &TestState,
) -> Option<&'static str> {
    if !monitor.pc_input_confirmed {
        Some("Switch on the confirmation above first.")
    } else if status != MonitorStatus::InUse {
        Some("This PC is not using this monitor right now.")
    } else if *test == TestState::Running {
        Some("A test is already running.")
    } else {
        None
    }
}

/// An input code typed by hand: a whole number from 0 to 255.
pub fn parse_code(text: &str) -> Option<u8> {
    text.trim().parse().ok()
}

impl Settings {
    pub fn start_detect(&mut self) {
        self.detecting = in_background(detect, |s, (snapshot, detected), _| {
            s.finish_detect(snapshot, detected);
        });
    }

    pub fn finish_detect(&mut self, snapshot: Snapshot, detected: Vec<Detected>) {
        if self.desk_memory.remember(&snapshot) && self.notify_tray {
            // Another process may have written the file since it was read: update a fresh
            // copy rather than write this one back.
            let path = paths::state_file();
            let mut state = State::load(&path);
            state.remember(&snapshot);
            if let Err(error) = state.save(&path) {
                log::warn!("could not save state: {error}");
            }
        }
        self.snapshot = snapshot;
        self.model.merge(&detected);
        self.detecting = false;
        self.sync_fields();
    }

    /// The name field of monitor `index` was typed into.
    pub fn monitor_name_changed(&mut self, index: usize) {
        if let (Some(monitor), Some(edit)) = (
            self.model.draft.monitors.get_mut(index),
            self.edits.monitor_names.get(index),
        ) {
            monitor.label = edit.text.clone();
        }
    }

    /// Records the input this PC is plugged into, chosen by its name in the dropdown.
    pub fn set_pc_input(&mut self, index: usize, name: &str) {
        let Some(monitor) = self.model.draft.monitors.get(index) else {
            return;
        };
        let chosen = input_choices(monitor)
            .into_iter()
            .find(|&input| input_name(input) == name);
        if let Some(input) = chosen {
            let id = monitor.id.clone();
            self.model.set_pc_input(&id, input);
        }
    }

    pub fn toggle_confirmed(&mut self, index: usize) {
        if let Some(monitor) = self.model.draft.monitors.get_mut(index)
            && monitor.pc_input.is_some()
        {
            monitor.pc_input_confirmed = !monitor.pc_input_confirmed;
        }
    }

    /// Picks the input the test will show, by its name in the dropdown.
    pub fn set_test_choice(&mut self, index: usize, name: &str) {
        let Some(monitor) = self.model.draft.monitors.get(index) else {
            return;
        };
        let chosen = present::other_inputs(&self.model.draft, &monitor.id)
            .into_iter()
            .find(|&input| input_name(input) == name);
        if let Some(input) = chosen {
            self.test_choice.insert(monitor.id.clone(), input);
        }
    }

    /// The input monitor `index`'s test will show: the one picked, else the first other input.
    fn test_target(&self, monitor: &MonitorConfig) -> Option<InputCode> {
        let others = present::other_inputs(&self.model.draft, &monitor.id);
        self.test_choice
            .get(&monitor.id)
            .copied()
            .filter(|input| others.contains(input))
            .or_else(|| others.first().copied())
    }

    pub fn start_test(&mut self, index: usize) {
        let Some(monitor) = self.model.draft.monitors.get(index) else {
            return;
        };
        let status = monitor_status(&self.snapshot, &monitor.id);
        if test_blocked(monitor, status, &self.test).is_some() {
            return;
        }
        let (Some(pc_input), Some(target)) = (monitor.pc_input, self.test_target(monitor)) else {
            return;
        };
        let id = monitor.id.clone();
        self.test = TestState::Running;
        in_background(
            move || {
                let mut display = WindowsDisplay;
                run_input_test(
                    |input| display.set_input(&id, input),
                    target,
                    pc_input,
                    TEST_DURATION,
                )
            },
            |s, result, tx| {
                if s.finish_test(result) {
                    tx.quit();
                }
            },
        );
    }

    /// Records how the test ended. Returns whether the app should now quit, which is the case
    /// when the window was closed while the monitor was away.
    pub fn finish_test(&mut self, result: Result<(), String>) -> bool {
        self.test = TestState::Finished(result);
        self.quit_after_test
    }

    /// Takes the hand-typed code of monitor `index` as the input this PC is plugged into.
    pub fn use_custom_code(&mut self, index: usize) {
        let code = self
            .edits
            .custom_codes
            .get(index)
            .and_then(|edit| parse_code(&edit.text));
        if let (Some(code), Some(monitor)) = (code, self.model.draft.monitors.get(index)) {
            let id = monitor.id.clone();
            self.model.set_pc_input(&id, InputCode(code));
        }
    }

    pub fn forget_monitor(&mut self, index: usize) {
        if let Some(monitor) = self.model.draft.monitors.get(index) {
            let id = monitor.id.clone();
            self.model.forget_monitor(&id);
            self.sync_fields();
        }
    }
}

pub fn view(p: Paint, k: &Kit, s: &Settings, layer: &mut Vec<El>) -> El {
    let mut page = col()
        .gap(space::S6)
        .child(ui::page_header(
            p,
            "Monitors",
            vec![
                ui::button(
                    k,
                    "identify",
                    "Identify",
                    ButtonVariant::Neutral,
                    true,
                    |s, _| s.identify(),
                ),
                ui::button(
                    k,
                    "redetect",
                    "Look for monitors again",
                    ButtonVariant::Neutral,
                    !s.detecting,
                    |s, _| s.start_detect(),
                ),
            ],
        ))
        .child(ui::note(
            p,
            "Deskflip needs to know which input on each monitor this PC is plugged into, so it \
             can always bring the monitor back.",
        ));
    let status = match &s.test {
        _ if s.detecting => Some("Asking the monitors what they support\u{2026}".to_owned()),
        TestState::Idle => None,
        TestState::Running => {
            Some("Testing\u{2026} the monitor returns to this PC in a few seconds.".to_owned())
        }
        TestState::Finished(Ok(())) => {
            Some("Test finished. Did the monitor show the other device and come back?".to_owned())
        }
        TestState::Finished(Err(error)) => Some(format!("The test did not work: {error}")),
    };
    if let Some(status) = status {
        page = page.child(ui::notice(p, "monitors-status", &status));
    }
    if s.model.draft.monitors.is_empty() && !s.detecting {
        page = page.child(ui::card(p).child(ui::note_row(p, "no-monitors", "No monitors found.")));
    }
    for (index, monitor) in s.model.draft.monitors.iter().enumerate() {
        page = page.child(card(p, k, s, index, monitor, layer));
    }
    page
}

fn card(
    p: Paint,
    k: &Kit,
    s: &Settings,
    index: usize,
    monitor: &MonitorConfig,
    layer: &mut Vec<El>,
) -> El {
    let status = monitor_status(&s.snapshot, &monitor.id);
    let mut rows = vec![ui::setting_row_note(
        p,
        "name",
        "Name",
        &format!("{} \u{00B7} {}", monitor.model, status.text()),
        match s.edits.monitor_names.get(index) {
            Some(edit) => el().key(format!("monitor-name-{index}")).child(
                TextInput::new(edit)
                    .label("Name")
                    .width(Some(FIELD_WIDTH))
                    .on_edit(move |s: &mut Settings| &mut s.edits.monitor_names[index])
                    .on_change(move |s: &mut Settings, _| s.monitor_name_changed(index))
                    .build(k),
            ),
            None => el(),
        },
    )];

    let choices = input_choices(monitor);
    if choices.is_empty() {
        rows.push(ui::note_row(
            p,
            "no-inputs",
            if status == MonitorStatus::ConnectedNotInUse {
                "Its inputs can only be read while this PC is using it. Switch to a layout that \
                 uses it, then choose \"Look for monitors again\"."
            } else {
                "This monitor does not accept input commands (DDC/CI is off or unsupported). It \
                 can still be turned on or off for this PC."
            },
        ));
    } else {
        let names: Vec<String> = choices.iter().copied().map(input_name).collect();
        let current = monitor.pc_input.map(input_name);
        rows.push(ui::setting_row(
            p,
            "pc-input",
            "This PC is plugged into",
            ui::dropdown(
                k,
                s,
                Dropdown {
                    key: format!("pc-input-{index}"),
                    label: "This PC is plugged into",
                    options: &names,
                    current: current.as_deref(),
                    width: FIELD_WIDTH,
                },
                move |s, name| s.set_pc_input(index, name),
                layer,
            ),
        ));
        rows.push(ui::setting_row_note(
            p,
            "confirm",
            "I've checked the label on the port at the back of the monitor",
            if monitor.pc_input_confirmed {
                "Deskflip can hand this monitor to another device and bring it back."
            } else {
                "Until this is switched on, Deskflip will not hand this monitor to another \
                 device: a wrong value here would leave no way to bring it back."
            },
            ui::toggle(
                p,
                &format!("confirm-{index}"),
                monitor.pc_input_confirmed,
                monitor.pc_input.is_some(),
                move |s| s.toggle_confirmed(index),
            ),
        ));
        let others: Vec<String> = present::other_inputs(&s.model.draft, &monitor.id)
            .into_iter()
            .map(input_name)
            .collect();
        if !others.is_empty() {
            let target = s.test_target(monitor).map(input_name);
            let blocked = test_blocked(monitor, status, &s.test);
            rows.push(ui::setting_row_note(
                p,
                "test",
                "Try another input",
                blocked.unwrap_or(
                    "The monitor shows that input, then returns to this PC. If the picture does \
                     not return within 10 seconds, press the input button on the monitor.",
                ),
                row()
                    .gap(space::S2)
                    .center_y()
                    .child(ui::dropdown(
                        k,
                        s,
                        Dropdown {
                            key: format!("test-input-{index}"),
                            label: "Input to try",
                            options: &others,
                            current: target.as_deref(),
                            width: 130.0,
                        },
                        move |s, name| s.set_test_choice(index, name),
                        layer,
                    ))
                    .child(ui::button(
                        k,
                        &format!("test-{index}"),
                        "Test for 5 seconds",
                        ButtonVariant::Neutral,
                        blocked.is_none(),
                        move |s, _| s.start_test(index),
                    )),
            ));
        }
    }
    if let Some(edit) = s.edits.custom_codes.get(index) {
        rows.push(ui::setting_row_note(
            p,
            "custom",
            "Input not listed?",
            "Enter the input's code from the monitor's manual (0\u{2013}255).",
            row()
                .gap(space::S2)
                .center_y()
                .child(
                    el().key(format!("custom-code-{index}")).child(
                        TextInput::new(edit)
                            .label("Input code")
                            .placeholder("Code")
                            .width(Some(72.0))
                            .on_edit(move |s: &mut Settings| &mut s.edits.custom_codes[index])
                            .build(k),
                    ),
                )
                .child(ui::button(
                    k,
                    &format!("use-code-{index}"),
                    "This PC is plugged into it",
                    ButtonVariant::Neutral,
                    parse_code(&edit.text).is_some(),
                    move |s, _| s.use_custom_code(index),
                )),
        ));
    }
    if status == MonitorStatus::NotConnected {
        rows.push(ui::setting_row(
            p,
            "forget",
            "This monitor is not connected",
            ui::button(
                k,
                &format!("forget-{index}"),
                "Forget this monitor",
                ButtonVariant::Danger,
                true,
                move |s, _| s.forget_monitor(index),
            ),
        ));
    }

    let title = format!("Monitor {}", index + 1);
    col()
        .key(format!("monitor-card-{index}"))
        .child(ui::section(p, "monitor", &title, rows))
}
