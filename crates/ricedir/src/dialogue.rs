//! The one modal in the program, and what it asks.
//!
//! This is the first dialogue ricedir has, so it settles what a dialogue looks
//! like: a panel over the list, a sentence saying what happened, and buttons
//! that each say what they will do rather than "OK".
//!
//! Everything here is a question about opening a file, because that is the
//! only thing in M1 that can go wrong in a way a person has to answer.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};

use crate::config::Theme;
use crate::open::Plan;

/// What is being asked.
#[derive(Debug, Clone)]
pub enum Dialogue {
    /// Nothing in the config opens this. The choices are here rather than in
    /// a config file nobody has opened yet.
    NoHandler { path: PathBuf, mime: Option<String> },
    /// A scan rule was unhappy but did not refuse.
    Warned {
        path: PathBuf,
        plan: Box<Plan>,
        rule: String,
        detail: Option<String>,
    },
    /// A scan rule refused. There is nothing to decide, only to read.
    Blocked {
        path: PathBuf,
        rule: String,
        detail: Option<String>,
    },
    /// The type is one the fallback may never guess at.
    Refused { path: PathBuf, mime: String },
    /// About to delete, for good.
    ///
    /// The one dialogue here that is not about opening a file. There is no
    /// trash yet and no undo, so this is the only thing standing between a
    /// keystroke and somebody's work.
    Deleting { paths: Vec<PathBuf>, buffer: usize },
}

/// What the person chose.
#[derive(Debug, Clone)]
pub enum Choice {
    /// Go ahead with the plan that was shown.
    Anyway,
    /// Use `xdg-open`, this once.
    Once,
    /// Use `xdg-open` from now on, and write that into the config.
    Always,
    /// Run what was typed into the box.
    Named,
    /// Type a program name.
    Typing(String),
    /// Delete what was named, for good.
    Delete,
    Dismiss,
}

impl Dialogue {
    const fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::NoHandler { path, .. }
            | Self::Warned { path, .. }
            | Self::Blocked { path, .. }
            | Self::Refused { path, .. } => Some(path),
            // It is about several, and the view names them itself.
            Self::Deleting { .. } => None,
        }
    }

    fn name(&self) -> String {
        let Some(path) = self.path() else {
            return String::new();
        };

        path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    }
}

/// The names, short enough to read, and how many were left out.
///
/// Three and a count. A list of nine hundred filenames is not a thing anybody
/// checks before pressing a button, so it would make the dialogue worse.
fn named(paths: &[PathBuf]) -> String {
    let shown: Vec<String> = paths
        .iter()
        .take(3)
        .map(|path| {
            path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        })
        .collect();

    match paths.len().saturating_sub(shown.len()) {
        0 => shown.join(", "),
        more => format!("{}, and {more} more", shown.join(", ")),
    }
}

/// Draw the dialogue over whatever is behind it.
pub fn view<'a>(dialogue: &'a Dialogue, theme: &Theme, typed: &'a str) -> Element<'a, Choice> {
    let heading = |what: String| text(what).size(16);
    let detail = |what: String| text(what).size(13).color(theme.dim.color());

    let body: Element<'a, Choice> = match dialogue {
        Dialogue::NoHandler { mime, .. } => {
            let kind = mime
                .clone()
                .unwrap_or_else(|| String::from("an unknown type"));

            column![
                heading(format!("Nothing opens {}", dialogue.name())),
                detail(format!("It looks like {kind}.")),
                text_input("a program, for example: emacs", typed)
                    .on_input(Choice::Typing)
                    .on_submit(Choice::Named)
                    .padding(6),
                row![
                    button(text("Open with that").size(13)).on_press(Choice::Named),
                    button(text("Use xdg-open once").size(13)).on_press(Choice::Once),
                    button(text("Always use xdg-open").size(13)).on_press(Choice::Always),
                    button(text("Cancel").size(13)).on_press(Choice::Dismiss),
                ]
                .spacing(8),
                detail(String::from(
                    "\"Always\" writes a handler into your config, so this is asked once."
                )),
            ]
            .spacing(10)
            .into()
        }

        Dialogue::Warned {
            rule, detail: why, ..
        } => column![
            heading(format!("{} looks wrong", dialogue.name())),
            detail(format!("The rule `{rule}` was unhappy.")),
            detail(why.clone().unwrap_or_default()),
            row![
                button(text("Open it anyway").size(13)).on_press(Choice::Anyway),
                button(text("Cancel").size(13)).on_press(Choice::Dismiss),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .into(),

        Dialogue::Blocked {
            rule, detail: why, ..
        } => column![
            heading(format!("{} was not opened", dialogue.name())),
            detail(format!("The rule `{rule}` refused it.")),
            detail(why.clone().unwrap_or_default()),
            detail(String::from(
                "Edit that rule in your config if this was wrong."
            )),
            button(text("Close").size(13)).on_press(Choice::Dismiss),
        ]
        .spacing(10)
        .into(),

        Dialogue::Deleting { paths, .. } => column![
            heading(format!(
                "Delete {} thing{} for good?",
                paths.len(),
                if paths.len() == 1 { "" } else { "s" }
            )),
            detail(named(paths)),
            detail(String::from(
                "There is no trash yet, so this cannot be undone. A directory \
                 goes with everything inside it."
            )),
            row![
                button(text("Delete for good").size(13)).on_press(Choice::Delete),
                button(text("Cancel").size(13)).on_press(Choice::Dismiss),
            ]
            .spacing(8),
        ]
        .spacing(10)
        .into(),

        Dialogue::Refused { mime, .. } => column![
            heading(format!("{} would be run, not opened", dialogue.name())),
            detail(format!("It is {mime}.")),
            detail(String::from(
                "ricedir will not guess a program for something that would run. \
                 Name a handler for this type in your config if you meant to."
            )),
            button(text("Close").size(13)).on_press(Choice::Dismiss),
        ]
        .spacing(10)
        .into(),
    };

    // Copied out rather than borrowed: the style closures outlive this call,
    // and a colour is four floats.
    let background = theme.background.color();
    let accent = theme.accent.color();

    // A panel, centred, over a wash that dims what is behind it. The wash is
    // what makes it read as modal without a separate surface.
    container(
        container(body)
            .padding(20)
            .max_width(560)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(background.into()),
                border: iced::Border {
                    color: accent,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(
            iced::Color {
                a: 0.6,
                ..background
            }
            .into(),
        ),
        ..container::Style::default()
    })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialogue() -> Dialogue {
        Dialogue::NoHandler {
            path: PathBuf::from("/tmp/holiday photos.zip"),
            mime: Some(String::from("application/zip")),
        }
    }

    /// A dialogue that does not say which file it is about is a dialogue
    /// nobody can answer -- and a name with a space in it must survive.
    #[test]
    fn the_dialogue_names_its_file() {
        assert_eq!(dialogue().name(), "holiday photos.zip");
    }

    /// A path with no filename at all still has to render something rather
    /// than an empty heading.
    #[test]
    fn a_pathological_path_still_has_a_name() {
        let odd = Dialogue::Blocked {
            path: PathBuf::from("/"),
            rule: String::from("test"),
            detail: None,
        };
        assert!(!odd.name().is_empty());
    }
}
