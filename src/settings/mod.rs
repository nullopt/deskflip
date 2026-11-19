//! The settings window: a separate, short-lived process started from the tray.
//!
//! Edits are made to a draft and reach the settings file only on Save. Saving signals the tray
//! process, which reloads; the two processes share nothing else.

mod general;
mod identify;
mod layouts;
pub mod model;
mod monitors;
mod present;
#[cfg(any(debug_assertions, test))]
pub mod scenes;
mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;
use std::thread;

use rune::tok::{space, type_};
use rune::{
    App, AppHandle, AppSpec, Cx, El, ScrollSpec, Shortcut, ShortcutError, TextEdit, Tx, Waker, col,
    row, text,
};
use rune_kit::{ButtonVariant, Dialog, Kit, ShortcutRecorderState};

use self::model::Model;
use self::monitors::TestState;
use self::present::GuideChoice;
use self::ui::Paint;
use crate::autostart;
use crate::config::Config;
use crate::display::{MonitorId, Snapshot};
use crate::hotkey;
use crate::input::InputCode;
use crate::paths;
use crate::state::State;
use crate::validate::validate;
use crate::win::{self, CHANGED_EVENT, NamedEvent, NamedMutex, SETTINGS_MUTEX};

pub const WINDOW_TITLE: &str = "Deskflip settings";

/// How background threads reach the window's state.
static HANDLE: OnceLock<AppHandle<Settings>> = OnceLock::new();

/// The page shown beside the sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// A layout, by its place in the draft.
    Layout(usize),
    Monitors,
    General,
}

/// Which hotkey a hotkey field sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyTarget {
    /// The hotkey that switches to the next layout.
    Cycle,
    /// A layout's own hotkey, by the layout's place in the draft.
    Layout(usize),
}

/// What closing the window leads to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    Quit,
    /// Ask what to do with the unsaved changes.
    Prompt,
    /// The monitors are in the middle of a test or a layout change; stay alive to finish it.
    Wait,
}

/// The hotkey fields. Each remembers whether it is waiting for a key press.
#[derive(Default)]
pub struct Recorders {
    pub cycle: ShortcutRecorderState,
    /// One per layout, in the draft's order.
    pub layouts: Vec<ShortcutRecorderState>,
}

/// The text being typed into each text field. The draft follows these.
#[derive(Default)]
pub struct Edits {
    /// The name of the layout on screen.
    pub layout_name: TextEdit,
    /// One per monitor, in the draft's order.
    pub monitor_names: Vec<TextEdit>,
    /// The hand-entered input code for each monitor.
    pub custom_codes: Vec<TextEdit>,
}

/// Everything the window shows and edits.
pub struct Settings {
    pub model: Model,
    /// The monitors as Windows last reported them.
    pub snapshot: Snapshot,
    pub page: Page,
    /// Where the settings are saved.
    pub config_path: PathBuf,
    /// Where each monitor was last seen on the desktop, for drawing the ones not in use.
    pub desk_memory: State,
    /// Whether a save is announced to the tray. Off for windows that are not the real one.
    notify_tray: bool,
    /// Shown instead of the pages when the settings file exists but cannot be used.
    pub load_error: Option<String>,
    pub save_error: Option<String>,
    /// The user closed the window with unsaved changes and has not yet said what to do.
    pub close_prompt: bool,
    /// Monitors are being asked what they support.
    pub detecting: bool,
    /// The monitor the Layouts page is editing; the first one when unset.
    pub selected_monitor: Option<MonitorId>,
    /// The setup checklist section the user opened or closed by hand, if any.
    pub guide_choice: Option<GuideChoice>,
    /// The dropdown that is open, by its key. Only one is open at a time.
    pub open_select: Option<String>,
    /// Delete was pressed once on this layout and waits to be pressed again.
    pub confirm_delete: bool,
    /// A layout is being applied to the real monitors.
    pub applying: bool,
    pub apply_message: Option<String>,
    pub test: TestState,
    /// The input each monitor's "Try another input" row will switch to.
    pub test_choice: HashMap<MonitorId, InputCode>,
    /// The window was closed while the monitors were busy; quit once they are done.
    pub quit_when_idle: bool,
    /// The numbers are on the monitors right now.
    pub identifying: bool,
    /// Whether Windows starts Deskflip at sign-in. Lives in the registry, not in the draft.
    pub start_with_windows: bool,
    pub autostart_error: Option<String>,
    pub recorders: Recorders,
    pub edits: Edits,
}

impl Settings {
    /// Reads the settings file. A file that cannot be used is reported, never overwritten.
    pub fn load() -> Self {
        let config_path = paths::config_file();
        let (saved, load_error) = match Config::load(&config_path) {
            Ok(saved) => (saved, None),
            Err(error) => (None, Some(error.to_string())),
        };
        let mut settings = Self::new(Model::new(saved), load_error);
        settings.config_path = config_path;
        settings.desk_memory = State::load(&paths::state_file());
        settings.notify_tray = true;
        settings.start_with_windows = autostart::is_enabled();
        settings
    }

    fn new(model: Model, load_error: Option<String>) -> Self {
        let page = if model.draft.layouts.is_empty() {
            Page::Monitors
        } else {
            Page::Layout(0)
        };
        let mut settings = Self {
            model,
            snapshot: Snapshot::default(),
            page,
            config_path: PathBuf::new(),
            desk_memory: State::default(),
            notify_tray: false,
            load_error,
            save_error: None,
            close_prompt: false,
            detecting: false,
            selected_monitor: None,
            guide_choice: None,
            open_select: None,
            confirm_delete: false,
            applying: false,
            apply_message: None,
            test: TestState::Idle,
            test_choice: HashMap::new(),
            quit_when_idle: false,
            identifying: false,
            start_with_windows: false,
            autostart_error: None,
            recorders: Recorders::default(),
            edits: Edits::default(),
        };
        settings.sync_fields();
        settings
    }

    /// Rebuilds the text and hotkey fields from the draft, after anything but typing changed it.
    pub fn sync_fields(&mut self) {
        let draft = &self.model.draft;
        let recorder = |hotkey: &Option<String>| {
            ShortcutRecorderState::new(hotkey.as_deref().and_then(present::shortcut_of))
        };
        self.recorders = Recorders {
            cycle: recorder(&draft.cycle_hotkey),
            layouts: draft
                .layouts
                .iter()
                .map(|layout| recorder(&layout.hotkey))
                .collect(),
        };
        let layout_name = match self.page {
            Page::Layout(index) => draft.layouts.get(index).map(|layout| layout.name.as_str()),
            _ => None,
        };
        self.edits = Edits {
            layout_name: TextEdit::new(layout_name.unwrap_or_default()),
            monitor_names: draft
                .monitors
                .iter()
                .map(|monitor| TextEdit::new(monitor.label.as_str()))
                .collect(),
            custom_codes: draft.monitors.iter().map(|_| TextEdit::new("")).collect(),
        };
    }

    /// Whether this window is changing the monitors right now. One change runs at a time: a
    /// second would talk to the monitors over the first.
    pub fn busy(&self) -> bool {
        self.applying || self.test == TestState::Running
    }

    pub fn close_action(&self) -> CloseAction {
        if self.load_error.is_some() {
            // Nothing was loaded, so there is nothing to save or lose.
            CloseAction::Quit
        } else if self.model.is_dirty() {
            CloseAction::Prompt
        } else if self.busy() {
            CloseAction::Wait
        } else {
            CloseAction::Quit
        }
    }

    pub fn go_to(&mut self, page: Page) {
        self.page = page;
        self.selected_monitor = None;
        self.open_select = None;
        self.confirm_delete = false;
        self.apply_message = None;
        self.sync_fields();
    }

    /// Opens a dropdown, or closes it if it is the open one.
    pub fn toggle_select(&mut self, key: &str) {
        self.open_select = match &self.open_select {
            Some(open) if open == key => None,
            _ => Some(key.to_owned()),
        };
    }

    /// Does what section `step` of the setup checklist asks for.
    pub fn guide_action(&mut self, step: usize) {
        match step {
            0 | 1 => self.go_to(Page::Monitors),
            2 => self.create_handover_layout(),
            _ => {
                self.save();
            }
        }
    }

    /// Opens section `step` of the setup checklist, or closes it if it is the open one.
    pub fn toggle_guide_step(&mut self, step: usize) {
        let Some(steps) = present::guide(&self.model) else {
            return;
        };
        let open = present::open_step(&steps, self.guide_choice);
        self.guide_choice = Some(GuideChoice {
            open: (open != Some(step)).then_some(step),
            when_next: present::next_step(&steps),
        });
    }

    /// What stops the draft from being saved, in words.
    pub fn problems(&self) -> Vec<String> {
        validate(&self.model.draft)
            .iter()
            .map(|problem| {
                let message = problem.message(&self.model.draft);
                match &problem.layout {
                    Some(layout) => format!("{layout}: {message}"),
                    None => message,
                }
            })
            .collect()
    }

    /// Writes the draft to disk and tells the tray. Returns whether it worked.
    pub fn save(&mut self) -> bool {
        if !self.problems().is_empty() {
            return false;
        }
        match self.model.draft.save(&self.config_path) {
            Ok(()) => {
                self.model.mark_saved();
                self.save_error = None;
                if self.notify_tray
                    && let Ok(event) = NamedEvent::open_or_create(CHANGED_EVENT)
                {
                    event.signal();
                }
                true
            }
            Err(error) => {
                self.save_error = Some(error.to_string());
                false
            }
        }
    }

    pub fn discard(&mut self) {
        self.model.discard();
        self.save_error = None;
        if let Page::Layout(index) = self.page
            && index >= self.model.draft.layouts.len()
        {
            self.page = Page::Monitors;
        }
        self.go_to(self.page);
    }

    /// Sets or clears a hotkey. A refusal leaves the draft as it was and says why.
    pub fn set_hotkey(
        &mut self,
        target: HotkeyTarget,
        shortcut: Option<&Shortcut>,
    ) -> Result<(), ShortcutError> {
        let stored = shortcut
            .map(present::stored_hotkey)
            .transpose()
            .map_err(|error| ShortcutError::InvalidAccelerator(error.to_string()))?;
        let draft = &mut self.model.draft;
        if let Some(wanted) = &stored {
            let others = std::iter::once((HotkeyTarget::Cycle, &draft.cycle_hotkey)).chain(
                draft
                    .layouts
                    .iter()
                    .enumerate()
                    .map(|(index, layout)| (HotkeyTarget::Layout(index), &layout.hotkey)),
            );
            for (owner, existing) in others {
                let taken = existing
                    .as_deref()
                    .is_some_and(|existing| &hotkey::pretty(existing) == wanted);
                if owner != target && taken {
                    return Err(ShortcutError::Conflict);
                }
            }
        }
        match target {
            HotkeyTarget::Cycle => draft.cycle_hotkey = stored,
            HotkeyTarget::Layout(index) => {
                if let Some(layout) = draft.layouts.get_mut(index) {
                    layout.hotkey = stored;
                }
            }
        }
        Ok(())
    }

    /// "Save" in the close question. Returns whether the window may now go.
    pub fn close_save(&mut self) -> bool {
        if self.save() {
            self.close_prompt = false;
            true
        } else {
            false
        }
    }

    /// "Don't save" in the close question.
    pub fn close_discard(&mut self) {
        self.model.discard();
        // On a first run there is nothing to return to; closing must still work.
        self.model.mark_saved();
        self.close_prompt = false;
    }

    pub fn close_cancel(&mut self) {
        self.close_prompt = false;
    }

    pub fn toggle_start_with_windows(&mut self) {
        let wanted = !self.start_with_windows;
        match autostart::set(wanted) {
            Ok(()) => {
                self.start_with_windows = wanted;
                self.autostart_error = None;
            }
            Err(error) => self.autostart_error = Some(error.to_string()),
        }
    }

    pub fn open_config_folder(&mut self) {
        if let Some(folder) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(folder);
            if let Err(error) = std::process::Command::new("explorer").arg(folder).spawn() {
                log::warn!("could not open {}: {error}", folder.display());
            }
        }
    }

    /// Sets the unreadable settings file aside and begins as on a first run.
    pub fn start_fresh(&mut self) {
        let backup = self.config_path.with_extension("toml.bad");
        if let Err(error) = std::fs::rename(&self.config_path, &backup) {
            self.load_error = Some(format!("Could not set the old file aside: {error}"));
            return;
        }
        self.load_error = None;
        self.model = Model::new(None);
        self.go_to(Page::Monitors);
        self.start_detect();
    }
}

/// Runs `work` off the UI thread and hands its result to `done` on the UI thread. Returns
/// whether the work was started: without a real window (tests, headless renders) nothing
/// runs, so no monitor is ever touched.
fn in_background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut Settings, T, &mut Tx) + Send + 'static,
) -> bool {
    if rune::is_headless() {
        return false;
    }
    let Some(handle) = HANDLE.get().cloned() else {
        return false;
    };
    thread::spawn(move || {
        let result = work();
        handle.update(move |s, tx| done(s, result, tx));
    });
    true
}

pub fn run() -> ExitCode {
    // Two settings windows editing one file would overwrite each other's changes. A headless
    // render edits nothing, so it may run beside an open window.
    let instance = NamedMutex::try_acquire(SETTINGS_MUTEX);
    if instance.is_none() && !rune::is_headless() {
        win::focus_window_titled(WINDOW_TITLE);
        return ExitCode::SUCCESS;
    }

    let spec = AppSpec {
        min_size: Some((760.0, 520.0)),
        #[cfg(debug_assertions)]
        scenes: scenes::SCENES,
        setup: Some(setup),
        ..AppSpec::new(WINDOW_TITLE, (920.0, 640.0), Settings::load, view)
    };
    // The window cannot refuse to close, so the process outlives it: a close with unsaved
    // changes brings the window straight back with a question.
    let app = App::new(spec)
        .keep_running()
        .on_close(on_close)
        .on_focus(on_focus);
    // Set once per process; a second call cannot happen.
    let _ = HANDLE.set(app.handle());
    app.run();
    ExitCode::SUCCESS
}

fn setup(s: &mut Settings, _: Waker) {
    if s.load_error.is_none() {
        s.start_detect();
    }
}

/// The monitors may have changed while the window was in the background: a hotkey, the tray
/// menu or Windows itself can move a monitor on or off this PC.
fn on_focus(s: &mut Settings, focused: bool, _: &mut Tx) {
    if focused && s.load_error.is_none() {
        s.glance();
    }
}

fn on_close(s: &mut Settings, tx: &mut Tx) {
    match s.close_action() {
        CloseAction::Quit => tx.quit(),
        CloseAction::Wait => s.quit_when_idle = true,
        CloseAction::Prompt => {
            s.close_prompt = true;
            if let Some(handle) = HANDLE.get() {
                handle.open();
            }
        }
    }
}

fn view(s: &Settings, cx: &mut Cx<Settings>) -> El {
    let p = ui::paint(cx);
    let k = ui::kit(p);
    let mut layer = Vec::new();

    if let Some(error) = &s.load_error {
        return rune_kit::root(load_error(p, &k, s, error), layer);
    }

    let page = match s.page {
        Page::Layout(index) => layouts::view(p, &k, s, index, &mut layer),
        Page::Monitors => monitors::view(p, &k, s, &mut layer),
        Page::General => general::view(p, &k, s),
    };
    let mut content = col().grow(1.0).min_w(0.0).min_h(0.0).h_full().child(
        col()
            .key("page")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(first_run(p, &k, s).child(page)),
    );
    if s.model.is_dirty() {
        content = content.child(ui::save_bar(p, &k, s));
    }
    close_prompt(p, &k, s, &mut layer);
    let body = row()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .child(ui::sidebar(p, s))
        .child(content);
    rune_kit::root(body, layer)
}

/// The page column, led by the setup checklist while setup is not finished.
fn first_run(p: Paint, k: &Kit, s: &Settings) -> El {
    let column = col().padding(space::S6).gap(space::S6);
    match present::guide(&s.model) {
        Some(steps) => column.child(ui::guide(p, k, s, &steps)),
        None => column,
    }
}

/// The question asked when the window is closed with unsaved changes.
fn close_prompt(p: Paint, k: &Kit, s: &Settings, layer: &mut Vec<El>) {
    let can_save = s.problems().is_empty();
    let mut body = col().w_full().px(16.0).pb(16.0).gap(space::S3);
    if !can_save {
        body = body.child(
            text("There are problems to fix before these changes can be saved.")
                .font(type_::LABEL)
                .color(p.warning)
                .wrap(),
        );
    }
    if let Some(error) = &s.save_error {
        body = body.child(
            text(format!("Could not save: {error}"))
                .font(type_::LABEL)
                .color(p.warning)
                .wrap(),
        );
    }
    let buttons = row()
        .w_full()
        .gap(space::S2)
        .justify_end()
        .child(ui::button(
            k,
            "close-cancel",
            "Cancel",
            ButtonVariant::Neutral,
            true,
            |s, _| s.close_cancel(),
        ))
        .child(ui::button(
            k,
            "close-discard",
            "Don't save",
            ButtonVariant::Neutral,
            true,
            |s, tx| {
                s.close_discard();
                tx.close_window();
            },
        ))
        .child(ui::button(
            k,
            "close-save",
            "Save",
            ButtonVariant::Contrast,
            can_save,
            |s, tx| {
                if s.close_save() {
                    tx.close_window();
                }
            },
        ));
    Dialog::new("close-prompt", "Save your changes?")
        .description("Changes you have not saved will be lost.")
        .open(s.close_prompt)
        .fit()
        .click_outside(false)
        .width(420.0)
        .child(body.child(buttons))
        .on_close(|s: &mut Settings| s.close_cancel())
        .build(k, layer);
}

/// Shown instead of the pages when the settings file cannot be used.
fn load_error(p: Paint, k: &Kit, s: &Settings, error: &str) -> El {
    let message = col()
        .key("load-error")
        .max_w(520.0)
        .gap(space::S3)
        .child(
            text("Deskflip could not read its settings file")
                .font(ui::PAGE_TITLE)
                .color(p.t.text)
                .wrap(),
        )
        .child(ui::note(p, error))
        .child(ui::note(p, &s.config_path.display().to_string()))
        .child(
            row()
                .gap(space::S2)
                .mt(space::S2)
                .child(ui::button(
                    k,
                    "open-folder",
                    "Open folder",
                    ButtonVariant::Neutral,
                    true,
                    |s, _| s.open_config_folder(),
                ))
                .child(ui::button(
                    k,
                    "start-fresh",
                    "Start fresh",
                    ButtonVariant::Contrast,
                    true,
                    |s, _| s.start_fresh(),
                )),
        )
        .child(ui::note(
            p,
            "Start fresh keeps the old file as config.toml.bad.",
        ));
    col()
        .key("window")
        .w_full()
        .h_full()
        .center()
        .padding(space::S6)
        .fill(p.t.bg)
        .child(message)
}

#[cfg(test)]
mod tests;
