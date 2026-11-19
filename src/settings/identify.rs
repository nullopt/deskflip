//! "Identify": each monitor briefly shows its number, so the user can tell which is which.

use std::thread;
use std::time::Duration;

use rune::tok::{TextStyle, space};
use rune::{AppSpec, Cx, El, WindowOptions, WindowSpec, row, text};

use super::model::Model;
use super::{HANDLE, Settings, ui};
use crate::display::Snapshot;

/// How long the numbers stay up.
pub const SHOWN_FOR: Duration = Duration::from_secs(3);

const NUMBER: TextStyle = TextStyle::new(40.0, 700.0);
const LABEL: TextStyle = TextStyle::new(15.0, 500.0);
/// The tag's size in points on a 1080p monitor at 100% scaling.
const TAG: (f32, f32) = (200.0, 72.0);

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

/// Where the tag's window goes on a monitor at `position` of `size`, both in desktop pixels:
/// its top left corner and its size. Bottom left, clear of the taskbar, and scaled with the
/// monitor so it reads the same on a 4K screen as on a 1080p one.
pub fn tag_geometry(position: (i32, i32), size: (u32, u32)) -> ((i32, i32), (u32, u32)) {
    let scale = (size.1 as f32 / 1080.0).clamp(1.0, 2.0);
    let pixels = |points: f32| (points * scale).round() as i32;
    let (width, height) = (pixels(TAG.0), pixels(TAG.1));
    (
        (
            position.0 + pixels(48.0),
            position.1 + size.1 as i32 - height - pixels(96.0),
        ),
        (width as u32, height as u32),
    )
}

/// The tag: a small opaque window, so nothing behind it is covered or dimmed.
fn view(badge: &Badge, cx: &mut Cx<Badge>) -> El {
    let dark = cx.system_dark();
    let theme = rune::tok::theme(dark);
    row()
        .key("badge")
        .w_full()
        .h_full()
        .center()
        .gap(space::S3)
        .fill(theme.card)
        .border(1.0, theme.card_border)
        .child(
            text(badge.number.to_string())
                .font(NUMBER)
                .color(rune::tok::accent(0, dark)),
        )
        .child(text(badge.label.as_str()).font(LABEL).color(theme.text))
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
                let (position, size) = tag_geometry(badge.position, badge.size);
                let options = WindowOptions {
                    borderless: true,
                    click_through: true,
                    always_on_top: true,
                    resizable: false,
                    position: Some(position),
                    physical_size: Some(size),
                    min_size: Some((0.0, 0.0)),
                    ..WindowOptions::default()
                };
                let spec = AppSpec::new(ui::APP_NAME, TAG, Badge::default, view);
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
