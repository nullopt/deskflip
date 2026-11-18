//! Global hotkeys as text, e.g. `Ctrl+Alt+M`.
//!
//! Hotkeys are stored in the settings file as text so the file stays readable. Several
//! spellings are accepted (`ctrl+alt+m`, `Control+Alt+KeyM`); [`format`] produces the one
//! Deskflip writes and shows.

use std::str::FromStr;

pub use global_hotkey::hotkey::{Code, HotKey, Modifiers};

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    #[error("\"{0}\" is not a key combination Deskflip recognises")]
    Invalid(String),
    /// A hotkey without Ctrl, Alt or Win would swallow ordinary typing system-wide.
    #[error("a hotkey needs Ctrl, Alt or Win")]
    NeedsModifier,
}

pub fn parse(text: &str) -> Result<HotKey, HotkeyError> {
    let hotkey =
        HotKey::from_str(text.trim()).map_err(|_| HotkeyError::Invalid(text.to_owned()))?;
    if !hotkey
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::META)
    {
        return Err(HotkeyError::NeedsModifier);
    }
    Ok(hotkey)
}

/// The spelling Deskflip writes and shows: `Ctrl+Alt+Shift+Win+Key`.
pub fn format(hotkey: &HotKey) -> String {
    let mut text = String::new();
    for (modifier, name) in [
        (Modifiers::CONTROL, "Ctrl+"),
        (Modifiers::ALT, "Alt+"),
        (Modifiers::SHIFT, "Shift+"),
        (Modifiers::META, "Win+"),
    ] {
        if hotkey.mods.contains(modifier) {
            text.push_str(name);
        }
    }
    let key = hotkey.key.to_string();
    // Letter and number keys are named "KeyM" and "Digit2"; show what is printed on the key.
    let label = key
        .strip_prefix("Key")
        .or_else(|| key.strip_prefix("Digit"))
        .filter(|rest| rest.len() == 1)
        .unwrap_or(&key);
    text.push_str(label);
    text
}

/// Formats hotkey text for display, leaving it unchanged if it cannot be parsed.
pub fn pretty(text: &str) -> String {
    parse(text).map_or_else(|_| text.to_owned(), |hotkey| format(&hotkey))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_friendly_and_formal_spellings_to_the_same_hotkey() {
        let friendly = parse("Ctrl+Alt+2").unwrap();
        let formal = parse("control+alt+Digit2").unwrap();

        assert_eq!(friendly.id(), formal.id());
        assert_eq!(friendly.key, Code::Digit2);
        assert!(friendly.mods.contains(Modifiers::CONTROL | Modifiers::ALT));
    }

    #[test]
    fn modifier_order_does_not_matter() {
        assert_eq!(
            parse("Alt+Ctrl+M").unwrap().id(),
            parse("Ctrl+Alt+M").unwrap().id()
        );
    }

    #[test]
    fn pretty_formats_common_keys() {
        assert_eq!(pretty("control+alt+KeyM"), "Ctrl+Alt+M");
        assert_eq!(pretty("ctrl+alt+2"), "Ctrl+Alt+2");
        assert_eq!(pretty("shift+ctrl+F5"), "Ctrl+Shift+F5");
        assert_eq!(pretty("super+alt+ArrowLeft"), "Alt+Win+ArrowLeft");
    }

    #[test]
    fn pretty_leaves_unparsable_text_alone() {
        assert_eq!(pretty("Ctrl+Nope"), "Ctrl+Nope");
    }

    #[test]
    fn a_plain_key_is_refused() {
        assert_eq!(parse("M").unwrap_err(), HotkeyError::NeedsModifier);
    }

    #[test]
    fn shift_alone_is_not_enough() {
        assert_eq!(parse("Shift+M").unwrap_err(), HotkeyError::NeedsModifier);
    }

    #[test]
    fn unknown_key_is_invalid() {
        assert!(matches!(parse("Ctrl+Nope"), Err(HotkeyError::Invalid(_))));
        assert!(matches!(parse(""), Err(HotkeyError::Invalid(_))));
        assert!(matches!(parse("Ctrl+"), Err(HotkeyError::Invalid(_))));
    }

    #[test]
    fn format_round_trips_through_parse() {
        let hotkey = parse("Ctrl+Shift+Alt+F12").unwrap();
        assert_eq!(parse(&format(&hotkey)).unwrap().id(), hotkey.id());
    }
}
