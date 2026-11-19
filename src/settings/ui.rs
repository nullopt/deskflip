//! The window's look: colours, and the pieces every page is built from.

use rune::tok::{self, TextStyle, Theme, motion, radius, space, type_};
use rune::{Color, Cx, El, Shortcut, ShortcutError, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit, PaletteSeed, Select};

use super::{Page, Settings};

/// The title of windows that are not the settings window itself.
pub const APP_NAME: &str = "Deskflip";
pub const PAGE_TITLE: TextStyle = TextStyle::new(20.0, 600.0);
pub const SIDEBAR_WIDTH: f32 = 200.0;
const ROW_HEIGHT: f32 = 44.0;

/// Everything a piece needs to paint itself.
#[derive(Clone, Copy)]
pub struct Paint {
    pub t: Theme,
    pub accent: Color,
    /// The colour of a monitor that is showing another device.
    pub other: Color,
    pub warning: Color,
    /// One device pixel, in points.
    pub hair: f32,
}

pub fn paint(cx: &mut Cx<Settings>) -> Paint {
    let dark = cx.system_dark();
    Paint {
        t: tok::theme(dark),
        accent: tok::accent(0, dark),
        other: tok::accent(3, dark),
        warning: tok::accent(3, dark),
        hair: 1.0 / cx.env().scale,
    }
}

/// The ready-made widgets, coloured to sit on a card.
pub fn kit(p: Paint) -> Kit {
    Kit::from_palette(PaletteSeed {
        bg: p.t.card,
        fg: p.t.text,
        accent: p.accent,
    })
}

/// A caption over a rounded card whose rows are divided by hairlines.
pub fn section(p: Paint, key: &str, title: &str, rows: Vec<El>) -> El {
    let count = rows.len();
    let mut card = card(p);
    for (index, row) in rows.into_iter().enumerate() {
        card = card.child(row);
        if index + 1 < count {
            card = card.child(
                el().key(format!("separator-{index}"))
                    .h(p.hair)
                    .ml(space::S3)
                    .mr(space::S3)
                    .fill(p.t.separator)
                    .a11y_hidden(),
            );
        }
    }
    col()
        .key(key)
        .gap(space::S2)
        .child(caption(p, title))
        .child(card)
}

pub fn caption(p: Paint, title: &str) -> El {
    text(title)
        .key("caption")
        .font(type_::SECTION)
        .color(p.t.text_secondary)
        .ml(space::S3)
}

/// An empty card, for content that is not a list of rows.
pub fn card(p: Paint) -> El {
    col()
        .key("card")
        .padding(space::S1)
        .fill(p.t.card)
        .radius(radius::LG)
        .border(p.hair, p.t.card_border)
        .shadow(1.0, 2.0, p.t.shadow)
        .shadow(8.0, 24.0, p.t.shadow_soft)
}

/// A label on the left and its control on the right.
pub fn setting_row(p: Paint, key: &str, label: &str, control: El) -> El {
    row()
        .key(key)
        .min_h(ROW_HEIGHT)
        .px(space::S3)
        .gap(space::S3)
        .center_y()
        .justify_between()
        .child(text(label).key("label").font(type_::BODY).color(p.t.text))
        .child(control)
}

/// A row whose label has a line of explanation under it.
pub fn setting_row_note(p: Paint, key: &str, label: &str, note_text: &str, control: El) -> El {
    row()
        .key(key)
        .min_h(ROW_HEIGHT)
        .px(space::S3)
        .py(space::S2)
        .gap(space::S6)
        .center_y()
        .justify_between()
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(text(label).key("label").font(type_::BODY).color(p.t.text))
                .child(note(p, note_text)),
        )
        .child(control)
}

/// Secondary text that wraps.
pub fn note(p: Paint, content: &str) -> El {
    text(content)
        .key("note")
        .font(type_::LABEL)
        .color(p.t.text_secondary)
        .wrap()
}

/// A row that is only explanation.
pub fn note_row(p: Paint, key: &str, content: &str) -> El {
    row()
        .key(key)
        .min_h(ROW_HEIGHT)
        .px(space::S3)
        .py(space::S2)
        .center_y()
        .child(note(p, content).grow(1.0).min_w(0.0))
}

/// A switch. A disabled one is dimmed and does nothing.
pub fn toggle(
    p: Paint,
    key: &str,
    on: bool,
    enabled: bool,
    on_press: impl Fn(&mut Settings) + 'static,
) -> El {
    let (width, height, knob) = (38.0, 22.0, 18.0);
    let inset = (height - knob) / 2.0;
    let switch = el().key(key).role("switch").checked(on);
    let switch = if enabled {
        switch.focusable(true).on_press(on_press)
    } else {
        switch.opacity(0.4).disabled(true)
    };
    switch
        .size(width, height)
        .radius(height / 2.0)
        .fill(if on { p.accent } else { p.t.control_off })
        .motion(motion::SNAPPY)
        .child(
            el().key("knob")
                .size(knob, knob)
                .absolute(if on { width - knob - inset } else { inset }, inset)
                .radius(knob / 2.0)
                .fill(p.t.knob)
                .shadow(1.0, 3.0, p.t.knob_shadow)
                .shadow(0.0, 0.5, p.t.edge),
        )
}

/// One choice out of a few, side by side. `blocked` choices are dimmed and do nothing.
pub fn segmented(
    p: Paint,
    key: &str,
    labels: &[&str],
    selected: Option<usize>,
    blocked: &[bool],
    segment_width: f32,
    on_pick: impl Fn(&mut Settings, usize) + Clone + 'static,
) -> El {
    let (height, pad) = (28.0, 2.0);
    let mut segments = row().key("segments");
    for (index, label) in labels.iter().enumerate() {
        let is_selected = selected == Some(index);
        let is_blocked = blocked.get(index).copied().unwrap_or(false);
        let mut segment = el()
            .key(format!("{key}-{index}"))
            .role("segment")
            .label(*label)
            .checked(is_selected)
            .size(segment_width, height - 2.0 * pad)
            .center()
            .radius(radius::SM)
            .child(
                text(*label)
                    .key("label")
                    .font(type_::LABEL)
                    .color(if is_selected {
                        p.t.text
                    } else {
                        p.t.text_secondary
                    })
                    .a11y_hidden(),
            );
        if is_blocked {
            segment = segment.opacity(0.4).disabled(true);
        } else {
            let pick = on_pick.clone();
            segment = segment
                .focusable(true)
                .hover(if is_selected {
                    Color::TRANSPARENT
                } else {
                    p.t.hover
                })
                .on_press(move |s: &mut Settings| pick(s, index));
        }
        segments = segments.child(segment);
    }
    let mut control = el()
        .key(key)
        .role("segmented")
        .size(segment_width * labels.len() as f32 + 2.0 * pad, height)
        .padding(pad)
        .radius(radius::MD)
        .fill(p.t.control_track)
        .motion(motion::SNAPPY);
    if let Some(index) = selected {
        control = control.child(
            el().key("indicator")
                .size(segment_width, height - 2.0 * pad)
                .absolute(pad + index as f32 * segment_width, pad)
                .radius(radius::SM)
                .fill(p.t.indicator)
                .shadow(1.0, 3.0, p.t.shadow)
                .shadow(0.0, 0.5, p.t.edge)
                .a11y_hidden(),
        );
    }
    control.child(segments)
}

/// A page's title with its buttons on the right.
pub fn page_header(p: Paint, title: &str, actions: Vec<El>) -> El {
    row()
        .key("page-header")
        .center_y()
        .justify_between()
        .gap(space::S3)
        .child(
            text(title)
                .key("page-title")
                .font(PAGE_TITLE)
                .color(p.t.text),
        )
        .child(row().gap(space::S2).center_y().children(actions))
}

/// A kit button that tests and screen readers can find by `key`.
pub fn button(
    k: &Kit,
    key: &str,
    label: &str,
    variant: ButtonVariant,
    enabled: bool,
    on_press: impl Fn(&mut Settings, &mut Tx) + 'static,
) -> El {
    el().key(key).child(
        Button::new(label)
            .variant(variant)
            .disabled(!enabled)
            .on_press_tx(on_press)
            .build(k),
    )
}

/// What a dropdown shows.
pub struct Dropdown<'a> {
    /// Names the dropdown; also what [`Settings::open_select`] holds while it is open.
    pub key: String,
    pub label: &'a str,
    pub options: &'a [String],
    pub current: Option<&'a str>,
    pub width: f32,
}

/// A dropdown that closes any other open one, and itself once a choice is made.
pub fn dropdown(
    k: &Kit,
    s: &Settings,
    dropdown: Dropdown,
    on_choose: impl Fn(&mut Settings, &str) + Clone + 'static,
    layer: &mut Vec<El>,
) -> El {
    let toggle_key = dropdown.key.clone();
    let select = Select::new(format!("{}-select", dropdown.key), dropdown.options)
        .current(dropdown.current)
        .placeholder("Choose\u{2026}")
        .label(dropdown.label)
        .width(dropdown.width)
        .open(s.open_select.as_deref() == Some(dropdown.key.as_str()))
        .on_toggle(move |s: &mut Settings| s.toggle_select(&toggle_key))
        .on_close(|s: &mut Settings| s.open_select = None)
        .on_choose(move |s: &mut Settings, name: &str, _| {
            on_choose(s, name);
            s.open_select = None;
        })
        .build(k, layer);
    el().key(dropdown.key).child(select)
}

/// Why a hotkey field refused a key combination, in Windows' names for the keys.
pub fn hotkey_refusal(error: &ShortcutError, shortcut: Option<&Shortcut>) -> String {
    let chord = shortcut.map(|shortcut| shortcut.keycaps().join("+"));
    match (error, chord) {
        (ShortcutError::Conflict, Some(chord)) => {
            format!("{chord} is already used by another hotkey here.")
        }
        (ShortcutError::InvalidAccelerator(_), Some(_)) => {
            "A hotkey needs Ctrl, Alt or Win, and a key Deskflip can register.".to_owned()
        }
        (_, None) => "That key can't be part of a hotkey.".to_owned(),
        (_, Some(chord)) => format!("{chord} can't be used."),
    }
}

/// A line of news about something the user just did.
pub fn notice(p: Paint, key: &str, message: &str) -> El {
    row()
        .key(key)
        .px(space::S3 + space::S1)
        .py(space::S3)
        .radius(radius::LG)
        .fill(p.accent.alpha(0.14))
        .border(p.hair, p.accent.alpha(0.35))
        .child(
            text(message)
                .font(type_::BODY)
                .color(p.t.text)
                .wrap()
                .grow(1.0)
                .min_w(0.0),
        )
}

pub fn sidebar(p: Paint, s: &Settings) -> El {
    let mut list = col()
        .key("sidebar")
        .w(SIDEBAR_WIDTH)
        .h_full()
        .padding(space::S3)
        .gap(2.0)
        .child(
            text("Layouts")
                .font(type_::SECTION)
                .color(p.t.text_secondary)
                .ml(space::S2)
                .mt(space::S2)
                .mb(space::S1),
        );
    for (index, layout) in s.model.draft.layouts.iter().enumerate() {
        let name = if layout.name.is_empty() {
            "Unnamed layout"
        } else {
            &layout.name
        };
        list = list.child(nav_item(
            p,
            &format!("nav-layout-{index}"),
            name,
            s.page == Page::Layout(index),
            p.t.text,
            move |s| s.go_to(Page::Layout(index)),
        ));
    }
    list.child(nav_item(
        p,
        "nav-new-layout",
        "+ New layout",
        false,
        p.accent,
        Settings::add_layout,
    ))
    .child(
        el().h(p.hair)
            .my(space::S2)
            .mx(space::S2)
            .fill(p.t.separator)
            .a11y_hidden(),
    )
    .child(nav_item(
        p,
        "nav-monitors",
        "Monitors",
        s.page == Page::Monitors,
        p.t.text,
        |s| s.go_to(Page::Monitors),
    ))
    .child(nav_item(
        p,
        "nav-general",
        "General",
        s.page == Page::General,
        p.t.text,
        |s| s.go_to(Page::General),
    ))
}

fn nav_item(
    p: Paint,
    key: &str,
    label: &str,
    selected: bool,
    color: Color,
    on_press: impl Fn(&mut Settings) + 'static,
) -> El {
    let tint = p.accent.alpha(0.18);
    row()
        .key(key)
        .role("tab")
        .label(label)
        .checked(selected)
        .focusable(true)
        .h(32.0)
        .px(space::S2)
        .center_y()
        .radius(radius::MD)
        .fill(if selected { tint } else { Color::TRANSPARENT })
        .hover(if selected { tint } else { p.t.hover })
        .press(p.t.press)
        .on_press(on_press)
        .child(
            text(label)
                .font(if selected { type_::TITLE } else { type_::BODY })
                .color(color)
                .a11y_hidden(),
        )
}

/// The bar that appears while there are unsaved changes.
pub fn save_bar(p: Paint, k: &Kit, s: &Settings) -> El {
    let problems = s.problems();
    let mut messages = col().grow(1.0).min_w(0.0).gap(2.0).child(
        text(if s.model.is_first_run() {
            "Nothing is saved yet"
        } else {
            "Unsaved changes"
        })
        .font(type_::TITLE)
        .color(p.t.text),
    );
    for problem in &problems {
        messages = messages.child(
            text(problem.as_str())
                .font(type_::LABEL)
                .color(p.warning)
                .wrap(),
        );
    }
    if let Some(error) = &s.save_error {
        messages = messages.child(
            text(format!("Could not save: {error}"))
                .font(type_::LABEL)
                .color(p.warning)
                .wrap(),
        );
    }
    let mut buttons = row().gap(space::S2).center_y();
    if !s.model.is_first_run() {
        buttons = buttons.child(button(
            k,
            "discard",
            "Discard",
            ButtonVariant::Neutral,
            true,
            |s, _| s.discard(),
        ));
    }
    buttons = buttons.child(button(
        k,
        "save",
        "Save",
        ButtonVariant::Contrast,
        problems.is_empty(),
        |s, _| {
            s.save();
        },
    ));
    col()
        .key("save-bar")
        .fill(p.t.card)
        .child(el().h(p.hair).fill(p.t.separator).a11y_hidden())
        .child(
            row()
                .px(space::S6)
                .py(space::S3)
                .gap(space::S3)
                .center_y()
                .child(messages)
                .child(buttons),
        )
}

/// A highlighted card with a title and a paragraph.
pub fn banner(p: Paint, key: &str, title: &str, body: &str) -> El {
    col()
        .key(key)
        .padding(space::S3 + space::S1)
        .gap(space::S1)
        .radius(radius::LG)
        .fill(p.accent.alpha(0.14))
        .border(p.hair, p.accent.alpha(0.35))
        .child(text(title).font(type_::TITLE).color(p.t.text))
        .child(text(body).font(type_::BODY).color(p.t.text).wrap())
}
