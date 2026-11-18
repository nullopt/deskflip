//! Monitor input sources: their codes, their names, and which ones a monitor says it has.
//!
//! Inputs are selected through VCP feature `0x60` ("Input Source") of the VESA Monitor Control
//! Command Set (MCCS). A monitor advertises the values it accepts in its capabilities string.

use serde::{Deserialize, Serialize};

/// The VCP feature code for "Input Source".
pub const INPUT_SOURCE_FEATURE: u8 = 0x60;

/// A value of the Input Source feature, e.g. `0x0F` for DisplayPort 1.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InputCode(pub u8);

/// A name a person would recognise from the back of their monitor.
///
/// Codes outside the MCCS standard table (some vendors use them for USB-C or Thunderbolt) get a
/// neutral name that includes the code, since their meaning varies by manufacturer.
pub fn input_name(code: InputCode) -> String {
    let name = match code.0 {
        0x01 => "VGA 1",
        0x02 => "VGA 2",
        0x03 => "DVI 1",
        0x04 => "DVI 2",
        0x05 => "Composite 1",
        0x06 => "Composite 2",
        0x07 => "S-Video 1",
        0x08 => "S-Video 2",
        0x09 => "Tuner 1",
        0x0A => "Tuner 2",
        0x0B => "Tuner 3",
        0x0C => "Component 1",
        0x0D => "Component 2",
        0x0E => "Component 3",
        0x0F => "DisplayPort 1",
        0x10 => "DisplayPort 2",
        0x11 => "HDMI 1",
        0x12 => "HDMI 2",
        other => return format!("Input {other} (0x{other:02X})"),
    };
    name.to_owned()
}

/// Extracts the input codes a monitor accepts from its MCCS capabilities string.
///
/// The string is a tree of parenthesised groups, for example
/// `(prot(monitor)vcp(10 12 60(11 12 0F) AC)mccs_ver(2.1))`. Inside the `vcp` group each word is
/// a feature code, optionally followed by a group listing its allowed values. Monitors are
/// sloppy about this format, so anything unexpected yields fewer codes rather than an error.
pub fn parse_input_codes(capabilities: &str) -> Vec<InputCode> {
    let tokens = tokenize(capabilities);
    let Some(vcp) = tokens.windows(2).position(
        |pair| matches!(pair, [Token::Word(word), Token::Open] if word.eq_ignore_ascii_case("vcp")),
    ) else {
        return Vec::new();
    };

    // Walk the direct children of the vcp group; `depth` counts nested value lists.
    let mut depth = 0usize;
    let mut index = vcp + 2;
    while let Some(token) = tokens.get(index) {
        match token {
            Token::Open => depth += 1,
            Token::Close if depth == 0 => break,
            Token::Close => depth -= 1,
            Token::Word(word) if depth == 0 && is_input_source(word) => {
                return match tokens.get(index + 1) {
                    Some(Token::Open) => value_list(&tokens[index + 2..]),
                    _ => Vec::new(),
                };
            }
            Token::Word(_) => {}
        }
        index += 1;
    }
    Vec::new()
}

fn is_input_source(word: &str) -> bool {
    u8::from_str_radix(word, 16) == Ok(INPUT_SOURCE_FEATURE)
}

/// Reads hex bytes up to the end of the current group, ignoring words that are not hex bytes.
fn value_list(tokens: &[Token]) -> Vec<InputCode> {
    let mut codes = Vec::new();
    for token in tokens {
        let Token::Word(word) = token else { break };
        if let Ok(value) = u8::from_str_radix(word, 16) {
            let code = InputCode(value);
            if !codes.contains(&code) {
                codes.push(code);
            }
        }
    }
    codes
}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    Open,
    Close,
    Word(&'a str),
}

fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut word_start = None;
    for (index, ch) in text.char_indices() {
        let is_delimiter = ch == '(' || ch == ')' || ch.is_whitespace();
        if is_delimiter && let Some(start) = word_start.take() {
            tokens.push(Token::Word(&text[start..index]));
        }
        match ch {
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            _ if is_delimiter => {}
            _ => {
                word_start.get_or_insert(index);
            }
        }
    }
    if let Some(start) = word_start {
        tokens.push(Token::Word(&text[start..]));
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELL: &str = "(prot(monitor)type(lcd)model(G2725D)cmds(01 02 03 07 0C E3 F3)vcp(02 04 05 08 10 12 14(04 05 08 0B) 16 18 1A 52 60( 11 12 0F) AC AE B2 B6 C8 C9 D6(01 04 05) DC(00 02 03 05 ) DF)mccs_ver(2.1)mswhql(1))";

    #[test]
    fn dell_inputs() {
        assert_eq!(
            parse_input_codes(DELL),
            [InputCode(0x11), InputCode(0x12), InputCode(0x0F)]
        );
    }

    #[test]
    fn sixty_without_value_list_gives_none() {
        assert!(parse_input_codes("(vcp(10 12 60 AC))").is_empty());
    }

    #[test]
    fn sixty_inside_another_list_is_not_the_feature() {
        assert!(parse_input_codes("(vcp(14(04 60 08) 16))").is_empty());
    }

    #[test]
    fn sixty_outside_vcp_group_is_ignored() {
        assert!(parse_input_codes("(cmds(01 60(11 12)) vcp(10))").is_empty());
    }

    #[test]
    fn garbage_and_unbalanced_do_not_panic() {
        for s in ["", "(((", "vcp(60(", "vcp(60(zz 11))", ")))", "vcp"] {
            let _ = parse_input_codes(s);
        }
    }

    #[test]
    fn unparsable_values_are_skipped() {
        assert_eq!(parse_input_codes("(vcp(60(zz 11 123)))"), [InputCode(0x11)]);
    }

    #[test]
    fn lowercase_hex_and_no_space() {
        assert_eq!(
            parse_input_codes("(vcp(60(0f 11)))"),
            [InputCode(0x0F), InputCode(0x11)]
        );
    }

    #[test]
    fn duplicates_are_removed_keeping_first_position() {
        assert_eq!(
            parse_input_codes("(vcp(60(11 0F 11)))"),
            [InputCode(0x11), InputCode(0x0F)]
        );
    }

    #[test]
    fn vcp_group_without_outer_parentheses() {
        assert_eq!(parse_input_codes("vcp(60(12))"), [InputCode(0x12)]);
    }

    #[test]
    fn names() {
        assert_eq!(input_name(InputCode(0x0F)), "DisplayPort 1");
        assert_eq!(input_name(InputCode(0x12)), "HDMI 2");
        assert_eq!(input_name(InputCode(0x01)), "VGA 1");
        assert_eq!(input_name(InputCode(0x1B)), "Input 27 (0x1B)");
        assert_eq!(input_name(InputCode(0x00)), "Input 0 (0x00)");
    }

    #[test]
    fn serializes_as_a_plain_number() {
        assert_eq!(
            toml::to_string(&Wrapper {
                input: InputCode(18)
            })
            .unwrap(),
            "input = 18\n"
        );
        let parsed: Wrapper = toml::from_str("input = 0x12").unwrap();
        assert_eq!(parsed.input, InputCode(0x12));
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Wrapper {
        input: InputCode,
    }
}
