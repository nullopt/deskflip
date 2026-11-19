//! The General page: settings that are not about a monitor or a layout.

use rune::El;
use rune::tok::{space, type_};
use rune::{col, text};
use rune_kit::{ButtonVariant, Kit, ShortcutRecorder};

use super::ui::{self, Paint};
use super::{HotkeyTarget, Settings};

pub fn view(p: Paint, k: &Kit, s: &Settings) -> El {
    let startup = ui::section(
        p,
        "startup",
        "Startup",
        vec![ui::setting_row_note(
            p,
            "start-with-windows",
            "Start with Windows",
            s.autostart_error
                .as_deref()
                .unwrap_or("Deskflip waits in the system tray, next to the clock."),
            ui::toggle(
                p,
                "start-with-windows-switch",
                s.start_with_windows,
                true,
                Settings::toggle_start_with_windows,
            ),
        )],
    );
    let mut hotkey_rows = vec![ui::setting_row_note(
        p,
        "cycle",
        "Switch to the next layout",
        "Each layout can also have a hotkey of its own.",
        ShortcutRecorder::new("cycle-hotkey", &s.recorders.cycle, |s: &mut Settings| {
            &mut s.recorders.cycle
        })
        .label("Switch to the next layout")
        .error_message(ui::hotkey_refusal)
        .on_change(|s: &mut Settings, shortcut, _| s.set_hotkey(HotkeyTarget::Cycle, shortcut))
        .build(k),
    )];
    hotkey_rows.extend(ui::unshown_hotkey(
        p,
        k,
        "cycle-hotkey",
        s.model.draft.cycle_hotkey.as_deref(),
        HotkeyTarget::Cycle,
    ));
    let hotkeys = ui::section(p, "hotkeys", "Hotkeys", hotkey_rows);
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
                    |s, _| s.open_config_folder(),
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
