//! The settings window: a separate, short-lived process started from the tray.
//!
//! Edits are made to a draft and reach the settings file only on Save. Saving signals the tray
//! process, which reloads; the two processes share nothing else.

mod general;
mod layouts;
pub mod model;
mod monitors;
#[cfg(any(debug_assertions, test))]
pub mod scenes;
mod ui;

use std::process::ExitCode;
use std::sync::OnceLock;

use rune::tok::space;
use rune::{App, AppHandle, AppSpec, Cx, El, ScrollSpec, Shortcut, TextEdit, Tx, col, row};
use rune_kit::ShortcutRecorderState;

use self::model::Model;
use crate::autostart;
use crate::config::Config;
use crate::display::{MonitorId, Snapshot};
use crate::paths;
use crate::win::{self, NamedMutex, SETTINGS_MUTEX};

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
    /// Shown instead of the pages when the settings file exists but cannot be used.
    pub load_error: Option<String>,
    pub save_error: Option<String>,
    /// The user closed the window with unsaved changes and has not yet said what to do.
    pub close_prompt: bool,
    /// Monitors are being asked what they support.
    pub detecting: bool,
    /// The monitor the Layouts page is editing; the first one when unset.
    pub selected_monitor: Option<MonitorId>,
    /// Whether Windows starts Deskflip at sign-in. Lives in the registry, not in the draft.
    pub start_with_windows: bool,
    pub recorders: Recorders,
    pub edits: Edits,
}

impl Settings {
    /// Reads the settings file. A file that cannot be used is reported, never overwritten.
    pub fn load() -> Self {
        let (saved, load_error) = match Config::load(&paths::config_file()) {
            Ok(saved) => (saved, None),
            Err(error) => (None, Some(error.to_string())),
        };
        let mut settings = Self::new(Model::new(saved), load_error);
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
            load_error,
            save_error: None,
            close_prompt: false,
            detecting: false,
            selected_monitor: None,
            start_with_windows: false,
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
            ShortcutRecorderState::new(hotkey.as_deref().and_then(shortcut_of))
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

    pub fn go_to(&mut self, page: Page) {
        self.page = page;
        self.selected_monitor = None;
        self.sync_fields();
    }

    pub fn add_layout(&mut self) {}

    /// Writes the draft to disk and tells the tray. Returns whether it worked.
    pub fn save(&mut self) -> bool {
        false
    }

    pub fn discard(&mut self) {}
}

/// The stored form of a hotkey ("Ctrl+Alt+2") as the hotkey field's value.
fn shortcut_of(stored: &str) -> Option<Shortcut> {
    stored.parse().ok()
}

pub fn run() -> ExitCode {
    // Two settings windows editing one file would overwrite each other's changes.
    let Some(_instance) = NamedMutex::try_acquire(SETTINGS_MUTEX) else {
        win::focus_window_titled(WINDOW_TITLE);
        return ExitCode::SUCCESS;
    };

    let spec = AppSpec {
        min_size: Some((760.0, 520.0)),
        #[cfg(debug_assertions)]
        scenes: scenes::SCENES,
        ..AppSpec::new(WINDOW_TITLE, (920.0, 640.0), Settings::load, view)
    };
    // The window cannot refuse to close, so the process outlives it: a close with unsaved
    // changes brings the window straight back with a question.
    let app = App::new(spec).keep_running().on_close(on_close);
    // Set once per process; a second call cannot happen.
    let _ = HANDLE.set(app.handle());
    app.run();
    ExitCode::SUCCESS
}

fn on_close(s: &mut Settings, tx: &mut Tx) {
    if s.model.is_dirty() {
        s.close_prompt = true;
        if let Some(handle) = HANDLE.get() {
            handle.open();
        }
    } else {
        tx.quit();
    }
}

fn view(s: &Settings, cx: &mut Cx<Settings>) -> El {
    let p = ui::paint(cx);
    let k = ui::kit(p);
    let mut layer = Vec::new();

    let page = match s.page {
        Page::Layout(index) => layouts::view(p, &k, s, index, &mut layer),
        Page::Monitors => monitors::view(p, &k, s, &mut layer),
        Page::General => general::view(p, &k, s, &mut layer),
    };
    let mut content = col().grow(1.0).min_w(0.0).min_h(0.0).h_full().child(
        col()
            .key("page")
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec {
                stick_bottom: false,
            })
            .child(first_run(p, s).child(page)),
    );
    if s.model.is_dirty() {
        content = content.child(ui::save_bar(p, &k, s));
    }
    let body = row()
        .key("window")
        .w_full()
        .h_full()
        .fill(p.t.bg)
        .child(ui::sidebar(p, s))
        .child(content);
    rune_kit::root(body, layer)
}

/// The page column, led by a welcome while nothing has been saved yet.
fn first_run(p: ui::Paint, s: &Settings) -> El {
    let column = col().padding(space::S6).gap(space::S6);
    if !s.model.is_first_run() {
        return column;
    }
    column.child(ui::banner(
        p,
        "first-run-banner",
        "Welcome to Deskflip",
        "Check that each monitor is named the way you think of it, then tell Deskflip which input this PC is plugged into. After that, create a layout for each way you use your desk.",
    ))
}

#[cfg(test)]
mod tests {
    use rune::{Env, Runtime};

    use super::*;

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
    fn first_run_shows_the_banner() {
        assert!(
            window(scenes::first_run())
                .find("first-run-banner")
                .is_some()
        );
        assert!(window(scenes::desk()).find("first-run-banner").is_none());
    }
}
