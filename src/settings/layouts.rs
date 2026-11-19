//! The Layouts page: what each monitor shows in one layout, led by a picture of the desk.

use rune::tok::{TextStyle, radius, space, type_};
use rune::{Color, El, col, el, row, text};
use rune_kit::{ButtonVariant, Kit, Select, ShortcutRecorder, TextInput};

use super::Settings;
use super::model::input_choices;
use super::ui::{self, Paint};
use crate::config::{Layout, MonitorConfig, Role};
use crate::input::input_name;

const FIELD_WIDTH: f32 = 220.0;
const ROLE_LABELS: [&str; 4] = ["This PC", "Another device", "Off", "Don't change"];
const MONITOR_NUMBER: TextStyle = TextStyle::new(26.0, 700.0);

pub fn view(p: Paint, k: &Kit, s: &Settings, index: usize, layer: &mut Vec<El>) -> El {
    let Some(layout) = s.model.draft.layouts.get(index) else {
        return ui::note(p, "This layout no longer exists.");
    };
    let monitors = &s.model.draft.monitors;
    let selected = s
        .selected_monitor
        .as_ref()
        .and_then(|id| monitors.iter().position(|monitor| &monitor.id == id))
        .unwrap_or(0);

    let title = if layout.name.is_empty() {
        "Unnamed layout"
    } else {
        &layout.name
    };
    let mut page = col()
        .gap(space::S6)
        .child(ui::page_header(
            p,
            title,
            vec![ui::button(
                k,
                "apply-now",
                "Apply now",
                ButtonVariant::Contrast,
                true,
                |_| {},
            )],
        ))
        .child(desk(p, k, layout, monitors, selected));
    if let Some(monitor) = monitors.get(selected) {
        page = page.child(monitor_section(p, k, layout, monitor, selected, layer));
    }
    page.child(layout_section(p, k, s, index))
}

fn role_of<'a>(layout: &'a Layout, monitor: &MonitorConfig) -> Option<&'a Role> {
    layout
        .monitors
        .iter()
        .find(|entry| entry.id == monitor.id)
        .map(|entry| &entry.role)
}

/// The picture of the desk: one tile per monitor, coloured by what it shows in this layout.
fn desk(p: Paint, k: &Kit, layout: &Layout, monitors: &[MonitorConfig], selected: usize) -> El {
    let mut tiles = row().key("desk").gap(space::S3).center().h(150.0).w_full();
    for (index, monitor) in monitors.iter().enumerate() {
        tiles = tiles.child(tile(
            p,
            index,
            monitor,
            role_of(layout, monitor),
            index == selected,
        ));
    }
    if monitors.is_empty() {
        tiles = tiles.child(ui::note(p, "No monitors yet. See the Monitors page."));
    }
    ui::card(p).child(tiles).child(
        row()
            .px(space::S3)
            .pb(space::S2)
            .gap(space::S3)
            .center_y()
            .justify_between()
            .child(
                text("")
                    .key("summary")
                    .font(type_::BODY)
                    .color(p.t.text_secondary)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
            )
            .child(ui::button(
                k,
                "identify",
                "Identify",
                ButtonVariant::Neutral,
                true,
                |_| {},
            )),
    )
}

fn tile(
    p: Paint,
    index: usize,
    monitor: &MonitorConfig,
    role: Option<&Role>,
    selected: bool,
) -> El {
    let (fill, ink, status) = match role {
        Some(Role::Pc) => (p.accent, Color::WHITE, "This PC".to_owned()),
        Some(Role::Other { input }) => (p.other, Color::WHITE, input_name(*input)),
        Some(Role::Off) => (Color::TRANSPARENT, p.t.text_secondary, "Off".to_owned()),
        None => (
            Color::TRANSPARENT,
            p.t.text_secondary,
            "Unchanged".to_owned(),
        ),
    };
    let outlined = fill == Color::TRANSPARENT;
    let screen = col()
        .size(168.0, 100.0)
        .center()
        .gap(2.0)
        .radius(radius::MD)
        .fill(fill)
        .border(
            if outlined { 1.5 } else { 0.0 },
            if outlined {
                p.t.text_secondary.alpha(0.5)
            } else {
                Color::TRANSPARENT
            },
        )
        .child(
            text((index + 1).to_string())
                .font(MONITOR_NUMBER)
                .color(ink),
        )
        .child(text(monitor.label.as_str()).font(type_::TITLE).color(ink))
        .child(text(status).font(type_::LABEL).color(ink.alpha(0.85)));
    // The ring sits outside the screen so selecting a monitor does not move anything.
    el().key(format!("desk-monitor-{index}"))
        .role("button")
        .label(monitor.label.as_str())
        .focusable(true)
        .padding(3.0)
        .radius(radius::MD + 3.0)
        .border(
            2.0,
            if selected {
                p.accent
            } else {
                Color::TRANSPARENT
            },
        )
        .child(screen)
}

fn monitor_section(
    p: Paint,
    k: &Kit,
    layout: &Layout,
    monitor: &MonitorConfig,
    index: usize,
    layer: &mut Vec<El>,
) -> El {
    let role = role_of(layout, monitor);
    let choice = match role {
        Some(Role::Pc) => 0,
        Some(Role::Other { .. }) => 1,
        Some(Role::Off) => 2,
        None => 3,
    };
    let blocked = [false, !monitor.pc_input_confirmed, false, false];
    let mut rows = vec![ui::setting_row(
        p,
        "shows",
        "In this layout it shows",
        ui::segmented(
            p,
            "role",
            &ROLE_LABELS,
            Some(choice),
            &blocked,
            104.0,
            |_, _| {},
        ),
    )];
    if !monitor.pc_input_confirmed {
        rows.push(ui::note_row(
            p,
            "role-blocked",
            "To hand this monitor to another device, first confirm on the Monitors page which \
             input this PC is plugged into.",
        ));
    }
    match role {
        Some(Role::Other { input }) => {
            let names: Vec<String> = input_choices(monitor)
                .into_iter()
                .filter(|&choice| Some(choice) != monitor.pc_input)
                .map(input_name)
                .collect();
            let current = input_name(*input);
            rows.push(ui::setting_row(
                p,
                "input",
                "The other device is plugged into",
                Select::new("other-input", &names)
                    .current(Some(&current))
                    .label("The other device is plugged into")
                    .width(FIELD_WIDTH)
                    .build(k, layer),
            ));
        }
        Some(Role::Pc) => rows.push(ui::setting_row_note(
            p,
            "main",
            "Main monitor",
            "The taskbar and new windows appear on the main monitor.",
            el().key("main-monitor")
                .child(ui::toggle(p, layout.primary.as_ref() == Some(&monitor.id))),
        )),
        _ => {}
    }
    let title = format!("Monitor {} \u{00B7} {}", index + 1, monitor.label);
    ui::section(p, "monitor", &title, rows)
}

fn layout_section(p: Paint, k: &Kit, s: &Settings, index: usize) -> El {
    let mut rows = vec![ui::setting_row(
        p,
        "name",
        "Name",
        el().key("layout-name").child(
            TextInput::new(&s.edits.layout_name)
                .label("Name")
                .width(Some(FIELD_WIDTH))
                .build(k),
        ),
    )];
    if let Some(recorder) = s.recorders.layouts.get(index) {
        rows.push(ui::setting_row(
            p,
            "hotkey",
            "Hotkey",
            ShortcutRecorder::new("layout-hotkey", recorder, move |s: &mut Settings| {
                &mut s.recorders.layouts[index]
            })
            .label("Hotkey for this layout")
            .width(FIELD_WIDTH)
            .build(k),
        ));
    }
    rows.push(ui::setting_row(
        p,
        "delete",
        "Delete this layout",
        ui::button(
            k,
            "delete-layout",
            "Delete",
            ButtonVariant::Danger,
            true,
            |_| {},
        ),
    ));
    ui::section(p, "layout", "Layout", rows)
}
