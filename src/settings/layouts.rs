//! The Layouts page: what each monitor shows in one layout, led by a picture of the desk.

use rune::tok::{TextStyle, radius, space, type_};
use rune::{Color, El, col, el, row, text};
use rune_kit::{ButtonVariant, Kit, ShortcutRecorder, TextInput};

use super::present::{self, DeskRect, RoleChoice, desk_rects, layout_summary, role_choice};
use super::ui::{self, Dropdown, Paint};
use super::{HotkeyTarget, Page, Settings, in_background};
use crate::apply;
use crate::config::{Layout, LayoutMonitor, MonitorConfig, Role};
use crate::display::MonitorId;
use crate::input::input_name;

const FIELD_WIDTH: f32 = 220.0;
/// The size of the desk picture, in points.
const DESK: (f32, f32) = (560.0, 190.0);
const MONITOR_NUMBER: TextStyle = TextStyle::new(26.0, 700.0);

impl Settings {
    /// The layout on screen, by its place in the draft.
    fn current_layout(&self) -> Option<usize> {
        match self.page {
            Page::Layout(index) if index < self.model.draft.layouts.len() => Some(index),
            _ => None,
        }
    }

    /// The monitor the page is editing: the one picked in the desk picture, else the first.
    fn edited_monitor(&self) -> Option<MonitorId> {
        let monitors = &self.model.draft.monitors;
        self.selected_monitor
            .as_ref()
            .filter(|id| monitors.iter().any(|monitor| &&monitor.id == id))
            .or_else(|| monitors.first().map(|monitor| &monitor.id))
            .cloned()
    }

    pub fn select_monitor(&mut self, index: usize) {
        self.selected_monitor = self
            .model
            .draft
            .monitors
            .get(index)
            .map(|monitor| monitor.id.clone());
        self.open_select = None;
    }

    /// Gives the edited monitor a role in the layout on screen.
    pub fn set_role(&mut self, choice: RoleChoice) {
        if let (Some(layout), Some(id)) = (self.current_layout(), self.edited_monitor()) {
            present::set_role(&mut self.model.draft, layout, &id, choice);
        }
    }

    /// Picks the input the other device is on, by its name in the dropdown.
    pub fn set_other_input(&mut self, name: &str) {
        let (Some(layout), Some(id)) = (self.current_layout(), self.edited_monitor()) else {
            return;
        };
        let chosen = present::other_inputs(&self.model.draft, &id)
            .into_iter()
            .find(|&input| input_name(input) == name);
        let entry = self.model.draft.layouts[layout]
            .monitors
            .iter_mut()
            .find(|entry| entry.id == id);
        if let (Some(input), Some(entry)) = (chosen, entry)
            && matches!(entry.role, Role::Other { .. })
        {
            entry.role = Role::Other { input };
        }
    }

    /// Makes the edited monitor the main one, or stops it being so. Only a monitor this PC
    /// uses can be the main one.
    pub fn toggle_primary(&mut self) {
        let (Some(index), Some(id)) = (self.current_layout(), self.edited_monitor()) else {
            return;
        };
        let layout = &mut self.model.draft.layouts[index];
        if layout.primary.as_ref() == Some(&id) {
            layout.primary = None;
        } else if role_choice(layout, &id) == RoleChoice::ThisPc {
            layout.primary = Some(id);
        }
    }

    /// `base`, or `base` with the first number that makes it a name no layout has.
    fn unused_layout_name(&self, base: &str) -> String {
        let layouts = &self.model.draft.layouts;
        let taken = |name: &str| layouts.iter().any(|layout| layout.name == name);
        std::iter::once(base.to_owned())
            .chain((2..).map(|number| format!("{base} {number}")))
            .find(|name| !taken(name))
            .unwrap_or_default()
    }

    pub fn add_layout(&mut self) {
        self.add_layout_named("New layout");
    }

    /// Adds a layout that hands the first monitor that can be handed over to its first other
    /// input. With no such monitor yet, shows the page where one is confirmed.
    pub fn create_handover_layout(&mut self) {
        let draft = &self.model.draft;
        let shareable = draft
            .monitors
            .iter()
            .find(|monitor| present::another_device_blocked(draft, &monitor.id).is_none())
            .map(|monitor| monitor.id.clone());
        let Some(id) = shareable else {
            self.go_to(Page::Monitors);
            return;
        };
        self.add_layout_named("Other device");
        self.selected_monitor = Some(id);
        self.set_role(RoleChoice::AnotherDevice);
    }

    fn add_layout_named(&mut self, base: &str) {
        let name = self.unused_layout_name(base);
        let monitors = self
            .model
            .draft
            .monitors
            .iter()
            .map(|monitor| LayoutMonitor {
                id: monitor.id.clone(),
                role: Role::Pc,
            })
            .collect();
        self.model.draft.layouts.push(Layout {
            name,
            hotkey: None,
            primary: None,
            monitors,
        });
        self.go_to(Page::Layout(self.model.draft.layouts.len() - 1));
    }

    /// Deletes the layout on screen, on the second press: the first one only asks.
    pub fn delete_layout(&mut self) {
        let Some(index) = self.current_layout() else {
            return;
        };
        if !self.confirm_delete {
            self.confirm_delete = true;
            return;
        }
        self.model.draft.layouts.remove(index);
        let remaining = self.model.draft.layouts.len();
        self.go_to(if remaining == 0 {
            Page::Monitors
        } else {
            Page::Layout(index.min(remaining - 1))
        });
    }

    /// The name field was typed into.
    pub fn layout_name_changed(&mut self) {
        if let Some(index) = self.current_layout() {
            self.model.draft.layouts[index].name = self.edits.layout_name.text.clone();
        }
    }

    /// Saves, then switches the real monitors to the layout on screen.
    pub fn apply_now(&mut self) {
        let Some(index) = self.current_layout() else {
            return;
        };
        if self.busy() || (self.model.is_dirty() && !self.save()) {
            return;
        }
        let name = self.model.draft.layouts[index].name.clone();
        self.applying = true;
        self.apply_message = None;
        in_background(
            move || {
                apply::run(&name)
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            },
            |s, result, tx| {
                if s.finish_apply(result) {
                    tx.quit();
                }
            },
        );
    }

    /// Records how the layout change ended. Returns whether the app should now quit, which is
    /// the case when the window was closed while the change was under way.
    pub fn finish_apply(&mut self, result: Result<(), String>) -> bool {
        self.applying = false;
        self.apply_message = result
            .err()
            .map(|error| format!("Could not apply: {error}"));
        if !self.quit_when_idle {
            self.start_detect();
        }
        self.quit_when_idle
    }
}

pub fn view(p: Paint, k: &Kit, s: &Settings, index: usize, layer: &mut Vec<El>) -> El {
    let Some(layout) = s.model.draft.layouts.get(index) else {
        return ui::note(p, "This layout no longer exists.");
    };
    let edited = s.edited_monitor();
    let title = if layout.name.is_empty() {
        "Unnamed layout"
    } else {
        &layout.name
    };
    let mut page = col().gap(space::S6).child(ui::page_header(
        p,
        title,
        vec![ui::button(
            k,
            "apply-now",
            if s.applying {
                "Applying\u{2026}"
            } else {
                "Apply now"
            },
            ButtonVariant::Contrast,
            !s.busy() && s.problems().is_empty(),
            |s, _| s.apply_now(),
        )],
    ));
    if let Some(message) = &s.apply_message {
        page = page.child(ui::notice(p, "apply-message", message));
    }
    page = page.child(desk(p, k, s, layout, edited.as_ref()));
    let monitor = edited.as_ref().and_then(|id| s.model.draft.monitor(id));
    if let Some(monitor) = monitor {
        page = page.child(monitor_section(p, k, s, layout, monitor, layer));
    }
    page.child(layout_section(p, k, s, index))
}

/// The picture of the desk: one tile per monitor, placed as on the real desk and coloured by
/// what it shows in this layout.
fn desk(p: Paint, k: &Kit, s: &Settings, layout: &Layout, edited: Option<&MonitorId>) -> El {
    let config = &s.model.draft;
    let mut stage = el().key("desk").size(DESK.0, DESK.1);
    for rect in desk_rects(&s.snapshot, &s.desk_memory, config, DESK) {
        let Some(index) = config
            .monitors
            .iter()
            .position(|monitor| monitor.id == rect.id)
        else {
            continue;
        };
        let monitor = &config.monitors[index];
        let role = layout
            .monitors
            .iter()
            .find(|entry| entry.id == monitor.id)
            .map(|entry| &entry.role);
        stage = stage.child(tile(
            p,
            index,
            monitor,
            role,
            &rect,
            edited == Some(&monitor.id),
        ));
    }
    let picture = if config.monitors.is_empty() {
        row()
            .h(DESK.1)
            .center()
            .child(ui::note(p, "No monitors yet. See the Monitors page."))
    } else {
        row().center().child(stage)
    };
    ui::card(p).child(picture).child(
        row()
            .px(space::S3)
            .pb(space::S2)
            .gap(space::S3)
            .center_y()
            .justify_between()
            .child(
                text(layout_summary(config, layout))
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
                |s, _| s.identify(),
            )),
    )
}

fn tile(
    p: Paint,
    index: usize,
    monitor: &MonitorConfig,
    role: Option<&Role>,
    rect: &DeskRect,
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
    // The selection ring is drawn in a 3 pt margin kept free around every screen, so
    // selecting a monitor moves nothing.
    let ring = 3.0;
    let screen = col()
        .w_full()
        .h_full()
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
    el().key(format!("desk-monitor-{index}"))
        .role("button")
        .label(monitor.label.as_str())
        .focusable(true)
        .absolute(rect.x, rect.y)
        .size(rect.w, rect.h)
        .padding(ring)
        .radius(radius::MD + ring)
        .border(
            2.0,
            if selected {
                p.accent
            } else {
                Color::TRANSPARENT
            },
        )
        .on_press(move |s: &mut Settings| s.select_monitor(index))
        .child(screen)
}

fn monitor_section(
    p: Paint,
    k: &Kit,
    s: &Settings,
    layout: &Layout,
    monitor: &MonitorConfig,
    layer: &mut Vec<El>,
) -> El {
    let config = &s.model.draft;
    let choice = role_choice(layout, &monitor.id);
    let blocked = present::another_device_blocked(config, &monitor.id);
    let labels = RoleChoice::ALL.map(RoleChoice::label);
    let mut rows = vec![ui::setting_row(
        p,
        "shows",
        "In this layout it shows",
        ui::segmented(
            p,
            "role",
            &labels,
            RoleChoice::ALL.iter().position(|&each| each == choice),
            &[false, blocked.is_some(), false, false],
            104.0,
            |s, index| s.set_role(RoleChoice::ALL[index]),
        ),
    )];
    if let Some(reason) = blocked {
        let mut row = ui::note_row(p, "role-blocked", reason).gap(space::S3);
        if !monitor.pc_input_confirmed {
            row = row.child(ui::button(
                k,
                "confirm-now",
                "Confirm now",
                ButtonVariant::Neutral,
                true,
                |s, _| s.go_to(Page::Monitors),
            ));
        }
        rows.push(row);
    }
    match choice {
        RoleChoice::AnotherDevice => {
            let names: Vec<String> = present::other_inputs(config, &monitor.id)
                .into_iter()
                .map(input_name)
                .collect();
            let current = layout
                .monitors
                .iter()
                .find(|entry| entry.id == monitor.id)
                .and_then(|entry| match entry.role {
                    Role::Other { input } => Some(input_name(input)),
                    _ => None,
                });
            rows.push(ui::setting_row(
                p,
                "input",
                "The other device is plugged into",
                ui::dropdown(
                    k,
                    s,
                    Dropdown {
                        key: "other-input".to_owned(),
                        label: "The other device is plugged into",
                        options: &names,
                        current: current.as_deref(),
                        width: FIELD_WIDTH,
                    },
                    |s, name| s.set_other_input(name),
                    layer,
                ),
            ));
        }
        RoleChoice::ThisPc => rows.push(ui::setting_row_note(
            p,
            "main",
            "Main monitor",
            "The taskbar and new windows appear on the main monitor.",
            ui::toggle(
                p,
                "main-monitor",
                layout.primary.as_ref() == Some(&monitor.id),
                true,
                Settings::toggle_primary,
            ),
        )),
        RoleChoice::Off | RoleChoice::DontChange => {}
    }
    let number = config
        .monitors
        .iter()
        .position(|each| each.id == monitor.id)
        .map_or(0, |index| index + 1);
    let title = format!("Monitor {number} \u{00B7} {}", monitor.label);
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
                .on_edit(|s: &mut Settings| &mut s.edits.layout_name)
                .on_change(|s: &mut Settings, _| s.layout_name_changed())
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
            .error_message(ui::hotkey_refusal)
            .on_change(move |s: &mut Settings, shortcut, _| {
                s.set_hotkey(HotkeyTarget::Layout(index), shortcut)
            })
            .build(k),
        ));
    }
    let stored = s.model.draft.layouts.get(index);
    rows.extend(ui::unshown_hotkey(
        p,
        k,
        "layout-hotkey",
        stored.and_then(|layout| layout.hotkey.as_deref()),
        HotkeyTarget::Layout(index),
    ));
    rows.push(ui::setting_row(
        p,
        "delete",
        "Delete this layout",
        ui::button(
            k,
            "delete-layout",
            if s.confirm_delete {
                "Really delete?"
            } else {
                "Delete"
            },
            ButtonVariant::Danger,
            true,
            |s, _| s.delete_layout(),
        ),
    ));
    ui::section(p, "layout", "Layout", rows)
}
