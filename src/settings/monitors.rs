//! The Monitors page: naming monitors, and establishing which input this PC is plugged into.

use rune::El;
use rune::tok::space;
use rune::{col, row};
use rune_kit::{ButtonVariant, Kit, Select, TextInput};

use super::Settings;
use super::model::input_choices;
use super::ui::{self, Paint};
use crate::config::MonitorConfig;
use crate::input::input_name;

const FIELD_WIDTH: f32 = 220.0;

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
                    |_| {},
                ),
                ui::button(
                    k,
                    "redetect",
                    "Look for monitors again",
                    ButtonVariant::Neutral,
                    !s.detecting,
                    |_| {},
                ),
            ],
        ))
        .child(ui::note(
            p,
            "Deskflip needs to know which input on each monitor this PC is plugged into, so it \
             can always bring the monitor back.",
        ));
    if s.model.draft.monitors.is_empty() {
        page = page.child(ui::card(p).child(ui::note_row(
            p,
            "no-monitors",
            if s.detecting {
                "Asking the monitors what they support\u{2026}"
            } else {
                "No monitors found."
            },
        )));
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
    let connected = s.snapshot.get(&monitor.id);
    let status = match connected {
        Some(found) if found.active => "in use by this PC",
        Some(_) => "connected, not in use by this PC right now",
        None => "not connected",
    };
    let mut rows = vec![ui::setting_row_note(
        p,
        "name",
        "Name",
        &format!("{} \u{00B7} {status}", monitor.model),
        match s.edits.monitor_names.get(index) {
            Some(edit) => rune::el().key(format!("monitor-name-{index}")).child(
                TextInput::new(edit)
                    .label("Name")
                    .width(Some(FIELD_WIDTH))
                    .build(k),
            ),
            None => rune::el(),
        },
    )];

    let choices = input_choices(monitor);
    if choices.is_empty() {
        rows.push(ui::note_row(
            p,
            "no-inputs",
            match connected {
                Some(found) if !found.active => {
                    "Its inputs can only be read while this PC is using it. Switch to a layout \
                     that uses it, then choose \"Look for monitors again\"."
                }
                _ => {
                    "This monitor does not accept input commands (DDC/CI is off or \
                     unsupported). It can still be turned on or off for this PC."
                }
            },
        ));
    } else {
        let names: Vec<String> = choices.iter().copied().map(input_name).collect();
        let current = monitor.pc_input.map(input_name);
        rows.push(ui::setting_row(
            p,
            "pc-input",
            "This PC is plugged into",
            Select::new(format!("pc-input-{index}"), &names)
                .current(current.as_deref())
                .placeholder("Choose\u{2026}")
                .label("This PC is plugged into")
                .width(FIELD_WIDTH)
                .build(k, layer),
        ));
        rows.push(ui::setting_row_note(
            p,
            "confirm",
            "I've checked the label on the port at the back of the monitor",
            if monitor.pc_input_confirmed {
                "Deskflip can hand this monitor to another device and bring it back."
            } else {
                "Until this is switched on, Deskflip will not hand this monitor to another device: \
                 a wrong value here would leave no way to bring it back."
            },
            rune::el()
                .key(format!("confirm-{index}"))
                .opacity(if monitor.pc_input.is_some() { 1.0 } else { 0.4 })
                .child(ui::toggle(p, monitor.pc_input_confirmed)),
        ));
        let others: Vec<String> = choices
            .iter()
            .copied()
            .filter(|&input| Some(input) != monitor.pc_input)
            .map(input_name)
            .collect();
        if !others.is_empty() {
            rows.push(ui::setting_row_note(
                p,
                "test",
                "Try another input",
                "The monitor shows that input, then returns to this PC. If the picture does \
                 not return within 10 seconds, press the input button on the monitor.",
                row()
                    .gap(space::S2)
                    .center_y()
                    .child(
                        Select::new(format!("test-input-{index}"), &others)
                            .current(others.first().map(String::as_str))
                            .label("Input to try")
                            .width(130.0)
                            .build(k, layer),
                    )
                    .child(ui::button(
                        k,
                        &format!("test-{index}"),
                        "Test for 5 seconds",
                        ButtonVariant::Neutral,
                        monitor.pc_input_confirmed,
                        |_| {},
                    )),
            ));
        }
    }
    if connected.is_none() {
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
                |_| {},
            ),
        ));
    }

    let title = format!("Monitor {}", index + 1);
    col()
        .key(format!("monitor-card-{index}"))
        .child(ui::section(p, "monitor", &title, rows))
}
