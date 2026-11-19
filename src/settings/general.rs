//! The General page: settings that are not about a monitor or a layout.

use rune::El;
use rune::tok::{space, type_};
use rune::{col, text};
use rune_kit::{ButtonVariant, Kit, ShortcutRecorder};

use super::Settings;
use super::ui::{self, Paint};

pub fn view(p: Paint, k: &Kit, s: &Settings, _layer: &mut Vec<El>) -> El {
    let startup = ui::section(
        p,
        "startup",
        "Startup",
        vec![ui::setting_row_note(
            p,
            "start-with-windows",
            "Start with Windows",
            "Deskflip waits in the system tray, next to the clock.",
            ui::toggle(p, s.start_with_windows),
        )],
    );
    let hotkeys = ui::section(
        p,
        "hotkeys",
        "Hotkeys",
        vec![ui::setting_row_note(
            p,
            "cycle",
            "Switch to the next layout",
            "Each layout can also have a hotkey of its own.",
            ShortcutRecorder::new("cycle-hotkey", &s.recorders.cycle, |s: &mut Settings| {
                &mut s.recorders.cycle
            })
            .label("Switch to the next layout")
            .build(k),
        )],
    );
    let about = ui::section(
        p,
        "about",
        "About",
        vec![
            ui::setting_row(
                p,
                "settings-file",
                "Settings file",
                ui::button(
                    k,
                    "open-folder",
                    "Open folder",
                    ButtonVariant::Neutral,
                    true,
                    |_| {},
                ),
            ),
            ui::setting_row(
                p,
                "version",
                "Version",
                text(env!("CARGO_PKG_VERSION"))
                    .font(type_::BODY)
                    .color(p.t.text_secondary),
            ),
        ],
    );
    col()
        .gap(space::S6)
        .child(ui::page_header(p, "General", Vec::new()))
        .child(startup)
        .child(hotkeys)
        .child(about)
}
