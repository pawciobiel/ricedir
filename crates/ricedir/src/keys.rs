//! Which key does what.
//!
//! A flat table. No modes, no chords of two keys, no leader key. `## Decided
//! before any code` in `TODO.md` settled that shape.
//!
//! The right-hand side of a line is the *name of an action*. A name the
//! registry does not know is reported, not ignored, and a binding can never
//! reach anything the registry does not offer. The keys are therefore not a
//! second door into the program.
//!
//! Moving the cursor is not in the table. The arrows, `Home`, `End`,
//! `PageUp`, `PageDown` and `Tab` always move, because a table that can
//! unbind them can leave a list nobody can walk.

use std::collections::HashMap;

use iced::keyboard::{Key, Modifiers, key::Named};

/// An action a key can be bound to.
///
/// Every one of these takes no argument. A binding has no pointer and no row
/// to name, so anything that needs one -- `Select`, `Toggle` -- is not here.
/// `Open` acts on the cursor, which is the keyboard's own idea of "this one".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Open,
    Leave,
    Back,
    Forward,
    SelectAll,
    Invert,
    Filter,
    ShowHidden,
    Relist,
    EditPath,
    CopyPath,
    Bookmark,
    Buffers,
    NextLayout,
    ListLayout,
    DetailLayout,
    IconLayout,
    SplitRight,
    SplitDown,
    CloseTile,
    NextTile,
    PreviousTile,
    Escape,
    Keys,
}

impl Bound {
    /// The name this action has in the config.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Leave => "up",
            Self::Back => "back",
            Self::Forward => "forward",
            Self::SelectAll => "select-all",
            Self::Invert => "invert-selection",
            Self::Filter => "filter",
            Self::ShowHidden => "show-hidden",
            Self::Relist => "relist",
            Self::EditPath => "edit-path",
            Self::CopyPath => "copy-path",
            Self::Bookmark => "add-to-places",
            Self::Buffers => "open-directories",
            Self::NextLayout => "next-layout",
            Self::ListLayout => "list-layout",
            Self::DetailLayout => "detail-layout",
            Self::IconLayout => "icon-layout",
            Self::SplitRight => "split-right",
            Self::SplitDown => "split-down",
            Self::CloseTile => "close-tile",
            Self::NextTile => "next-tile",
            Self::PreviousTile => "previous-tile",
            Self::Escape => "escape",
            Self::Keys => "keys",
        }
    }

    /// Every action a key can be bound to, for the error message and for the
    /// list a person can read.
    pub const ALL: &'static [Self] = &[
        Self::Open,
        Self::Leave,
        Self::Back,
        Self::Forward,
        Self::SelectAll,
        Self::Invert,
        Self::Filter,
        Self::ShowHidden,
        Self::Relist,
        Self::EditPath,
        Self::CopyPath,
        Self::Bookmark,
        Self::Buffers,
        Self::NextLayout,
        Self::ListLayout,
        Self::DetailLayout,
        Self::IconLayout,
        Self::SplitRight,
        Self::SplitDown,
        Self::CloseTile,
        Self::NextTile,
        Self::PreviousTile,
        Self::Escape,
        Self::Keys,
    ];

    /// Find an action by the name a config line used.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|one| one.name() == name)
    }
}

/// One key, without its modifiers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Press {
    /// Compared in lower case. iced reports the key *without* modifiers
    /// applied, so `Shift+N` arrives as `n` and this stays right.
    Character(String),
    Named(Named),
}

/// A key and the modifiers held with it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Chord {
    pub press: Press,
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Chord {
    /// Read a chord from a config key such as `"ctrl+shift+n"`.
    ///
    /// The last part is the key. Everything before it is a modifier.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim().to_lowercase();

        // A lone `+` is a key, not a separator, and splitting it would give
        // two empty parts.
        let mut parts: Vec<&str> = if text == "+" {
            vec!["+"]
        } else {
            text.split('+').map(str::trim).collect()
        };

        let Some(last) = parts.pop().filter(|part| !part.is_empty()) else {
            return Err(format!("`{text}` names no key"));
        };

        let mut chord = Self {
            press: named(last).map_or_else(|| Press::Character(last.to_owned()), Press::Named),
            control: false,
            shift: false,
            alt: false,
        };

        for part in parts {
            match part {
                "ctrl" | "control" => chord.control = true,
                "shift" => chord.shift = true,
                "alt" | "meta" => chord.alt = true,
                other => return Err(format!("`{other}` is not a modifier")),
            }
        }

        Ok(chord)
    }

    /// The chord a key press amounts to.
    pub fn of(key: &Key, modifiers: Modifiers) -> Option<Self> {
        let press = match key {
            Key::Character(character) => Press::Character(character.to_lowercase()),
            Key::Named(named) => Press::Named(*named),
            Key::Unidentified => return None,
        };

        Some(Self {
            press,
            // `command()` rather than `control()`, which is what iced calls
            // the platform's own modifier.
            control: modifiers.command(),
            shift: modifiers.shift(),
            alt: modifiers.alt(),
        })
    }

    /// How this chord is written in the config, for the readable list.
    pub fn written(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();
        for (held, word) in [
            (self.control, "Ctrl"),
            (self.shift, "Shift"),
            (self.alt, "Alt"),
        ] {
            if held {
                out.push_str(word);
                out.push('+');
            }
        }
        match &self.press {
            Press::Character(character) => out.push_str(character),
            Press::Named(named) => {
                let _ = write!(out, "{named:?}");
            }
        }
        out
    }
}

/// The named keys a config line may use.
fn named(word: &str) -> Option<Named> {
    Some(match word {
        "enter" | "return" => Named::Enter,
        "escape" | "esc" => Named::Escape,
        "backspace" => Named::Backspace,
        "delete" | "del" => Named::Delete,
        "insert" => Named::Insert,
        "tab" => Named::Tab,
        "space" => Named::Space,
        "home" => Named::Home,
        "end" => Named::End,
        "pageup" => Named::PageUp,
        "pagedown" => Named::PageDown,
        "left" => Named::ArrowLeft,
        "right" => Named::ArrowRight,
        "up" => Named::ArrowUp,
        "down" => Named::ArrowDown,
        "f1" => Named::F1,
        "f2" => Named::F2,
        "f3" => Named::F3,
        "f4" => Named::F4,
        "f5" => Named::F5,
        "f6" => Named::F6,
        "f7" => Named::F7,
        "f8" => Named::F8,
        "f9" => Named::F9,
        "f10" => Named::F10,
        "f11" => Named::F11,
        "f12" => Named::F12,
        _ => return None,
    })
}

/// Every binding in force.
#[derive(Debug, Clone)]
pub struct Bindings(HashMap<Chord, Bound>);

impl Default for Bindings {
    /// The defaults, in code, so an empty config still works.
    ///
    /// Windows and GNOME agree on most of these. That is the point: the
    /// first hour should cost a newcomer nothing.
    ///
    /// **`Ctrl+C` is copy, not `Ctrl+Shift+C`.** A terminal needs the shift
    /// because `Ctrl+C` already means interrupt there. A file manager has no
    /// such clash, and every other window on the desktop uses the plain one.
    /// `Ctrl+Shift+C` is copy *the path*, which is what several file managers
    /// use it for and what people reach for to paste a path into a terminal.
    /// The plain one is reserved here and bound in M2.
    fn default() -> Self {
        let table = [
            ("enter", Bound::Open),
            ("backspace", Bound::Leave),
            ("alt+up", Bound::Leave),
            ("alt+left", Bound::Back),
            ("alt+right", Bound::Forward),
            ("ctrl+a", Bound::SelectAll),
            ("ctrl+i", Bound::Invert),
            ("/", Bound::Filter),
            ("ctrl+f", Bound::Filter),
            ("ctrl+h", Bound::ShowHidden),
            ("f5", Bound::Relist),
            // Nautilus, Thunar and PCManFM use this one; Dolphin uses F5.
            // Both are bound, because a table maps many chords to one action
            // and nobody has to learn which family this came from.
            ("ctrl+r", Bound::Relist),
            ("ctrl+l", Bound::EditPath),
            ("ctrl+shift+c", Bound::CopyPath),
            ("ctrl+d", Bound::Bookmark),
            ("ctrl+b", Bound::Buffers),
            ("`", Bound::NextLayout),
            ("ctrl+1", Bound::ListLayout),
            ("ctrl+2", Bound::DetailLayout),
            ("ctrl+3", Bound::IconLayout),
            ("ctrl+\\", Bound::SplitRight),
            // Every other file manager opens a tab with this, and ricedir
            // has no tabs. A second view side by side is the nearest thing
            // it has, so the key does what the hand expected rather than
            // nothing at all.
            ("ctrl+t", Bound::SplitRight),
            ("ctrl+-", Bound::SplitDown),
            ("ctrl+w", Bound::CloseTile),
            ("escape", Bound::Escape),
            ("?", Bound::Keys),
        ];

        let bindings = table
            .into_iter()
            .map(|(chord, bound)| {
                let chord = Chord::parse(chord).expect("a default binding must parse");
                (chord, bound)
            })
            .collect();

        Self(bindings)
    }
}

impl Bindings {
    /// Apply what the config said, on top of the defaults.
    ///
    /// Problems are returned rather than thrown away. A typed action name
    /// that silently did nothing is the worst way for this to fail: the key
    /// simply stops working and nothing says why.
    ///
    /// An action name of `"none"` removes a default binding, which is the
    /// only way to get a key back for the window manager.
    pub fn with(mut self, table: &HashMap<String, String>) -> (Self, Vec<String>) {
        let mut problems = Vec::new();

        for (chord, action) in table {
            let chord = match Chord::parse(chord) {
                Ok(chord) => chord,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };

            if action == "none" {
                self.0.remove(&chord);
                continue;
            }

            match Bound::from_name(action) {
                Some(bound) => {
                    self.0.insert(chord, bound);
                }
                None => problems.push(format!("`{action}` is not an action")),
            }
        }

        (self, problems)
    }

    /// What this key press is bound to, if anything.
    ///
    /// Tried twice, because iced reports two keys and both are wanted.
    /// `key` has no modifiers applied, so `Shift+/` arrives as `/` and a
    /// binding can be written `shift+/`. `modified_key` has them applied, so
    /// the same press is also `?` -- which is what a person writes, and what
    /// the menu calls it. Looking only at the first made `?` unbindable on
    /// this layout, and the shifted form is layout-specific anyway.
    pub fn action(&self, key: &Key, modified: &Key, modifiers: Modifiers) -> Option<Bound> {
        if let Some(found) = Chord::of(key, modifiers).and_then(|chord| self.0.get(&chord)) {
            return Some(*found);
        }

        // Shift is dropped: it is already baked into the character.
        let without = modifiers - Modifiers::SHIFT;
        Chord::of(modified, without).and_then(|chord| self.0.get(&chord).copied())
    }

    /// Every binding, sorted by the action's name, for a list people read.
    pub fn listed(&self) -> Vec<(String, &'static str)> {
        let mut rows: Vec<(String, &'static str)> = self
            .0
            .iter()
            .map(|(chord, bound)| (chord.written(), bound.name()))
            .collect();
        rows.sort_by(|left, right| left.1.cmp(right.1).then(left.0.cmp(&right.0)));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chord_reads_its_modifiers_and_its_key() {
        let chord = Chord::parse("ctrl+shift+n").expect("should parse");
        assert_eq!(chord.press, Press::Character(String::from("n")));
        assert!(chord.control && chord.shift && !chord.alt);

        let chord = Chord::parse("F5").expect("named keys are not case-sensitive");
        assert_eq!(chord.press, Press::Named(Named::F5));
        assert!(!chord.control);

        // A lone `+` is a key, not a separator.
        assert_eq!(
            Chord::parse("+").expect("should parse").press,
            Press::Character(String::from("+"))
        );
    }

    /// A typo must be reported. A binding that silently does nothing is the
    /// worst failure here: the key stops working and nothing says why.
    #[test]
    fn a_bad_line_is_reported_rather_than_ignored() {
        assert!(Chord::parse("hyper+x").is_err(), "no such modifier");
        assert!(Chord::parse("ctrl+").is_err(), "no key");

        let mut table = HashMap::new();
        table.insert(String::from("ctrl+q"), String::from("quti"));
        let (_, problems) = Bindings::default().with(&table);
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("quti"), "say the name: {problems:?}");
    }

    /// Shift changes the character, so `?` arrives as `/` in `key` and as
    /// `?` in `modified_key`. Looking at only the first made `?` unbindable.
    #[test]
    fn a_shifted_character_is_found_by_its_modified_key() {
        let keys = Bindings::default();

        assert_eq!(
            keys.action(
                &Key::Character("/".into()),
                &Key::Character("?".into()),
                Modifiers::SHIFT
            ),
            Some(Bound::Keys)
        );

        // The plain slash is still the filter, and must not become this.
        assert_eq!(
            keys.action(
                &Key::Character("/".into()),
                &Key::Character("/".into()),
                Modifiers::empty()
            ),
            Some(Bound::Filter)
        );
    }

    /// iced reports the key without modifiers applied, so a shifted letter
    /// still arrives as the plain one. The lookup has to agree.
    #[test]
    fn a_press_finds_its_binding() {
        let keys = Bindings::default();
        let control = Modifiers::CTRL;

        assert_eq!(
            keys.action(
                &Key::Character("a".into()),
                &Key::Character("a".into()),
                control
            ),
            Some(Bound::SelectAll)
        );
        assert_eq!(
            keys.action(
                &Key::Named(Named::F5),
                &Key::Named(Named::F5),
                Modifiers::empty()
            ),
            Some(Bound::Relist)
        );
        assert_eq!(
            keys.action(
                &Key::Character("c".into()),
                &Key::Character("c".into()),
                control | Modifiers::SHIFT
            ),
            Some(Bound::CopyPath)
        );
        // Plain Ctrl+C is left for M2's copy, so it must not be the path one.
        assert_eq!(
            keys.action(
                &Key::Character("c".into()),
                &Key::Character("c".into()),
                control
            ),
            None
        );
        assert_eq!(
            keys.action(
                &Key::Character("z".into()),
                &Key::Character("z".into()),
                control
            ),
            None,
            "nothing is bound to this"
        );
    }

    #[test]
    fn the_config_overrides_and_can_unbind() {
        let mut table = HashMap::new();
        table.insert(String::from("f3"), String::from("relist"));
        table.insert(String::from("ctrl+d"), String::from("none"));

        let (keys, problems) = Bindings::default().with(&table);
        assert!(problems.is_empty(), "{problems:?}");

        assert_eq!(
            keys.action(
                &Key::Named(Named::F3),
                &Key::Named(Named::F3),
                Modifiers::empty()
            ),
            Some(Bound::Relist)
        );
        assert_eq!(
            keys.action(
                &Key::Character("d".into()),
                &Key::Character("d".into()),
                Modifiers::CTRL
            ),
            None,
            "the default should have gone"
        );
    }

    /// Every action has a name, and every name finds its action back.
    #[test]
    fn every_action_has_a_name_that_round_trips() {
        for bound in Bound::ALL {
            let name = bound.name();
            assert_eq!(Bound::from_name(name), Some(*bound), "{name}");
            assert!(
                !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "`{name}` should be kebab-case"
            );
        }
    }
}
