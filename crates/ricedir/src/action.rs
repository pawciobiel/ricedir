//! Every request, from wherever it came.
//!
//! A menu item, a key, a click and a line of JSON on the socket all become one
//! `Action` and go through [`dispatch`]. There is one door, so a request
//! cannot reach a path that skips the scan chain, the plan, the job engine or
//! the audit log by arriving a different way.
//!
//! `ricedir_protocol::Request` is the same idea on the wire. [`Action::of`]
//! turns one into the other, and it is the only place that conversion happens.

use std::path::PathBuf;

use iced::Task;
use iced::widget::pane_grid::Axis;
use ricedir_protocol::{Kind, Request};

use crate::app::{App, Message};

/// One thing ricedir can be asked to do.
///
/// A superset of [`Request`]: some of these only a person can ask for, because
/// they are about a pointer or a modal and mean nothing to an agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    // --- session: change nothing ------------------------------------------
    Selection,
    Buffers,
    Cursor,

    // --- view: act at once, touch no file ---------------------------------
    /// Put the cursor on a row and select only it.
    Select {
        buffer: usize,
        row: usize,
    },
    /// Add or remove one row.
    Toggle {
        buffer: usize,
        row: usize,
    },
    /// Select from the anchor to a row.
    Extend {
        buffer: usize,
        row: usize,
    },
    SelectAll {
        buffer: usize,
    },
    /// Select what is not selected.
    Invert {
        buffer: usize,
    },
    /// Select the cells a rubber band covered.
    Band {
        buffer: usize,
        rows: std::ops::Range<usize>,
        /// `None` means whole rows, which is what the list layouts drag.
        columns: Option<std::ops::Range<usize>>,
        /// How many cells sit side by side, from the widget that knows.
        across: usize,
        add: bool,
    },
    /// Show a directory.
    Go {
        buffer: usize,
        path: PathBuf,
    },
    Back {
        buffer: usize,
    },
    Forward {
        buffer: usize,
    },
    /// Up one level.
    Leave {
        buffer: usize,
    },
    /// Narrow the listing.
    Filter {
        buffer: usize,
        text: String,
    },
    /// Show or hide the filter box. A person only: an agent filters by text.
    Filtering(bool),
    /// Turn the path bar over to its text face, or back to its breadcrumbs.
    ///
    /// A person only. An agent already names a path in `Go` and has nothing
    /// to gain from a text box being on screen.
    TypingPath {
        buffer: usize,
        typing: bool,
    },
    /// Put away whatever is in front. A person only.
    Escape,
    /// Show a menu at a point on screen. A person only: a pointer and a
    /// button are the only things that have a point.
    Menu {
        kind: crate::app::MenuKind,
        buffer: usize,
        at: (f32, f32),
    },
    /// Sort by this field, or turn the order round when it is already the one.
    SortBy(crate::config::Sort),
    ReverseSort,
    /// Put the path of what is selected on the clipboard.
    ///
    /// The thing people want a path bar for, and a menu is where it lives
    /// until the path bar grows its text face in M6.
    CopyPath {
        buffer: usize,
    },
    /// Add a directory to the favourite places.
    ///
    /// `path` names a subdirectory the menu was opened over; `None` means the
    /// directory the buffer is showing, which is what Ctrl+D asks for and
    /// what a browser's star does. One action rather than two, because the
    /// only difference between them is which path.
    Bookmark {
        buffer: usize,
        path: Option<PathBuf>,
    },
    /// Take one back out again.
    ///
    /// Built at the same time as the menu it lives in. Adding without
    /// removing makes every mistake permanent, which is worse than not
    /// offering the button.
    Unbookmark {
        path: PathBuf,
    },
    /// Split the focused tile and show a directory in the new half.
    ///
    /// Not `Split` followed by `Go`: the two would be separate actions, and
    /// the second would have to name a buffer that the first has only just
    /// created.
    OpenBeside {
        path: PathBuf,
    },
    /// Show or hide the files whose names start with a dot, in one buffer.
    ///
    /// Per buffer for the same reason as [`Action::Layout`]: a tile opened on
    /// a `.config` is no reason for the tile beside it to fill up with `.git`
    /// and `.cache`.
    ShowHidden {
        buffer: usize,
        showing: bool,
    },
    /// Arrange one buffer's entries a different way.
    ///
    /// One buffer, not the window: two tiles wanting different views is the
    /// ordinary case, and a switch that changed both made the second useless.
    Layout {
        buffer: usize,
        layout: crate::config::Layout,
    },
    /// Point the focused tile at a buffer that is already open.
    ///
    /// The other half of the emacs model. Splitting gives the new tile a
    /// buffer of its own, which is what a file manager usually wants; this is
    /// how two tiles come to *share* one listing and one watcher.
    ShowBuffer {
        buffer: usize,
    },
    /// Forget a buffer no tile is showing.
    ///
    /// A long session opens directories and closes tiles, and without this
    /// the list grows for as long as the window is open.
    CloseBuffer {
        buffer: usize,
    },
    /// Split the focused tile in two.
    Split(Axis),
    /// Close the focused tile. The buffer it showed stays open.
    CloseTile,
    /// Move the keyboard to the next tile, or the one before.
    NextTile,
    PreviousTile,
    /// Read the directory again.
    Relist {
        buffer: usize,
    },

    // --- opening ----------------------------------------------------------
    /// Enter a directory, or open a file through the scan chain and the
    /// handler table. Never a shortcut past either.
    Activate {
        buffer: usize,
        row: usize,
    },
}

impl Action {
    /// What kind this is, and so what it is allowed to do without asking.
    ///
    /// The kinds are defined once in `ricedir-protocol` and read here, so the
    /// window and the wire cannot disagree about which requests need a person.
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Selection | Self::Buffers | Self::Cursor => Kind::Session,

            Self::Select { .. }
            | Self::Toggle { .. }
            | Self::Extend { .. }
            | Self::SelectAll { .. }
            | Self::Invert { .. }
            | Self::Band { .. }
            | Self::Go { .. }
            | Self::Back { .. }
            | Self::Forward { .. }
            | Self::Leave { .. }
            | Self::Filter { .. }
            | Self::Filtering(_)
            | Self::TypingPath { .. }
            | Self::Escape
            | Self::Menu { .. }
            | Self::SortBy(_)
            | Self::ReverseSort
            | Self::CopyPath { .. }
            | Self::ShowHidden { .. }
            | Self::Layout { .. }
            | Self::Split(_)
            | Self::CloseTile
            | Self::NextTile
            | Self::PreviousTile
            | Self::OpenBeside { .. }
            | Self::ShowBuffer { .. }
            | Self::CloseBuffer { .. }
            | Self::Relist { .. } => Kind::View,

            // A bookmark writes a file, but ricedir's own, not one of the
            // person's. Nothing in the plan mechanism is about protecting
            // ricedir from itself, so it acts at once like any other view.
            Self::Bookmark { .. } | Self::Unbookmark { .. } => Kind::View,

            // Opening runs a program. It is not a file *write*, so it is not
            // `Kind::File`, but it is the one view-shaped action with a scan
            // chain in front of it -- see `open::plan`.
            Self::Activate { .. } => Kind::View,
        }
    }

    /// Turn a request off the wire into an action, for a named buffer.
    ///
    /// The only conversion point. Anything the wire can ask for has to be
    /// written here, which is what stops a transport inventing a request the
    /// registry has never seen.
    pub fn of(request: &Request, buffer: usize) -> Result<Self, String> {
        match request {
            Request::Selection => Ok(Self::Selection),
            Request::Buffers => Ok(Self::Buffers),
            Request::Cursor => Ok(Self::Cursor),

            Request::Go { path } => Ok(Self::Go {
                buffer,
                path: path.clone(),
            }),
            Request::Filter { text } => Ok(Self::Filter {
                buffer,
                text: text.clone(),
            }),

            // Answered by reading the filesystem rather than by changing the
            // window, so they never become an action.
            Request::Hello { .. }
            | Request::ListDir { .. }
            | Request::Stat { .. }
            | Request::Mime { .. }
            | Request::ScanVerdict { .. }
            | Request::Visible
            | Request::SetSelection { .. } => Err(format!(
                "`{}` is answered without an action",
                request.name()
            )),
        }
    }
}

/// Carry out one action.
///
/// Every caller ends up here: the list widget, the menus, the keys, and in M3
/// the socket. `app::update` holds the message plumbing; this holds what the
/// messages mean.
pub fn dispatch(app: &mut App, action: Action) -> Task<Message> {
    // A `File` action must never reach the plumbing below without a plan and a
    // person. None exists yet -- M4 adds them -- so the check is here from the
    // start rather than being remembered later.
    if action.kind() == Kind::File {
        return crate::app::refuse(
            app,
            String::from("that would change a file, and plans arrive in M4"),
        );
    }

    // Any action but opening a menu closes the one that is open. A menu that
    // outlives the thing it was about is a menu that acts on the wrong file.
    if !matches!(action, Action::Menu { .. }) {
        crate::app::close_menu(app);
    }

    crate::app::carry_out(app, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every action has to declare a kind. A `match` with no wildcard is what
    /// forces that, and this is the test that notices if one is added with a
    /// wildcard to make the compiler quiet.
    #[test]
    fn every_action_declares_a_kind() {
        let actions = [
            Action::Selection,
            Action::Buffers,
            Action::Cursor,
            Action::Select { buffer: 0, row: 0 },
            Action::Toggle { buffer: 0, row: 0 },
            Action::Extend { buffer: 0, row: 0 },
            Action::SelectAll { buffer: 0 },
            Action::Invert { buffer: 0 },
            Action::Band {
                buffer: 0,
                rows: 0..1,
                columns: None,
                across: 1,
                add: false,
            },
            Action::Go {
                buffer: 0,
                path: PathBuf::from("/"),
            },
            Action::Back { buffer: 0 },
            Action::Forward { buffer: 0 },
            Action::Leave { buffer: 0 },
            Action::Filter {
                buffer: 0,
                text: String::new(),
            },
            Action::Filtering(true),
            Action::TypingPath {
                buffer: 0,
                typing: true,
            },
            Action::Escape,
            Action::Menu {
                kind: crate::app::MenuKind::Context { row: Some(0) },
                buffer: 0,
                at: (0.0, 0.0),
            },
            Action::Menu {
                kind: crate::app::MenuKind::Place { index: 0 },
                buffer: 0,
                at: (0.0, 0.0),
            },
            Action::SortBy(crate::config::Sort::Size),
            Action::ReverseSort,
            Action::CopyPath { buffer: 0 },
            Action::Bookmark {
                buffer: 0,
                path: Some(PathBuf::from("/tmp")),
            },
            Action::Bookmark {
                buffer: 0,
                path: None,
            },
            Action::Unbookmark {
                path: PathBuf::from("/tmp"),
            },
            Action::OpenBeside {
                path: PathBuf::from("/tmp"),
            },
            Action::ShowHidden {
                buffer: 0,
                showing: true,
            },
            Action::Layout {
                buffer: 0,
                layout: crate::config::Layout::Icons,
            },
            Action::ShowBuffer { buffer: 0 },
            Action::CloseBuffer { buffer: 0 },
            Action::Split(Axis::Vertical),
            Action::CloseTile,
            Action::NextTile,
            Action::PreviousTile,
            Action::Relist { buffer: 0 },
            Action::Activate { buffer: 0, row: 0 },
        ];

        for action in actions {
            let kind = action.kind();
            assert_ne!(
                kind,
                Kind::File,
                "{action:?} claims to write, and nothing in M1 may"
            );
        }
    }

    /// The wire and the window must agree about which requests need a person.
    /// A request that is `View` on the wire cannot become something heavier
    /// once it is inside.
    #[test]
    fn a_request_keeps_its_kind_through_the_conversion() {
        for request in [
            Request::Selection,
            Request::Buffers,
            Request::Cursor,
            Request::Go {
                path: PathBuf::from("/tmp"),
            },
            Request::Filter {
                text: String::from("x"),
            },
        ] {
            let action = Action::of(&request, 0).expect("should convert");
            assert_eq!(
                action.kind(),
                request.kind(),
                "`{}` changed kind on the way in",
                request.name()
            );
        }
    }

    /// A request the registry has no action for must be refused by name, not
    /// silently turned into something near it.
    #[test]
    fn an_unknown_request_is_refused_by_name() {
        let problem = Action::of(
            &Request::Stat {
                path: PathBuf::from("/tmp"),
            },
            0,
        )
        .expect_err("stat is answered by reading, not by acting");

        assert!(problem.contains("stat"), "unhelpful message: {problem}");
    }
}
