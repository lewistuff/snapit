//! Converts between the settings form of a shortcut ("shift+cmd+x"), Carbon
//! virtual key codes and modifier masks, and the glyph form shown in the UI
//! ("⇧⌘X").
//!
//! Key codes are physical key positions (kVK_* constants), so bindings stay
//! stable across keyboard layouts; names are those of the US ANSI layout.

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;

/// Carbon modifier masks, as passed to RegisterEventHotKey.
pub const CMD: u32 = 1 << 8;
pub const SHIFT: u32 = 1 << 9;
pub const OPT: u32 = 1 << 11;
pub const CTRL: u32 = 1 << 12;

/// The order macOS uses when displaying modifiers.
const MOD_ORDER: [(u32, &str, &str); 4] =
    [(CTRL, "ctrl", "⌃"), (OPT, "opt", "⌥"), (SHIFT, "shift", "⇧"), (CMD, "cmd", "⌘")];

fn mod_alias(s: &str) -> Option<u32> {
    Some(match s {
        "cmd" | "command" | "super" => CMD,
        "shift" => SHIFT,
        "opt" | "option" | "alt" => OPT,
        "ctrl" | "control" => CTRL,
        _ => return None,
    })
}

/// A key code plus modifiers. The default value means "unbound".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Binding {
    pub code: u32,
    pub mods: u32,
}

/// (key code, settings name, display glyph)
const KEYS: &[(u32, &str, &str)] = &[
    (0x00, "a", "A"), (0x0B, "b", "B"), (0x08, "c", "C"), (0x02, "d", "D"),
    (0x0E, "e", "E"), (0x03, "f", "F"), (0x05, "g", "G"), (0x04, "h", "H"),
    (0x22, "i", "I"), (0x26, "j", "J"), (0x28, "k", "K"), (0x25, "l", "L"),
    (0x2E, "m", "M"), (0x2D, "n", "N"), (0x1F, "o", "O"), (0x23, "p", "P"),
    (0x0C, "q", "Q"), (0x0F, "r", "R"), (0x01, "s", "S"), (0x11, "t", "T"),
    (0x20, "u", "U"), (0x09, "v", "V"), (0x0D, "w", "W"), (0x07, "x", "X"),
    (0x10, "y", "Y"), (0x06, "z", "Z"),
    (0x1D, "0", "0"), (0x12, "1", "1"), (0x13, "2", "2"), (0x14, "3", "3"),
    (0x15, "4", "4"), (0x17, "5", "5"), (0x16, "6", "6"), (0x1A, "7", "7"),
    (0x1C, "8", "8"), (0x19, "9", "9"),
    (0x18, "equal", "="), (0x1B, "minus", "-"), (0x21, "leftbracket", "["),
    (0x1E, "rightbracket", "]"), (0x27, "quote", "'"), (0x29, "semicolon", ";"),
    (0x2A, "backslash", "\\"), (0x2B, "comma", ","), (0x2C, "slash", "/"),
    (0x2F, "period", "."), (0x32, "grave", "`"),
    (0x31, "space", "Space"), (0x24, "return", "↩"), (0x30, "tab", "⇥"),
    (0x33, "delete", "⌫"), (0x75, "forwarddelete", "⌦"), (0x35, "escape", "⎋"),
    (0x7B, "left", "←"), (0x7C, "right", "→"), (0x7D, "down", "↓"), (0x7E, "up", "↑"),
    (0x73, "home", "↖"), (0x77, "end", "↘"), (0x74, "pageup", "⇞"), (0x79, "pagedown", "⇟"),
    (0x7A, "f1", "F1"), (0x78, "f2", "F2"), (0x63, "f3", "F3"), (0x76, "f4", "F4"),
    (0x60, "f5", "F5"), (0x61, "f6", "F6"), (0x62, "f7", "F7"), (0x64, "f8", "F8"),
    (0x65, "f9", "F9"), (0x6D, "f10", "F10"), (0x67, "f11", "F11"), (0x6F, "f12", "F12"),
    (0x69, "f13", "F13"), (0x6B, "f14", "F14"), (0x71, "f15", "F15"), (0x6A, "f16", "F16"),
    (0x40, "f17", "F17"), (0x4F, "f18", "F18"), (0x50, "f19", "F19"), (0x5A, "f20", "F20"),
];

fn by_name(name: &str) -> Option<&'static (u32, &'static str, &'static str)> {
    KEYS.iter().find(|k| k.1 == name)
}

fn by_code(code: u32) -> Option<&'static (u32, &'static str, &'static str)> {
    KEYS.iter().find(|k| k.0 == code)
}

fn is_function_key(code: u32) -> bool {
    by_code(code).is_some_and(|k| k.1.len() >= 2 && k.1.starts_with('f') && k.1.as_bytes()[1].is_ascii_digit())
}

impl Binding {
    pub fn is_unbound(&self) -> bool {
        *self == Binding::default()
    }

    /// Parses "cmd+shift+x". The empty string parses to the unbound binding.
    pub fn parse(s: &str) -> Result<Binding, String> {
        let s = s.trim().to_lowercase();
        if s.is_empty() {
            return Ok(Binding::default());
        }
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last().unwrap();
        let mut b = Binding::default();
        for m in mods {
            b.mods |= mod_alias(m).ok_or_else(|| format!("hotkey {s:?}: unknown modifier {m:?}"))?;
        }
        b.code = by_name(key).ok_or_else(|| format!("hotkey {s:?}: unknown key {key:?}"))?.0;
        b.validate()?;
        Ok(b)
    }

    /// Rejects bindings that would swallow ordinary typing: a shortcut needs
    /// ⌘, ⌃ or ⌥ unless it is a function key.
    pub fn validate(&self) -> Result<(), String> {
        if self.is_unbound() {
            return Ok(());
        }
        if by_code(self.code).is_none() {
            return Err(format!("unsupported key code 0x{:02X}", self.code));
        }
        if self.mods & (CMD | CTRL | OPT) == 0 && !is_function_key(self.code) {
            return Err("shortcut needs ⌘, ⌃ or ⌥".into());
        }
        Ok(())
    }

    /// The glyph form, e.g. "⇧⌘X"; "" when unbound.
    pub fn display(&self) -> String {
        if self.is_unbound() {
            return String::new();
        }
        let mut out: String = MOD_ORDER.iter().filter(|m| self.mods & m.0 != 0).map(|m| m.2).collect();
        match by_code(self.code) {
            Some(k) => out.push_str(k.2),
            None => out.push_str(&format!("0x{:02X}", self.code)),
        }
        out
    }

    /// A well-known shortcut this binding would shadow, if any.
    pub fn conflict(&self) -> Option<&'static str> {
        KNOWN.get(self).copied()
    }
}

/// The settings form, e.g. "shift+cmd+x"; "" when unbound.
impl fmt::Display for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_unbound() {
            return Ok(());
        }
        let mut parts: Vec<&str> = MOD_ORDER.iter().filter(|m| self.mods & m.0 != 0).map(|m| m.1).collect();
        parts.push(by_code(self.code).map_or("?", |k| k.1));
        f.write_str(&parts.join("+"))
    }
}

/// Well-known shortcuts that a global hotkey would shadow. System entries are
/// ones macOS reserves; app entries are common developer-tool shortcuts.
static KNOWN: LazyLock<HashMap<Binding, &'static str>> = LazyLock::new(|| {
    [
        ("shift+cmd+3", "macOS: screenshot of screen"),
        ("shift+cmd+4", "macOS: screenshot of selection"),
        ("shift+cmd+5", "macOS: screenshot toolbar"),
        ("shift+cmd+6", "macOS: Touch Bar screenshot"),
        ("ctrl+shift+cmd+3", "macOS: screenshot to clipboard"),
        ("ctrl+shift+cmd+4", "macOS: selection to clipboard"),
        ("cmd+space", "macOS: Spotlight"),
        ("ctrl+space", "macOS: input source"),
        ("ctrl+cmd+space", "macOS: emoji picker"),
        ("opt+cmd+space", "macOS: Finder search"),
        ("cmd+tab", "macOS: app switcher"),
        ("cmd+grave", "macOS: window switcher"),
        ("ctrl+cmd+q", "macOS: lock screen"),
        ("ctrl+cmd+f", "macOS: full screen"),
        ("opt+cmd+escape", "macOS: force quit"),
        ("shift+cmd+q", "macOS: log out"),
        ("cmd+q", "every app: Quit"),
        ("cmd+w", "every app: Close"),
        ("cmd+h", "every app: Hide"),
        ("cmd+m", "every app: Minimize"),
        ("cmd+c", "every app: Copy"),
        ("cmd+v", "every app: Paste"),
        ("cmd+x", "every app: Cut"),
        ("cmd+z", "every app: Undo"),
        ("shift+cmd+z", "every app: Redo"),
        ("cmd+a", "every app: Select All"),
        ("cmd+s", "every app: Save"),
        ("cmd+comma", "every app: Settings"),
        ("shift+cmd+c", "Chrome DevTools: Inspect element"),
        ("shift+cmd+x", "Slack: strikethrough"),
        ("shift+cmd+p", "VS Code: Command Palette"),
        ("shift+cmd+f", "VS Code: Search in files"),
        ("shift+cmd+t", "Browsers: reopen closed tab"),
        ("shift+cmd+n", "Browsers: private window"),
    ]
    .into_iter()
    .map(|(s, why)| (Binding::parse(s).expect(s), why))
    .collect()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trip() {
        for (input, canon, glyph) in [
            ("cmd+shift+x", "shift+cmd+x", "⇧⌘X"),
            ("CMD+Shift+C", "shift+cmd+c", "⇧⌘C"),
            ("ctrl+alt+cmd+1", "ctrl+opt+cmd+1", "⌃⌥⌘1"),
            ("opt+space", "opt+space", "⌥Space"),
            ("f13", "f13", "F13"),
            ("", "", ""),
        ] {
            let b = Binding::parse(input).unwrap();
            assert_eq!(b.to_string(), canon, "{input}");
            assert_eq!(b.display(), glyph, "{input}");
        }
    }

    #[test]
    fn carbon_values() {
        let b = Binding::parse("cmd+shift+c").unwrap();
        assert_eq!((b.code, b.mods), (0x08, 768));
    }

    #[test]
    fn parse_errors() {
        for s in ["cmd+", "hyper+x", "x", "shift+x", "cmd+nope"] {
            assert!(Binding::parse(s).is_err(), "{s} should fail");
        }
    }

    #[test]
    fn conflicts() {
        assert!(Binding::parse("shift+cmd+4").unwrap().conflict().is_some());
        assert!(Binding::parse("ctrl+opt+cmd+k").unwrap().conflict().is_none());
    }
}
