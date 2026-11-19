//! "Identify": a large number shown on each monitor, so the user can tell which is which.

use std::thread;
use std::time::Duration;

use rune::tok::{TextStyle, radius, space};
use rune::{AppSpec, Cx, El, WindowOptions, WindowSpec, col, text};

use super::model::Model;
use super::{HANDLE, Settings, ui};
use crate::display::Snapshot;

/// How long the numbers stay up.
pub const SHOWN_FOR: Duration = Duration::from_secs(3);

const NUMBER: TextStyle = TextStyle::new(140.0, 700.0);
const LABEL: TextStyle = TextStyle::new(22.0, 500.0);

/// What to show on one monitor, and where that monitor is on the desktop.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Badge {
    pub number: usize,
    pub label: String,
    /// The monitor's top left corner, in desktop pixels.
    pub position: (i32, i32),
    /// The monitor's size, in pixels.
    pub size: (u32, u32),
}

/// One badge per monitor this PC is using. Other monitors cannot show anything from here.
pub fn badges(snapshot: &Snapshot, model: &Model) -> Vec<Badge> {
    snapshot
        .monitors
        .iter()
        .filter(|monitor| monitor.active)
        .filter_map(|monitor| {
            let rect = monitor.rect?;
            let number = model.monitor_number(&monitor.id)?;
            let label = model
                .draft
                .monitor(&monitor.id)
                .map_or_else(String::new, |known| known.label.clone());
            Some(Badge {
                number,
                label,
                position: (rect.x, rect.y),
                size: (rect.width, rect.height),
            })
        })
        .collect()
}

/// The badge window: transparent and as large as its monitor, with a panel in the middle.
fn view(badge: &Badge, cx: &mut Cx<Badge>) -> El {
    let dark = cx.system_dark();
    let theme = rune::tok::theme(dark);
    let panel = col()
        .key("badge")
        .size(320.0, 240.0)
        .center()
        .gap(space::S1)
        .radius(radius::LG * 2.0)
        .fill(theme.card.alpha(0.94))
        .border(1.0, theme.card_border)
        .shadow(8.0, 32.0, theme.shadow)
        .child(
            text(badge.number.to_string())
                .font(NUMBER)
                .color(rune::tok::accent(0, dark)),
        )
        .child(text(badge.label.as_str()).font(LABEL).color(theme.text));
    col().w_full().h_full().center().child(panel)
}

impl Settings {
    /// Shows each monitor's number on it for a few seconds.
    pub fn identify(&mut self) {
        let Some(app) = HANDLE.get().cloned() else {
            return;
        };
        if self.identifying || rune::is_headless() {
            return;
        }
        let badges = badges(&self.snapshot, &self.model);
        if badges.is_empty() {
            return;
        }
        self.identifying = true;
        let windows: Vec<_> = badges
            .into_iter()
            .map(|badge| {
                let options = WindowOptions {
                    transparent: true,
                    borderless: true,
                    click_through: true,
                    always_on_top: true,
                    resizable: false,
                    position: Some(badge.position),
                    physical_size: Some(badge.size),
                    min_size: Some((0.0, 0.0)),
                    ..WindowOptions::default()
                };
                let spec = AppSpec::new(ui::APP_NAME, (320.0, 240.0), Badge::default, view);
                rune::open_window(
                    WindowSpec::new(format!("identify-{}", badge.number), spec)
                        .state(badge)
                        .options(options)
                        .inactive(),
                )
            })
            .collect();
        thread::spawn(move || {
            thread::sleep(SHOWN_FOR);
            app.update(move |s, _| {
                for window in &windows {
                    window.close();
                }
                s.identifying = false;
            });
        });
    }
}
