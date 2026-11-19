//! The settings window: a separate, short-lived process started from the tray.
//!
//! Edits are made to a draft and reach the settings file only on Save. Saving signals the tray
//! process, which reloads; the two processes share nothing else.

pub mod model;

use std::process::ExitCode;
use std::sync::OnceLock;

use rune::tok::{self, type_};
use rune::{App, AppHandle, AppSpec, Cx, El, Tx, col, text};

use self::model::Model;
use crate::config::Config;
use crate::display::Snapshot;
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

/// Everything the window shows and edits.
pub struct Settings {
    pub model: Model,
    /// The monitors as Windows last reported them.
    pub snapshot: Snapshot,
    pub page: Page,
    /// Shown instead of the pages when the settings file exists but cannot be used.
    pub load_error: Option<String>,
    /// The user closed the window with unsaved changes and has not yet said what to do.
    pub close_prompt: bool,
}

impl Settings {
    /// Reads the settings file. A file that cannot be used is reported, never overwritten.
    pub fn load() -> Self {
        let (saved, load_error) = match Config::load(&paths::config_file()) {
            Ok(saved) => (saved, None),
            Err(error) => (None, Some(error.to_string())),
        };
        Self::new(Model::new(saved), load_error)
    }

    fn new(model: Model, load_error: Option<String>) -> Self {
        let page = if model.draft.layouts.is_empty() {
            Page::Monitors
        } else {
            Page::Layout(0)
        };
        Self {
            model,
            snapshot: Snapshot::default(),
            page,
            load_error,
            close_prompt: false,
        }
    }
}

pub fn run() -> ExitCode {
    // Two settings windows editing one file would overwrite each other's changes.
    let Some(_instance) = NamedMutex::try_acquire(SETTINGS_MUTEX) else {
        win::focus_window_titled(WINDOW_TITLE);
        return ExitCode::SUCCESS;
    };

    let spec = AppSpec {
        min_size: Some((760.0, 520.0)),
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

fn view(_: &Settings, cx: &mut Cx<Settings>) -> El {
    let theme = tok::theme(cx.system_dark());
    col()
        .w_full()
        .h_full()
        .center()
        .fill(theme.bg)
        .child(text(WINDOW_TITLE).font(type_::TITLE).color(theme.text))
}
