//! "Identify": each monitor briefly shows its number, so the user can tell which is which.

use std::thread;
use std::time::Duration;

use rune::tok::{TextStyle, radius, space};
use rune::{AppSpec, Cx, El, WindowOptions, WindowSpec, col, row, text};

use super::model::Model;
use super::{HANDLE, Settings, ui};
use crate::display::Snapshot;

/// How long the numbers stay up.
pub const SHOWN_FOR: Duration = Duration::from_secs(3);

const NUMBER: TextStyle = TextStyle::new(40.0, 700.0);
const LABEL: TextStyle = TextStyle::new(15.0, 500.0);
/// How far the tag sits from the monitor's edges, clear of the taskbar.
const MARGIN: f32 = 64.0;

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

/// The badge window: transparent and as large as its monitor, with a small tag in the bottom
/// left corner, out of the way of whatever is on screen.
fn view(badge: &Badge, cx: &mut Cx<Badge>) -> El {
    let dark = cx.system_dark();
    let theme = rune::tok::theme(dark);
    let tag = row()
        .key("badge")
        .px(space::S3 + space::S1)
        .py(space::S2)
        .gap(space::S3)
        .center_y()
        .radius(radius::LG)
        .fill(theme.card.alpha(0.94))
        .border(1.0, theme.card_border)
        .shadow(4.0, 16.0, theme.shadow)
        .child(
            text(badge.number.to_string())
                .font(NUMBER)
                .color(rune::tok::accent(0, dark)),
        )
        .child(text(badge.label.as_str()).font(LABEL).color(theme.text));
    col()
        .w_full()
        .h_full()
        .padding(MARGIN)
        .justify_end()
        .items_start()
        .child(tag)
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
