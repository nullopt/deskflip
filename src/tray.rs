//! The always-running part of Deskflip: a tray icon, its menu, and the global hotkeys.
//!
//! This process deliberately loads no GUI toolkit. It sits in a plain Windows message loop,
//! and everything it does goes through [`apply::run`], the same entry point the command line
//! uses. The settings window is a separate, short-lived process.

use std::collections::HashSet;
use std::env;
use std::process::{Command, ExitCode};

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::apply::{self, ApplyError};
use crate::config::{Config, VERSION};
use crate::display::{Display, WindowsDisplay};
use crate::hotkey::{self, HotKey};
use crate::notify;
use crate::paths;
use crate::state::{State, current_layout, next_layout};
use crate::validate::layout_is_usable;
use crate::win::{CHANGED_EVENT, NamedEvent, NamedMutex, TRAY_MUTEX, Wake, wait_for_message_or};

/// The icon's resource number in the executable, as embedded by the build script.
const ICON_RESOURCE: u16 = 1;

pub fn run() -> ExitCode {
    // A second copy has nothing to add: the first already owns the tray icon and the hotkeys.
    let Some(_instance) = NamedMutex::try_acquire(TRAY_MUTEX) else {
        return ExitCode::SUCCESS;
    };

    let mut tray = match Tray::new() {
        Ok(tray) => tray,
        Err(error) => {
            log::error!("could not start: {error}");
            notify::show("Deskflip could not start", &error);
            return ExitCode::FAILURE;
        }
    };
    log::info!("started");

    loop {
        match wait_for_message_or(&tray.changed) {
            Wake::Message => {
                if tray.handle_events() == Flow::Quit {
                    break;
                }
            }
            Wake::Signalled => tray.reload(),
            Wake::Quit => break,
        }
    }
    log::info!("stopped");
    ExitCode::SUCCESS
}

#[derive(Debug, PartialEq)]
enum MenuEntry {
    Layout {
        name: String,
        label: String,
        checked: bool,
        enabled: bool,
    },
    Separator,
    Settings,
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
enum Action {
    Apply(String),
    Cycle,
    Settings,
    Quit,
}

#[derive(PartialEq)]
enum Flow {
    Continue,
    Quit,
}

/// What the tray menu should contain for these settings.
fn menu_model(config: &Config, current: Option<&str>) -> Vec<MenuEntry> {
    let mut model: Vec<MenuEntry> = config
        .layouts
        .iter()
        .map(|layout| MenuEntry::Layout {
            name: layout.name.clone(),
            // A tab right-aligns what follows, where menus conventionally show shortcuts.
            label: layout.hotkey.as_deref().map_or_else(
                || layout.name.clone(),
                |text| format!("{}\t{}", layout.name, hotkey::pretty(text)),
            ),
            checked: current == Some(layout.name.as_str()),
            enabled: layout_is_usable(config, &layout.name),
        })
        .collect();
    if !model.is_empty() {
        model.push(MenuEntry::Separator);
    }
    model.push(MenuEntry::Settings);
    model.push(MenuEntry::Quit);
    model
}

/// The global hotkeys these settings ask for. Layouts that cannot be applied get none.
fn hotkey_bindings(config: &Config) -> Vec<(HotKey, Action)> {
    let layouts = config
        .layouts
        .iter()
        .filter(|layout| layout_is_usable(config, &layout.name))
        .filter_map(|layout| {
            Some((
                layout.hotkey.as_deref()?,
                Action::Apply(layout.name.clone()),
            ))
        });
    let cycle = config
        .cycle_hotkey
        .as_deref()
        .map(|text| (text, Action::Cycle));
    layouts
        .chain(cycle)
        .filter_map(|(text, action)| Some((hotkey::parse(text).ok()?, action)))
        .collect()
}

fn tooltip(current: Option<&str>) -> String {
    current.map_or_else(|| "Deskflip".to_owned(), |name| format!("Deskflip: {name}"))
}

struct Tray {
    icon: TrayIcon,
    hotkeys: GlobalHotKeyManager,
    changed: NamedEvent,
    config: Config,
    menu_actions: Vec<(MenuId, Action)>,
    bindings: Vec<(HotKey, Action)>,
    /// Problems already shown, so that reloading does not repeat the same notification.
    reported: HashSet<String>,
}

impl Tray {
    fn new() -> Result<Self, String> {
        let icon = Icon::from_resource(ICON_RESOURCE, None).map_err(|error| error.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(tooltip(None))
            .with_menu_on_left_click(true)
            .build()
            .map_err(|error| error.to_string())?;
        let hotkeys = GlobalHotKeyManager::new().map_err(|error| error.to_string())?;
        let changed =
            NamedEvent::open_or_create(CHANGED_EVENT).map_err(|error| error.to_string())?;

        let first_run = !paths::config_file().exists();
        let mut tray = Self {
            icon,
            hotkeys,
            changed,
            config: empty_config(),
            menu_actions: Vec::new(),
            bindings: Vec::new(),
            reported: HashSet::new(),
        };
        tray.reload();
        if first_run {
            open_settings();
        }
        Ok(tray)
    }

    /// Re-reads the settings and the current layout, and rebuilds the menu and hotkeys to match.
    fn reload(&mut self) {
        self.config = match Config::load(&paths::config_file()) {
            Ok(Some(config)) => config,
            Ok(None) => empty_config(),
            Err(error) => {
                // The file is left exactly as it is; the user is told and can fix it.
                self.report_once("Deskflip cannot read its settings", &error.to_string());
                empty_config()
            }
        };

        let state = State::load(&paths::state_file());
        let current = WindowsDisplay
            .snapshot()
            .ok()
            .and_then(|snapshot| current_layout(&self.config, &state, &snapshot))
            .map(|layout| layout.name.clone());

        self.rebuild_menu(current.as_deref());
        if let Err(error) = self.icon.set_tooltip(Some(tooltip(current.as_deref()))) {
            log::warn!("could not set tooltip: {error}");
        }
        self.rebind_hotkeys();
    }

    fn rebuild_menu(&mut self, current: Option<&str>) {
        let menu = Menu::new();
        self.menu_actions.clear();
        for entry in menu_model(&self.config, current) {
            let appended = match entry {
                MenuEntry::Layout {
                    name,
                    label,
                    checked,
                    enabled,
                } => {
                    let item = CheckMenuItem::new(label, enabled, checked, None);
                    self.menu_actions
                        .push((item.id().clone(), Action::Apply(name)));
                    menu.append(&item)
                }
                MenuEntry::Separator => menu.append(&PredefinedMenuItem::separator()),
                MenuEntry::Settings => {
                    let item = MenuItem::new("Settings\u{2026}", true, None);
                    self.menu_actions
                        .push((item.id().clone(), Action::Settings));
                    menu.append(&item)
                }
                MenuEntry::Quit => {
                    let item = MenuItem::new("Quit", true, None);
                    self.menu_actions.push((item.id().clone(), Action::Quit));
                    menu.append(&item)
                }
            };
            if let Err(error) = appended {
                log::warn!("could not build menu: {error}");
            }
        }
        self.icon.set_menu(Some(Box::new(menu)));
    }

    fn rebind_hotkeys(&mut self) {
        for (hotkey, _) in self.bindings.drain(..) {
            // Failing to release a hotkey we may never have held is not worth reporting.
            let _ = self.hotkeys.unregister(hotkey);
        }
        for (hotkey, action) in hotkey_bindings(&self.config) {
            match self.hotkeys.register(hotkey) {
                Ok(()) => self.bindings.push((hotkey, action)),
                Err(error) => {
                    let name = hotkey::format(&hotkey);
                    log::warn!("could not register {name}: {error}");
                    self.report_once(
                        "Deskflip could not use a hotkey",
                        &format!("{name} is already taken by another program. Choose a different one in Settings."),
                    );
                }
            }
        }
    }

    /// Acts on menu clicks and hotkey presses delivered since the last call.
    fn handle_events(&mut self) -> Flow {
        let mut actions = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some((_, action)) = self.menu_actions.iter().find(|(id, _)| id == event.id()) {
                actions.push(action.clone());
            }
        }
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state() == HotKeyState::Pressed
                && let Some((_, action)) = self
                    .bindings
                    .iter()
                    .find(|(hotkey, _)| hotkey.id() == event.id())
            {
                actions.push(action.clone());
            }
        }

        for action in actions {
            match action {
                Action::Apply(name) => self.apply(&name),
                Action::Cycle => {
                    let state = State::load(&paths::state_file());
                    if let Some(name) =
                        next_layout(&self.config, &state).map(|layout| layout.name.clone())
                    {
                        self.apply(&name);
                    }
                }
                Action::Settings => open_settings(),
                Action::Quit => return Flow::Quit,
            }
        }
        Flow::Continue
    }

    fn apply(&mut self, layout: &str) {
        match apply::run(layout) {
            Ok(outcome) if outcome.skipped.is_empty() => {}
            Ok(outcome) => {
                let labels: Vec<&str> = outcome
                    .skipped
                    .iter()
                    .map(|id| {
                        self.config
                            .monitor(id)
                            .map_or("a monitor", |monitor| monitor.label.as_str())
                    })
                    .collect();
                notify::show(
                    &format!("Switched to {layout}"),
                    &format!("Not connected, so left out: {}.", labels.join(", ")),
                );
            }
            // A second press while the first is still switching is not an error to report.
            Err(ApplyError::Busy) => {}
            Err(error) => notify::show(
                &format!("Deskflip could not switch to {layout}"),
                &sentence(&error.to_string()),
            ),
        }
        // `apply::run` signals the changed event, so the menu refreshes on the next wake.
    }

    fn report_once(&mut self, title: &str, body: &str) {
        if self.reported.insert(format!("{title}\n{body}")) {
            notify::show(title, body);
        }
    }
}

fn empty_config() -> Config {
    Config {
        version: VERSION,
        cycle_hotkey: None,
        monitors: Vec::new(),
        layouts: Vec::new(),
    }
}

fn open_settings() {
    let spawned = env::current_exe().and_then(|exe| Command::new(exe).arg("settings").spawn());
    if let Err(error) = spawned {
        log::error!("could not open settings: {error}");
        notify::show("Deskflip could not open Settings", &error.to_string());
    }
}

/// Error messages are written as lowercase fragments; a notification shows one as a sentence.
fn sentence(message: &str) -> String {
    let mut chars = message.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut text: String = first.to_uppercase().chain(chars).collect();
    if !text.ends_with('.') {
        text.push('.');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::desk;

    fn layout_entry(name: &str, label: &str, checked: bool, enabled: bool) -> MenuEntry {
        MenuEntry::Layout {
            name: name.to_owned(),
            label: label.to_owned(),
            checked,
            enabled,
        }
    }

    #[test]
    fn current_layout_is_checked() {
        let model = menu_model(&desk(), Some("work"));

        assert_eq!(
            model,
            [
                layout_entry("home", "home\tCtrl+Alt+1", false, true),
                layout_entry("work", "work\tCtrl+Alt+2", true, true),
                MenuEntry::Separator,
                MenuEntry::Settings,
                MenuEntry::Quit,
            ]
        );
    }

    #[test]
    fn unusable_layout_is_disabled() {
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;

        let model = menu_model(&config, None);

        assert_eq!(
            model[1],
            layout_entry("work", "work\tCtrl+Alt+2", false, false)
        );
        assert_eq!(
            model[0],
            layout_entry("home", "home\tCtrl+Alt+1", false, true)
        );
    }

    #[test]
    fn layout_without_hotkey_shows_only_its_name() {
        let mut config = desk();
        config.layouts[0].hotkey = None;

        let model = menu_model(&config, None);

        assert_eq!(model[0], layout_entry("home", "home", false, true));
    }

    #[test]
    fn no_layouts_still_has_settings_and_quit() {
        let mut config = desk();
        config.layouts.clear();

        assert_eq!(
            menu_model(&config, None),
            [MenuEntry::Settings, MenuEntry::Quit]
        );
    }

    #[test]
    fn bindings_cover_layouts_and_cycle() {
        let mut config = desk();
        config.cycle_hotkey = Some("Ctrl+Alt+M".to_owned());

        let bindings = hotkey_bindings(&config);

        let actions: Vec<&Action> = bindings.iter().map(|(_, action)| action).collect();
        assert_eq!(
            actions,
            [
                &Action::Apply("home".to_owned()),
                &Action::Apply("work".to_owned()),
                &Action::Cycle
            ]
        );
        assert_eq!(
            bindings[0].0.id(),
            crate::hotkey::parse("Ctrl+Alt+1").unwrap().id()
        );
    }

    #[test]
    fn bindings_skip_unusable_layouts_and_bad_hotkeys() {
        let mut config = desk();
        config.monitors[0].pc_input_confirmed = false;
        config.cycle_hotkey = Some("nonsense".to_owned());

        let bindings = hotkey_bindings(&config);

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].1, Action::Apply("home".to_owned()));
    }

    #[test]
    fn error_fragments_become_sentences() {
        assert_eq!(
            sentence("monitor is not connected"),
            "Monitor is not connected."
        );
        assert_eq!(sentence("Already fine."), "Already fine.");
        assert_eq!(sentence(""), "");
    }

    #[test]
    fn tooltip_names_the_current_layout() {
        assert_eq!(tooltip(Some("work")), "Deskflip: work");
        assert_eq!(tooltip(None), "Deskflip");
    }
}
