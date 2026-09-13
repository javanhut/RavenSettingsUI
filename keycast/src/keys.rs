//! What a key is called on its cap, and what kind of key it is.
//!
//! Keys are named by evdev code, which is the physical key, so the caps read
//! as a US layout whatever layout the session types with. The overlay reads
//! the devices rather than the compositor's keymap, and has no way to learn
//! that keymap.

/// The mouse buttons, which evdev reports as keys.
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Held while another key is pressed.
    Modifier(Modifier),
    /// Keys that put text into a document, and the ones that edit it. What
    /// "Shortcuts only" hides, because a password is made of them.
    Typing,
    /// Everything else on a keyboard: Esc, F-keys, arrows, media keys.
    Command,
    Mouse,
}

/// In the order a combination is spelled: Super + Ctrl + Alt + Shift + key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    Super,
    Ctrl,
    Alt,
    Shift,
}

/// The kind and cap label of `code`, or `None` for a key not worth showing.
pub fn describe(code: u16) -> Option<(Kind, &'static str)> {
    use Kind::{Command, Mouse, Typing};
    let m = |m: Modifier, label| (Kind::Modifier(m), label);
    Some(match code {
        1 => (Command, "Esc"),
        2 => (Typing, "1"),
        3 => (Typing, "2"),
        4 => (Typing, "3"),
        5 => (Typing, "4"),
        6 => (Typing, "5"),
        7 => (Typing, "6"),
        8 => (Typing, "7"),
        9 => (Typing, "8"),
        10 => (Typing, "9"),
        11 => (Typing, "0"),
        12 => (Typing, "-"),
        13 => (Typing, "="),
        14 => (Typing, "Backspace"),
        15 => (Typing, "Tab"),
        16 => (Typing, "Q"),
        17 => (Typing, "W"),
        18 => (Typing, "E"),
        19 => (Typing, "R"),
        20 => (Typing, "T"),
        21 => (Typing, "Y"),
        22 => (Typing, "U"),
        23 => (Typing, "I"),
        24 => (Typing, "O"),
        25 => (Typing, "P"),
        26 => (Typing, "["),
        27 => (Typing, "]"),
        28 => (Typing, "Enter"),
        29 | 97 => m(Modifier::Ctrl, "Ctrl"),
        30 => (Typing, "A"),
        31 => (Typing, "S"),
        32 => (Typing, "D"),
        33 => (Typing, "F"),
        34 => (Typing, "G"),
        35 => (Typing, "H"),
        36 => (Typing, "J"),
        37 => (Typing, "K"),
        38 => (Typing, "L"),
        39 => (Typing, ";"),
        40 => (Typing, "'"),
        41 => (Typing, "`"),
        42 | 54 => m(Modifier::Shift, "Shift"),
        43 => (Typing, "\\"),
        44 => (Typing, "Z"),
        45 => (Typing, "X"),
        46 => (Typing, "C"),
        47 => (Typing, "V"),
        48 => (Typing, "B"),
        49 => (Typing, "N"),
        50 => (Typing, "M"),
        51 => (Typing, ","),
        52 => (Typing, "."),
        53 => (Typing, "/"),
        55 => (Typing, "*"),
        56 | 100 => m(Modifier::Alt, "Alt"),
        57 => (Typing, "Space"),
        58 => (Command, "Caps Lock"),
        59 => (Command, "F1"),
        60 => (Command, "F2"),
        61 => (Command, "F3"),
        62 => (Command, "F4"),
        63 => (Command, "F5"),
        64 => (Command, "F6"),
        65 => (Command, "F7"),
        66 => (Command, "F8"),
        67 => (Command, "F9"),
        68 => (Command, "F10"),
        69 => (Command, "Num Lock"),
        70 => (Command, "Scroll Lock"),
        71 => (Typing, "7"),
        72 => (Typing, "8"),
        73 => (Typing, "9"),
        74 => (Typing, "-"),
        75 => (Typing, "4"),
        76 => (Typing, "5"),
        77 => (Typing, "6"),
        78 => (Typing, "+"),
        79 => (Typing, "1"),
        80 => (Typing, "2"),
        81 => (Typing, "3"),
        82 => (Typing, "0"),
        83 => (Typing, "."),
        87 => (Command, "F11"),
        88 => (Command, "F12"),
        96 => (Typing, "Enter"),
        98 => (Typing, "/"),
        99 => (Command, "Print"),
        102 => (Command, "Home"),
        103 => (Command, "↑"),
        104 => (Command, "Page Up"),
        105 => (Command, "←"),
        106 => (Command, "→"),
        107 => (Command, "End"),
        108 => (Command, "↓"),
        109 => (Command, "Page Down"),
        110 => (Command, "Insert"),
        111 => (Typing, "Delete"),
        113 => (Command, "Mute"),
        114 => (Command, "Volume −"),
        115 => (Command, "Volume +"),
        119 => (Command, "Pause"),
        125 | 126 => m(Modifier::Super, "Super"),
        127 | 139 => (Command, "Menu"),
        163 => (Command, "Next"),
        164 => (Command, "Play/Pause"),
        165 => (Command, "Previous"),
        166 => (Command, "Stop"),
        224 => (Command, "Brightness −"),
        225 => (Command, "Brightness +"),
        BTN_LEFT => (Mouse, "Click"),
        BTN_RIGHT => (Mouse, "Right Click"),
        BTN_MIDDLE => (Mouse, "Middle Click"),
        _ => return None,
    })
}

pub fn modifier(code: u16) -> Option<Modifier> {
    match describe(code) {
        Some((Kind::Modifier(m), _)) => Some(m),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_type_and_modifiers_hold() {
        assert_eq!(describe(30), Some((Kind::Typing, "A")));
        assert_eq!(modifier(29), Some(Modifier::Ctrl));
        assert_eq!(modifier(97), Some(Modifier::Ctrl));
        assert_eq!(modifier(125), Some(Modifier::Super));
        assert_eq!(describe(1), Some((Kind::Command, "Esc")));
        assert_eq!(describe(BTN_LEFT), Some((Kind::Mouse, "Click")));
    }

    #[test]
    fn unknown_keys_are_not_shown() {
        assert_eq!(describe(0), None);
        assert_eq!(describe(0x2ff), None);
    }

    #[test]
    fn combinations_spell_super_first_and_shift_last() {
        let mut mods = vec![
            Modifier::Shift,
            Modifier::Ctrl,
            Modifier::Super,
            Modifier::Alt,
        ];
        mods.sort();
        assert_eq!(
            mods,
            [
                Modifier::Super,
                Modifier::Ctrl,
                Modifier::Alt,
                Modifier::Shift
            ]
        );
    }
}
