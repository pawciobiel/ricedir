//! Lets an agent drive a running ricedir.
//!
//! This program speaks MCP on its standard input and ricedir's own protocol on
//! a socket. It is a translator, not a second file manager: it asks the window
//! to do things and reports what the window says.
//!
//! It cannot do more than that, and that is the point. Look at `Cargo.toml`:
//! it depends on `ricedir-protocol` and not on `ricedir`. Nothing that reads
//! or changes a file is linked into it, so "file tools refuse when no window
//! is running" is a compile error here rather than a rule somebody has to
//! remember. See the note at the top of the workspace `Cargo.toml`.
//!
//! The socket is M3. Until it exists this program says so and stops, which is
//! honest and keeps the crate boundary compiled and tested from the start.

use std::os::unix::net::UnixStream;

fn main() {
    let Some(socket) = ricedir_protocol::socket_path() else {
        eprintln!("ricedir-mcp: no XDG_RUNTIME_DIR, so there is no socket to find");
        std::process::exit(1);
    };

    match UnixStream::connect(&socket) {
        Ok(_) => {
            // M3 puts the MCP loop here.
            eprintln!("ricedir-mcp: found {}", socket.display());
            eprintln!("ricedir-mcp: the tool loop is M3; see TODO.md");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("ricedir-mcp: no ricedir at {}: {error}", socket.display());
            eprintln!("ricedir-mcp: start ricedir first. Reading tools need a window,");
            eprintln!("ricedir-mcp: and file tools need one so a person can watch.");
            std::process::exit(1);
        }
    }
}
