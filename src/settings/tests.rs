//! Tests of the window as a whole: its state changes, and presses on what it draws.

use std::time::Duration;

use rune::{Env, Runtime, ShortcutError};

use super::monitors::{TestState, parse_code, run_input_test, test_blocked};
use super::present::{MonitorStatus, RoleChoice, role_choice, shortcut_of};
use super::*;
use crate::config::{Config, Detected, Role};
use crate::display::DisplayError;
use crate::input::InputCode;
use crate::testing::temp_dir;

const DP: InputCode = InputCode(0x0F);
const HDMI_1: InputCode = InputCode(0x11);
const HDMI_2: InputCode = InputCode(0x12);

fn window(state: Settings) -> Runtime<Settings> {
    let env = Env {
        system_dark: true,
        scale: 1.0,
    };
    let mut runtime = Runtime::new(state, view, (920.0, 640.0), env);
    runtime.frame(0.0);
    runtime
}

fn press(runtime: &mut Runtime<Settings>, key: &str) {
    let (x, y) = runtime
        .find(key)
        .unwrap_or_else(|| panic!("nothing on screen is called {key}"));
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(1.0 / 60.0);
}

/// A scene whose Save writes into a fresh directory instead of the real settings file.
fn saving_to(name: &str, mut settings: Settings) -> Settings {
    settings.config_path = temp_dir(name).join("config.toml");
    settings
}

/// A scene whose Save cannot work: the path is a directory.
fn unable_to_save(name: &str, mut settings: Settings) -> Settings {
    settings.config_path = temp_dir(name);
    settings
}

fn left() -> MonitorId {
    MonitorId::new("left")
}

// --- the shell ---

#[test]
fn sidebar_lists_layouts_then_pages() {
    let runtime = window(scenes::desk());
    for key in [
        "nav-layout-0",
        "nav-layout-1",
        "nav-monitors",
        "nav-general",
    ] {
        assert!(runtime.find(key).is_some(), "{key} is missing");
    }
}

#[test]
fn pressing_monitors_shows_the_monitors_page() {
    let mut runtime = window(scenes::desk());
    press(&mut runtime, "nav-monitors");
    assert_eq!(runtime.state.page, Page::Monitors);
    assert!(runtime.find("monitor-card-0").is_some());
}

#[test]
fn save_bar_only_when_dirty() {
    assert!(window(scenes::desk()).find("save").is_none());
    assert!(window(scenes::unsaved_problem()).find("save").is_some());
}

#[test]
fn the_setup_checklist_shows_until_setup_is_done() {
    assert!(window(scenes::first_run()).find("guide").is_some());
    assert!(window(scenes::desk()).find("guide").is_none());
}

#[test]
fn load_error_replaces_the_pages() {
    let runtime = window(scenes::load_error());
    assert!(runtime.find("load-error").is_some());
    assert!(runtime.find("nav-monitors").is_none());
}

// --- save, discard, close ---

#[test]
fn save_writes_marks_clean_and_reports_no_error() {
    let mut settings = saving_to("save-writes", scenes::unsaved());
    assert!(settings.save());
    assert!(!settings.model.is_dirty());
    assert_eq!(settings.save_error, None);
    let written = Config::load(&settings.config_path).unwrap().unwrap();
    assert_eq!(written.monitors[0].label, "Left screen");
}

#[test]
fn save_is_refused_while_there_are_problems() {
    let mut settings = saving_to("save-refused", scenes::unsaved_problem());
    assert!(!settings.problems().is_empty());
    assert!(!settings.save());
    assert!(settings.model.is_dirty());
    assert!(!settings.config_path.exists());
}

#[test]
fn failed_save_keeps_the_draft() {
    let mut settings = unable_to_save("save-fails", scenes::unsaved());
    let draft = settings.model.draft.clone();
    assert!(!settings.save());
    assert!(settings.save_error.is_some());
    assert!(settings.model.is_dirty());
    assert_eq!(settings.model.draft, draft);
}

#[test]
fn discard_restores_the_saved_settings_and_fields() {
    let mut settings = scenes::unsaved();
    settings.save_error = Some("earlier".to_owned());
    settings.discard();
    assert!(!settings.model.is_dirty());
    assert_eq!(settings.edits.monitor_names[0].text, "Left");
    assert_eq!(settings.save_error, None);
}

#[test]
fn pressing_save_in_the_bar_saves() {
    let mut runtime = window(saving_to("press-save", scenes::unsaved()));
    press(&mut runtime, "save");
    assert!(!runtime.state.model.is_dirty());
    assert!(runtime.find("save").is_none());
}

#[test]
fn pressing_discard_in_the_bar_discards() {
    let mut runtime = window(scenes::unsaved());
    press(&mut runtime, "discard");
    assert_eq!(runtime.state.model.draft.monitors[0].label, "Left");
}

#[test]
fn closing_while_clean_quits_and_closing_while_dirty_prompts() {
    assert_eq!(close_action(false, false), CloseAction::Quit);
    assert_eq!(close_action(true, false), CloseAction::Prompt);
}

#[test]
fn closing_during_an_input_test_waits_for_it() {
    assert_eq!(close_action(false, true), CloseAction::WaitForTest);
    assert_eq!(close_action(true, true), CloseAction::Prompt);
}

#[test]
fn close_prompt_shows_its_three_choices() {
    let runtime = window(scenes::close_prompt());
    for key in ["close-save", "close-discard", "close-cancel"] {
        assert!(runtime.find(key).is_some(), "{key} is missing");
    }
}

#[test]
fn close_cancel_returns_to_editing() {
    let mut settings = scenes::close_prompt();
    settings.close_cancel();
    assert!(!settings.close_prompt);
    assert!(settings.model.is_dirty());
}

#[test]
fn close_discard_leaves_nothing_unsaved_even_on_a_first_run() {
    let mut settings = scenes::first_run();
    settings.close_prompt = true;
    settings.close_discard();
    assert!(!settings.model.is_dirty());
}

#[test]
fn close_save_reports_whether_the_window_may_go() {
    let mut saved = saving_to("close-save", scenes::close_prompt());
    assert!(saved.close_save());

    let mut stuck = unable_to_save("close-save-fails", scenes::close_prompt());
    assert!(!stuck.close_save());
    assert!(stuck.close_prompt, "the question stays up with the error");
}

// --- hotkeys ---

#[test]
fn recording_a_cycle_hotkey_updates_the_draft() {
    let mut settings = scenes::desk();
    let shortcut = shortcut_of("Ctrl+Alt+N").unwrap();
    assert_eq!(
        settings.set_hotkey(HotkeyTarget::Cycle, Some(&shortcut)),
        Ok(())
    );
    assert_eq!(
        settings.model.draft.cycle_hotkey.as_deref(),
        Some("Ctrl+Alt+N")
    );
}

#[test]
fn recording_a_layout_hotkey_updates_that_layout() {
    let mut settings = scenes::desk();
    let shortcut = shortcut_of("Ctrl+Alt+9").unwrap();
    assert_eq!(
        settings.set_hotkey(HotkeyTarget::Layout(1), Some(&shortcut)),
        Ok(())
    );
    assert_eq!(
        settings.model.draft.layouts[1].hotkey.as_deref(),
        Some("Ctrl+Alt+9")
    );
}

#[test]
fn clearing_a_hotkey_removes_it() {
    let mut settings = scenes::desk();
    assert_eq!(settings.set_hotkey(HotkeyTarget::Layout(0), None), Ok(()));
    assert_eq!(settings.model.draft.layouts[0].hotkey, None);
}

#[test]
fn a_hotkey_already_used_elsewhere_is_refused() {
    let mut settings = scenes::desk();
    let taken = shortcut_of("Ctrl+Alt+1").unwrap();
    assert_eq!(
        settings.set_hotkey(HotkeyTarget::Cycle, Some(&taken)),
        Err(ShortcutError::Conflict)
    );
    assert_eq!(
        settings.model.draft.cycle_hotkey.as_deref(),
        Some("Ctrl+Alt+M")
    );
}

#[test]
fn recording_a_layouts_own_hotkey_again_is_not_a_conflict() {
    let mut settings = scenes::desk();
    let own = shortcut_of("Ctrl+Alt+1").unwrap();
    assert_eq!(
        settings.set_hotkey(HotkeyTarget::Layout(0), Some(&own)),
        Ok(())
    );
}

#[test]
fn a_chord_our_parser_rejects_is_refused() {
    let mut settings = scenes::desk();
    let shift_only = shortcut_of("Shift+A").unwrap();
    assert!(matches!(
        settings.set_hotkey(HotkeyTarget::Cycle, Some(&shift_only)),
        Err(ShortcutError::InvalidAccelerator(_))
    ));
    assert_eq!(
        settings.model.draft.cycle_hotkey.as_deref(),
        Some("Ctrl+Alt+M")
    );
}

// --- the monitors page ---

fn detected(id: &str, inputs: &[InputCode]) -> Detected {
    Detected {
        id: MonitorId::new(id),
        model: "NEW".to_owned(),
        current_input: inputs.first().copied(),
        inputs: inputs.to_vec(),
    }
}

#[test]
fn finish_detect_merges_new_monitors_into_the_draft_and_clears_detecting() {
    let mut settings = scenes::desk();
    settings.detecting = true;
    let snapshot = settings.snapshot.clone();
    settings.finish_detect(snapshot, vec![detected("third", &[DP, HDMI_1])]);
    assert!(!settings.detecting);
    assert_eq!(settings.model.draft.monitors.len(), 3);
    assert_eq!(settings.edits.monitor_names.len(), 3);
}

#[test]
fn editing_a_name_updates_the_draft_label() {
    let mut settings = scenes::desk();
    settings.edits.monitor_names[0].text = "Portrait".to_owned();
    settings.monitor_name_changed(0);
    assert_eq!(settings.model.draft.monitors[0].label, "Portrait");
}

#[test]
fn choosing_a_pc_input_unconfirms_it() {
    let mut settings = scenes::desk();
    settings.set_pc_input(0, "HDMI 1");
    let monitor = &settings.model.draft.monitors[0];
    assert_eq!(monitor.pc_input, Some(HDMI_1));
    assert!(!monitor.pc_input_confirmed);
}

#[test]
fn confirming_needs_a_pc_input() {
    let mut settings = scenes::unconfirmed();
    settings.model.draft.monitors[0].pc_input = None;
    settings.toggle_confirmed(0);
    assert!(!settings.model.draft.monitors[0].pc_input_confirmed);

    settings.model.draft.monitors[0].pc_input = Some(DP);
    settings.toggle_confirmed(0);
    assert!(settings.model.draft.monitors[0].pc_input_confirmed);
}

#[test]
fn pressing_the_confirm_switch_confirms() {
    let mut runtime = window(scenes::unconfirmed());
    press(&mut runtime, "confirm-0");
    assert!(runtime.state.model.draft.monitors[0].pc_input_confirmed);
}

#[test]
fn test_button_needs_confirmation_an_active_monitor_and_no_running_test() {
    let settings = scenes::desk();
    let confirmed = &settings.model.draft.monitors[0];
    let mut unconfirmed = confirmed.clone();
    unconfirmed.pc_input_confirmed = false;
    let running = TestState::Running;

    assert_eq!(
        test_blocked(confirmed, MonitorStatus::InUse, &TestState::Idle),
        None
    );
    assert_eq!(
        test_blocked(&unconfirmed, MonitorStatus::InUse, &TestState::Idle),
        Some("Switch on the confirmation above first.")
    );
    assert_eq!(
        test_blocked(
            confirmed,
            MonitorStatus::ConnectedNotInUse,
            &TestState::Idle
        ),
        Some("This PC is not using this monitor right now.")
    );
    assert_eq!(
        test_blocked(confirmed, MonitorStatus::InUse, &running),
        Some("A test is already running.")
    );
}

#[test]
fn starting_a_test_marks_it_running_with_the_first_other_input() {
    let mut settings = scenes::desk();
    settings.start_test(0);
    assert_eq!(settings.test, TestState::Running);
}

#[test]
fn a_blocked_test_does_not_start() {
    let mut settings = scenes::unconfirmed();
    settings.start_test(0);
    assert_eq!(settings.test, TestState::Idle);
}

#[test]
fn input_test_switches_away_then_back() {
    let mut sent = Vec::new();
    let result = run_input_test(
        |input| {
            sent.push(input);
            Ok(())
        },
        HDMI_2,
        DP,
        Duration::ZERO,
    );
    assert_eq!(result, Ok(()));
    assert_eq!(sent, [HDMI_2, DP]);
}

#[test]
fn input_test_reports_a_failed_switch_back() {
    let mut calls = 0;
    let result = run_input_test(
        |_| {
            calls += 1;
            if calls == 2 {
                Err(DisplayError::DdcRejected)
            } else {
                Ok(())
            }
        },
        HDMI_2,
        DP,
        Duration::ZERO,
    );
    assert!(result.unwrap_err().starts_with("Could not switch back"));
}

#[test]
fn input_test_that_cannot_switch_away_still_asks_for_the_pc_input() {
    let mut sent = Vec::new();
    let result = run_input_test(
        |input| {
            sent.push(input);
            if input == HDMI_2 {
                Err(DisplayError::DdcRejected)
            } else {
                Ok(())
            }
        },
        HDMI_2,
        DP,
        Duration::ZERO,
    );
    assert!(result.is_err());
    assert_eq!(sent, [HDMI_2, DP]);
}

#[test]
fn finishing_a_test_records_the_result() {
    let mut settings = scenes::desk();
    settings.start_test(0);
    assert!(!settings.finish_test(Ok(())));
    assert_eq!(settings.test, TestState::Finished(Ok(())));
}

#[test]
fn finishing_a_test_after_the_window_closed_lets_the_app_quit() {
    let mut settings = scenes::desk();
    settings.start_test(0);
    settings.quit_after_test = true;
    assert!(settings.finish_test(Ok(())));
}

#[test]
fn custom_code_accepts_0_to_255_only() {
    assert_eq!(parse_code("18"), Some(18));
    assert_eq!(parse_code(" 0 "), Some(0));
    assert_eq!(parse_code("255"), Some(255));
    for bad in ["256", "", "x", "-1", "1.5"] {
        assert_eq!(parse_code(bad), None, "{bad:?}");
    }
}

#[test]
fn a_custom_code_becomes_the_pc_input() {
    let mut settings = scenes::desk();
    settings.edits.custom_codes[0].text = "27".to_owned();
    settings.use_custom_code(0);
    assert_eq!(
        settings.model.draft.monitors[0].pc_input,
        Some(InputCode(27))
    );

    settings.edits.custom_codes[1].text = "nope".to_owned();
    settings.use_custom_code(1);
    assert_eq!(settings.model.draft.monitors[1].pc_input, Some(DP));
}

#[test]
fn forget_is_offered_only_for_disconnected_monitors() {
    let connected = window(scenes::on_page(scenes::desk(), Page::Monitors));
    assert!(connected.find("forget-0").is_none());

    let mut settings = scenes::on_page(scenes::desk(), Page::Monitors);
    settings.snapshot.monitors.remove(0);
    let mut runtime = window(settings);
    press(&mut runtime, "forget-0");
    assert_eq!(runtime.state.model.draft.monitors.len(), 1);
    assert_eq!(runtime.state.edits.monitor_names.len(), 1);
}

// --- the layouts page ---

#[test]
fn clicking_a_monitor_in_the_desk_picture_selects_it() {
    let mut runtime = window(scenes::desk());
    press(&mut runtime, "desk-monitor-1");
    assert_eq!(
        runtime.state.selected_monitor,
        Some(MonitorId::new("right"))
    );
}

#[test]
fn choosing_another_device_sets_the_role_and_shows_the_input_row() {
    let mut settings = scenes::desk();
    settings.go_to(Page::Layout(0));
    let mut runtime = window(settings);
    assert!(runtime.find("other-input").is_none());
    press(&mut runtime, "role-1");
    assert_eq!(
        runtime.state.model.draft.layouts[0].monitors[0].role,
        Role::Other { input: HDMI_1 }
    );
    assert!(runtime.find("other-input").is_some());
}

#[test]
fn another_device_is_disabled_for_an_unconfirmed_monitor() {
    let mut settings = scenes::unconfirmed();
    settings.go_to(Page::Layout(0));
    let mut runtime = window(settings);
    assert!(runtime.find("role-blocked").is_some());
    press(&mut runtime, "role-1");
    assert_eq!(
        role_choice(&runtime.state.model.draft.layouts[0], &left()),
        RoleChoice::ThisPc
    );
}

#[test]
fn choosing_the_other_devices_input_by_name() {
    let mut settings = scenes::desk();
    settings.set_other_input("HDMI 1");
    assert_eq!(
        settings.model.draft.layouts[1].monitors[0].role,
        Role::Other { input: HDMI_1 }
    );
}

#[test]
fn add_layout_appends_selects_and_names_uniquely() {
    let mut settings = scenes::desk();
    settings.add_layout();
    assert_eq!(settings.page, Page::Layout(2));
    assert_eq!(settings.model.draft.layouts[2].name, "New layout");
    assert_eq!(settings.edits.layout_name.text, "New layout");
    assert_eq!(settings.recorders.layouts.len(), 3);
    assert!(
        settings.model.draft.layouts[2]
            .monitors
            .iter()
            .all(|entry| entry.role == Role::Pc)
    );

    settings.add_layout();
    assert_eq!(settings.model.draft.layouts[3].name, "New layout 2");
}

#[test]
fn delete_asks_once_before_deleting() {
    let mut settings = scenes::desk();
    settings.delete_layout();
    assert_eq!(settings.model.draft.layouts.len(), 2);
    assert!(settings.confirm_delete);
    settings.delete_layout();
    assert_eq!(settings.model.draft.layouts.len(), 1);
    assert!(!settings.confirm_delete);
}

#[test]
fn delete_layout_moves_to_a_neighbour() {
    let mut settings = scenes::desk();
    settings.delete_layout();
    settings.delete_layout();
    assert_eq!(settings.page, Page::Layout(0));
    assert_eq!(settings.model.draft.layouts[0].name, "Home PC");
    assert_eq!(settings.edits.layout_name.text, "Home PC");
}

#[test]
fn deleting_the_last_layout_shows_monitors() {
    let mut settings = scenes::first_run();
    settings.go_to(Page::Layout(0));
    settings.delete_layout();
    settings.delete_layout();
    assert!(settings.model.draft.layouts.is_empty());
    assert_eq!(settings.page, Page::Monitors);
}

#[test]
fn moving_to_another_page_withdraws_the_delete_question() {
    let mut settings = scenes::desk();
    settings.delete_layout();
    settings.go_to(Page::General);
    assert!(!settings.confirm_delete);
}

#[test]
fn rename_updates_the_draft_and_so_the_sidebar() {
    let mut settings = scenes::desk();
    settings.edits.layout_name.text = "Laptop time".to_owned();
    settings.layout_name_changed();
    assert_eq!(settings.model.draft.layouts[1].name, "Laptop time");
}

#[test]
fn main_monitor_can_only_be_a_pc_monitor() {
    let mut settings = scenes::desk();
    settings.model.draft.layouts[1].primary = None;
    settings.selected_monitor = Some(left());
    settings.toggle_primary();
    assert_eq!(settings.model.draft.layouts[1].primary, None);

    settings.selected_monitor = Some(MonitorId::new("right"));
    settings.toggle_primary();
    assert_eq!(
        settings.model.draft.layouts[1].primary,
        Some(MonitorId::new("right"))
    );
    settings.toggle_primary();
    assert_eq!(settings.model.draft.layouts[1].primary, None);
}

#[test]
fn apply_now_saves_first_then_applies() {
    let mut settings = saving_to("apply-saves", scenes::unsaved());
    settings.apply_now();
    assert!(!settings.model.is_dirty());
    assert!(settings.applying);
}

#[test]
fn apply_now_does_not_apply_when_the_save_fails() {
    let mut settings = unable_to_save("apply-save-fails", scenes::unsaved());
    settings.apply_now();
    assert!(!settings.applying);
    assert!(settings.save_error.is_some());
}

#[test]
fn finishing_an_apply_shows_what_happened() {
    let mut settings = scenes::desk();
    settings.applying = true;
    settings.finish_apply(Err("the monitor did not answer".to_owned()));
    assert!(!settings.applying);
    assert_eq!(
        settings.apply_message.as_deref(),
        Some("Could not apply: the monitor did not answer")
    );
}

// --- general ---

#[test]
fn start_fresh_backs_up_the_bad_file_and_becomes_first_run() {
    let mut settings = saving_to("start-fresh", scenes::load_error());
    std::fs::write(&settings.config_path, "not toml [").unwrap();
    settings.start_fresh();
    assert_eq!(settings.load_error, None);
    assert!(settings.model.is_first_run());
    assert!(!settings.config_path.exists());
    assert!(settings.config_path.with_extension("toml.bad").exists());
}

// --- identify ---

#[test]
fn badges_cover_active_monitors_only_with_their_numbers_and_labels() {
    let mut settings = scenes::desk();
    settings.snapshot.monitors[0].active = false;
    let badges = identify::badges(&settings.snapshot, &settings.model);
    assert_eq!(badges.len(), 1);
    assert_eq!(badges[0].number, 2);
    assert_eq!(badges[0].label, "Right");
}

#[test]
fn badges_use_the_monitor_rectangle_in_pixels() {
    let settings = scenes::desk();
    let badges = identify::badges(&settings.snapshot, &settings.model);
    assert_eq!(badges[0].position, (-2560, 0));
    assert_eq!(badges[0].size, (2560, 1440));
}

#[test]
fn a_monitor_windows_gives_no_place_for_gets_no_badge() {
    let mut settings = scenes::desk();
    settings.snapshot.monitors[1].rect = None;
    assert_eq!(
        identify::badges(&settings.snapshot, &settings.model).len(),
        1
    );
}

#[test]
fn identify_without_a_real_window_shows_nothing() {
    let mut settings = scenes::desk();
    settings.identify();
    assert!(!settings.identifying);
}

// --- the setup checklist ---

fn done(settings: &Settings) -> Vec<bool> {
    present::guide(&settings.model)
        .expect("the checklist is showing")
        .iter()
        .map(|step| step.done)
        .collect()
}

#[test]
fn checklist_starts_with_nothing_done() {
    assert_eq!(done(&scenes::first_run()), [false, false, false, false]);
}

#[test]
fn checklist_ticks_follow_the_draft() {
    let mut settings = scenes::first_run();
    settings.model.draft.monitors[0].label = "Left".to_owned();
    assert_eq!(
        done(&settings),
        [false, false, false, false],
        "one is still unnamed"
    );
    settings.model.draft.monitors[1].label = "Right".to_owned();
    assert_eq!(done(&settings), [true, false, false, false]);

    settings.toggle_confirmed(0);
    assert_eq!(done(&settings), [true, true, false, false]);

    settings.create_handover_layout();
    assert_eq!(done(&settings), [true, true, true, false]);
}

#[test]
fn checklist_goes_once_a_handover_layout_is_saved() {
    let mut settings = saving_to("guide-done", scenes::first_run());
    settings.toggle_confirmed(0);
    settings.create_handover_layout();
    assert!(settings.save());
    assert!(present::guide(&settings.model).is_none());
}

#[test]
fn checklist_stays_away_from_someone_who_set_up_on_off_layouts_only() {
    let mut settings = scenes::desk();
    for layout in &mut settings.model.draft.layouts {
        layout.monitors[0].role = Role::Pc;
    }
    settings.model.mark_saved();
    assert!(present::guide(&settings.model).is_none());
}

#[test]
fn create_handover_layout_hands_the_first_confirmed_monitor_over() {
    let mut settings = scenes::first_run();
    settings.toggle_confirmed(1);
    settings.create_handover_layout();

    assert_eq!(settings.page, Page::Layout(1));
    let layout = &settings.model.draft.layouts[1];
    assert_eq!(layout.name, "Other device");
    assert_eq!(layout.monitors[0].role, Role::Pc);
    assert_eq!(layout.monitors[1].role, Role::Other { input: HDMI_1 });
    assert_eq!(settings.selected_monitor, Some(MonitorId::new("right")));
}

#[test]
fn create_handover_layout_without_a_confirmed_monitor_leads_to_the_monitors_page() {
    let mut settings = scenes::first_run();
    settings.go_to(Page::Layout(0));
    settings.create_handover_layout();
    assert_eq!(settings.model.draft.layouts.len(), 1);
    assert_eq!(settings.page, Page::Monitors);
}

#[test]
fn the_checklist_button_acts_on_the_first_step_not_yet_done() {
    let mut runtime = window(scenes::on_page(scenes::first_run(), Page::General));
    press(&mut runtime, "guide-action");
    assert_eq!(runtime.state.page, Page::Monitors);
    assert!(
        runtime.find("guide-action").is_none(),
        "no button that leads to the page already on screen"
    );
}

#[test]
fn confirm_now_jumps_from_a_blocked_role_to_the_monitors_page() {
    let mut settings = scenes::unconfirmed();
    settings.go_to(Page::Layout(0));
    let mut runtime = window(settings);
    press(&mut runtime, "confirm-now");
    assert_eq!(runtime.state.page, Page::Monitors);
}

#[test]
fn the_identify_tag_sits_in_the_bottom_left_corner_of_its_monitor() {
    for (position, size) in [
        ((-2560, -7), (2560, 1440)),
        ((0, 0), (1920, 1080)),
        ((1920, 0), (3840, 2160)),
    ] {
        let (tag_position, tag_size) = identify::tag_geometry(position, size);
        let (left, top) = (tag_position.0 - position.0, tag_position.1 - position.1);
        let (right, bottom) = (left + tag_size.0 as i32, top + tag_size.1 as i32);
        assert!(left > 0 && top > 0, "inside the monitor: {position:?}");
        assert!(right < size.0 as i32 / 4, "towards the left: {position:?}");
        assert!(
            top > size.1 as i32 * 3 / 4,
            "towards the bottom: {position:?}"
        );
        assert!(
            bottom < size.1 as i32 - 40,
            "clear of the taskbar: {position:?}"
        );
    }
}

#[test]
fn the_identify_tag_grows_with_the_monitor_but_only_so_far() {
    let small = identify::tag_geometry((0, 0), (1280, 720)).1;
    let full_hd = identify::tag_geometry((0, 0), (1920, 1080)).1;
    let huge = identify::tag_geometry((0, 0), (7680, 4320)).1;
    assert_eq!(small, full_hd, "never smaller than on a 1080p monitor");
    assert_eq!(
        huge,
        (full_hd.0 * 2, full_hd.1 * 2),
        "never more than twice that"
    );
}

#[test]
fn looking_again_keeps_what_is_being_typed_when_no_monitor_is_new() {
    let mut settings = scenes::desk();
    settings.edits.monitor_names[0].text = "Lef".to_owned();
    let snapshot = settings.snapshot.clone();
    settings.finish_detect(snapshot, Vec::new());
    assert_eq!(settings.edits.monitor_names[0].text, "Lef");
}

#[test]
fn looking_again_notices_a_monitor_that_came_back_to_this_pc() {
    let mut settings = scenes::desk();
    let mut away = settings.snapshot.clone();
    away.monitors[0].active = false;
    settings.finish_detect(away, Vec::new());
    assert!(
        test_blocked(
            &settings.model.draft.monitors[0],
            present::monitor_status(&settings.snapshot, &left()),
            &settings.test
        )
        .is_some()
    );

    settings.finish_detect(scenes::desk().snapshot, Vec::new());
    assert_eq!(
        present::monitor_status(&settings.snapshot, &left()),
        MonitorStatus::InUse
    );
}

// --- the setup checklist as an accordion ---

fn open_section(settings: &Settings) -> Option<usize> {
    let steps = present::guide(&settings.model).expect("the checklist is showing");
    present::open_step(&steps, settings.guide_choice)
}

#[test]
fn the_first_unfinished_section_is_open() {
    let mut settings = scenes::first_run();
    assert_eq!(open_section(&settings), Some(0));
    settings.model.draft.monitors[0].label = "Left".to_owned();
    settings.model.draft.monitors[1].label = "Right".to_owned();
    assert_eq!(open_section(&settings), Some(1));
}

#[test]
fn clicking_a_section_opens_it_and_clicking_it_again_closes_it() {
    let mut settings = scenes::first_run();
    settings.toggle_guide_step(2);
    assert_eq!(open_section(&settings), Some(2));
    settings.toggle_guide_step(2);
    assert_eq!(open_section(&settings), None);
    settings.toggle_guide_step(0);
    assert_eq!(open_section(&settings), Some(0));
}

#[test]
fn finishing_a_section_collapses_it_and_opens_the_next_even_if_opened_by_hand() {
    let mut settings = scenes::first_run();
    settings.toggle_guide_step(0);
    settings.toggle_guide_step(0);
    settings.toggle_guide_step(0);
    settings.model.draft.monitors[0].label = "Left".to_owned();
    settings.model.draft.monitors[1].label = "Right".to_owned();
    assert_eq!(open_section(&settings), Some(1));
}

#[test]
fn a_finished_section_can_be_reopened_to_read_it() {
    let mut settings = scenes::first_run();
    settings.toggle_confirmed(0);
    settings.toggle_guide_step(1);
    assert_eq!(open_section(&settings), Some(1));
}

#[test]
fn pressing_a_section_header_opens_that_section() {
    let mut runtime = window(scenes::first_run());
    assert!(runtime.find("guide-body-0").is_some());
    assert!(runtime.find("guide-body-2").is_none());
    press(&mut runtime, "guide-step-2");
    assert!(runtime.find("guide-body-2").is_some());
    assert!(runtime.find("guide-body-0").is_none());
}

#[test]
fn each_sections_button_does_that_sections_job() {
    let mut settings = scenes::on_page(scenes::first_run(), Page::General);
    settings.guide_action(1);
    assert_eq!(settings.page, Page::Monitors);

    let mut settings = saving_to("guide-save", scenes::first_run());
    settings.guide_action(3);
    assert!(!settings.model.is_dirty());
}
