//! What ricedir and an agent say to each other.
//!
//! This crate is the whole interface. It holds the messages and the socket
//! path, and nothing that can read or change a file. That is deliberate: the
//! MCP program depends on this crate alone, so the compiler stops it from
//! calling anything in the window. A rule in a document is weaker.
//!
//! One request is one line of JSON. One response is one line of JSON. A shell
//! script with `socat` is a first-class client, which is why the wire is a
//! line and not a framed binary format.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The version of these messages.
///
/// Sent when a client connects. A window that speaks a different version says
/// so and closes, rather than half-understanding what it is asked.
pub const VERSION: u32 = 1;

/// What kind of thing a request is.
///
/// The rule the whole design rests on: only `File` needs a person. It is
/// declared once, here, so no transport can decide differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Answers a question about the filesystem. Changes nothing.
    Reading,
    /// Answers a question about what the window is showing. Changes nothing.
    Session,
    /// Changes what the window shows. Touches no file, so it acts at once --
    /// and it is visible on screen, which is its own review.
    View,
    /// Changes a file. Builds a plan and waits for a person.
    File,
}

/// Something an agent asks ricedir to do.
///
/// Deliberately not an open-ended command string. Each request is a named
/// thing with typed arguments, so a window can refuse one it does not know
/// rather than guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum Request {
    /// What this connection may do, and which version is spoken.
    Hello {
        version: u32,
        agent: String,
    },

    // --- reading -----------------------------------------------------------
    ListDir {
        path: PathBuf,
        limit: Option<usize>,
    },
    Stat {
        path: PathBuf,
    },
    Mime {
        path: PathBuf,
    },
    /// What the scan chain says about a file, without opening it.
    ScanVerdict {
        path: PathBuf,
    },

    // --- session -----------------------------------------------------------
    /// What is selected, which is what makes "copy these" a sentence with a
    /// meaning.
    Selection,
    /// Every open directory.
    Buffers,
    /// Where the keyboard cursor is.
    Cursor,
    /// Which rows are on screen.
    Visible,

    // --- view --------------------------------------------------------------
    Go {
        path: PathBuf,
    },
    SetSelection {
        paths: Vec<PathBuf>,
    },
    Filter {
        text: String,
    },
}

impl Request {
    /// What kind of request this is.
    ///
    /// A `match` with no wildcard arm, so a new request cannot be added
    /// without somebody deciding what kind it is.
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Hello { .. }
            | Self::ListDir { .. }
            | Self::Stat { .. }
            | Self::Mime { .. }
            | Self::ScanVerdict { .. } => Kind::Reading,

            Self::Selection | Self::Buffers | Self::Cursor | Self::Visible => Kind::Session,

            Self::Go { .. } | Self::SetSelection { .. } | Self::Filter { .. } => Kind::View,
        }
    }

    /// The name an MCP tool list shows.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::ListDir { .. } => "list_dir",
            Self::Stat { .. } => "stat",
            Self::Mime { .. } => "mime",
            Self::ScanVerdict { .. } => "scan_verdict",
            Self::Selection => "selection",
            Self::Buffers => "buffers",
            Self::Cursor => "cursor",
            Self::Visible => "visible",
            Self::Go { .. } => "go",
            Self::SetSelection { .. } => "set_selection",
            Self::Filter { .. } => "filter",
        }
    }
}

/// One entry, as an agent sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub directory: bool,
    pub size: u64,
    pub mime: Option<String>,
}

/// One open directory, as an agent sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buffer {
    pub index: usize,
    pub path: PathBuf,
    pub entries: usize,
    pub selected: usize,
    pub filter: String,
}

/// What ricedir says back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "response", rename_all = "kebab-case")]
pub enum Response {
    Hello {
        version: u32,
    },

    /// A listing, and whether it was cut short.
    ///
    /// `truncated` is reported rather than the list quietly ending: an agent
    /// that reads 500 of 20000 entries and thinks it saw them all will act on
    /// a directory it does not know.
    Entries {
        entries: Vec<Entry>,
        truncated: bool,
    },

    Entry(Entry),
    Mime {
        mime: Option<String>,
    },
    Verdict {
        verdict: String,
        rule: Option<String>,
    },
    Buffers(Vec<Buffer>),
    Cursor {
        buffer: usize,
        row: usize,
        path: Option<PathBuf>,
    },
    /// A view request was carried out.
    Done,
    /// The request was understood and refused, with the reason.
    Refused {
        reason: String,
    },
    /// The request was not understood.
    Error {
        message: String,
    },
}

/// Where the socket lives.
///
/// One per Wayland display, so two sessions on one machine do not talk to each
/// other's window. `$XDG_RUNTIME_DIR` because it is per-user, mode 0700 and
/// cleaned up at logout.
pub fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| String::from("default"));

    // A display name is a filename, and `wayland-1` normally is one. Refuse
    // anything with a separator in it rather than writing a socket somewhere
    // unexpected.
    if display.contains('/') || display.contains("..") {
        return None;
    }

    Some(PathBuf::from(runtime).join(format!("ricedir/{display}.sock")))
}

/// Read one message from a line.
pub fn decode<T: for<'a> Deserialize<'a>>(line: &str) -> Result<T, String> {
    serde_json::from_str(line).map_err(|error| error.to_string())
}

/// Write one message as a line, with the newline.
pub fn encode<T: Serialize>(message: &T) -> Result<String, String> {
    let mut line = serde_json::to_string(message).map_err(|error| error.to_string())?;
    line.push('\n');
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A message has to survive the wire both ways, or the two programs
    /// disagree about what was said.
    #[test]
    fn a_request_survives_the_wire() {
        let sent = Request::ListDir {
            path: PathBuf::from("/tmp/holiday photos"),
            limit: Some(500),
        };

        let line = encode(&sent).expect("should encode");
        assert!(line.ends_with('\n'), "a message is one line");

        let back: Request = decode(line.trim()).expect("should decode");
        assert_eq!(back, sent);
    }

    /// A filename is bytes, and a name with a quote or a newline in it must
    /// not be able to end the message early.
    #[test]
    fn a_hostile_name_survives_the_wire() {
        let nasty = PathBuf::from("/tmp/with\na\"quote\" and \\ backslash");
        let sent = Request::Stat {
            path: nasty.clone(),
        };

        let line = encode(&sent).expect("should encode");
        assert_eq!(line.matches('\n').count(), 1, "one line, always");

        let Request::Stat { path } = decode(line.trim()).expect("should decode") else {
            panic!("wrong request came back");
        };
        assert_eq!(path, nasty);
    }

    /// Only `File` needs a person, and nothing else may claim to.
    #[test]
    fn only_file_requests_need_a_person() {
        assert_eq!(Request::Selection.kind(), Kind::Session);
        assert_eq!(Request::Cursor.kind(), Kind::Session);
        assert_eq!(
            Request::Go {
                path: PathBuf::from("/tmp")
            }
            .kind(),
            Kind::View
        );
        assert_eq!(
            Request::Stat {
                path: PathBuf::from("/tmp")
            }
            .kind(),
            Kind::Reading
        );
    }

    /// No request in this version writes anything. Writing arrives in M4 with
    /// the plan mechanism, and until then a `File` request cannot be built.
    #[test]
    fn nothing_here_can_change_a_file_yet() {
        for request in [
            Request::Selection,
            Request::Buffers,
            Request::Cursor,
            Request::Visible,
            Request::Go {
                path: PathBuf::from("/"),
            },
            Request::Filter {
                text: String::new(),
            },
            Request::SetSelection { paths: Vec::new() },
        ] {
            assert_ne!(request.kind(), Kind::File, "{} writes", request.name());
        }
    }

    /// A truncated listing must say so. An agent that reads 500 of 20000 and
    /// believes it saw the directory will act on one it does not know.
    #[test]
    fn a_cut_listing_admits_it() {
        let response = Response::Entries {
            entries: Vec::new(),
            truncated: true,
        };
        let line = encode(&response).expect("should encode");
        assert!(line.contains("\"truncated\":true"));
    }

    /// The socket is per display, so two sessions do not share a window.
    #[test]
    fn the_socket_is_per_display() {
        // SAFETY: single-threaded test, and both variables are restored below.
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
            std::env::set_var("WAYLAND_DISPLAY", "wayland-2");
        }
        assert_eq!(
            socket_path(),
            Some(PathBuf::from("/run/user/1000/ricedir/wayland-2.sock"))
        );

        // A display name is a filename. One with a separator would put the
        // socket somewhere nobody asked for.
        unsafe {
            std::env::set_var("WAYLAND_DISPLAY", "../../etc/evil");
        }
        assert_eq!(socket_path(), None);

        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
        }
    }
}
