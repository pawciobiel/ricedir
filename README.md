# ricedir

**A file manager you can watch an agent use.**
Mouse-driven, one TOML file, no D-Bus. Built with [iced](https://iced.rs) for
Hyprland, sway and niri.

> **Early. It runs, but it cannot change a file yet.** You can browse, open
> files and drive it from a socket. Copy, move and delete arrive in M2.
> [`TODO.md`](TODO.md) is the plan and the design record.

Say to your assistant *"select all the images in Pictures, make thumbnails and
move them to backup"*. You then see 240 files light up in the window. Nothing
moves yet. You read the plan and you accept it. The copy then runs in a jobs
panel, with a cancel button and the name of the agent on the row.

Everything else is a good file manager underneath that.

## Why another file manager?

There are two reasons. The second one matters more.

**Opening a file must not be a guess.** Here is the usual way to come to harm.
A file arrives from a site you do not trust. You double-click it. The file
manager then follows a chain: the extension, the MIME database, the `Exec=`
line of a `.desktop` file, and a shell to expand that line. Each link in the
chain is data that an attacker wrote. ricedir uses a list of programs that you
wrote. If no entry matches, it asks you.

**An agent with a shell does not need a file manager.** It has `cp`. A file
manager for an agent is only useful if it is better than `cp`. It is better
because you watch it. You see the selection before anything moves. You read
the plan and you accept it. You get progress, a cancel button, an undo, and a
log of what each agent did. A shell prompt gives you none of these.

For this reason ricedir has no headless mode. It will never have one.

## What works now

| Part | State |
| --- | --- |
| Listing a directory | Any size. 100,000 entries scroll as fast as 100. |
| Tiles and buffers | Split, close, and point a tile at any open directory. |
| Layouts | Detail rows, compact list, and a grid of icons. Each tile keeps its own. |
| Selection | Click, Ctrl-click, Shift-click, Ctrl-A, invert, and a rubber band. |
| Places | Home, the XDG directories, mounted disks, and your bookmarks. |
| Path bar | Breadcrumbs, and a text face you can select a part of and copy. |
| Filter and sort | Narrow as you type, and sort by name, size, date or type. |
| Watching | A change on disk updates the listing. No refresh needed. |
| Opening files | The handler table and the scan chain, with four refusals. |
| Menus | On a file, on empty space, and on a place. Each one is different. |

Not built yet: copy, move, delete, trash, undo (M2), the agent socket and MCP
(M3), thumbnails (M7).

## How it is built

**One process, one window, many tiles.** `iced::daemon` runs the loop. A
second window is a second window, not a second program. This matters for the
agent socket: an agent asks about "the selection", and two processes would
give two answers.

**A buffer is an open directory. A tile shows one.** `App.buffers` is a flat
list and a tile holds an index into it. A directory open in two tiles is read
once and watched once. This is the model emacs uses. It is not tabs.

**Every request is an action.** Menus, the toolbar, keys, the socket and MCP
all go through one function, `action::dispatch`. There is no second door. This
is what stops an agent from reaching a file without the scan chain, the plan
and the log.

**Actions have four kinds.** *Reading* and *session* actions change nothing.
*View* actions change what the window shows and act at once. *File* actions
build a plan and wait for a person. The kind is declared once, in the
registry, so every transport obeys it.

**File work never runs on the loop.** Reading a directory, reading a changed
file, and later copying a file all run on other threads. They send their
results back as messages. A slow or hung mount can then stall by itself
without stopping the window.

**Three crates, one repository.**

```
crates/protocol/   the messages an agent sends and receives, and nothing else
crates/ricedir/    the file manager
crates/mcp/        MCP on stdin and stdout, forwarded to the socket
```

`ricedir-mcp` does not depend on `ricedir`. It therefore cannot read or change
a file by itself; it must ask the running window. The rule is enforced by the
compiler, not by a comment.

## Watching an agent work

ricedir serves MCP, so any client that speaks it can drive the window:

```sh
claude mcp add ricedir -- ricedir --mcp
```

The tools come in four kinds:

- **Reading** — `list_dir`, `stat`, `mime`, `search`. These answer questions.
- **Session** — `selection`, `cursor`, `buffers`, `visible`. These give
  *"copy these two"* a meaning. No filesystem MCP server has them. They are
  most of the reason to want voice control: speech is full of words that
  point, and a pointing word needs a target.
- **View** — `go`, `set_selection`, `filter`, `split`. These change what the
  window shows. They touch no file, so they act at once. You see the selection
  change, and that is the review.
- **File** — `copy`, `move`, `trash`, `transform`. These build a plan and
  stop. The plan shows the sources, the destination, the counts, the bytes and
  the conflicts. **A person always accepts the plan.** No setting lets an
  agent accept its own plan.

ricedir paces agent actions so that you can follow them. It tints an agent's
selection differently from yours. A strip shows the calls as they arrive.
Each job says who asked for it. One switch detaches every agent at once.

ricedir holds no model, no prompt, no tool-call loop and no API key. It never
will. Agents use it. It is not an agent.

## Opening files

A handler is a list of arguments. It is never a command line:

```toml
[[handler]]
mime = "video/*"
run = ["flatpak", "run", "io.mpv.Mpv", "--", "{path}"]

[[handler]]
mime = "text/*"
run = ["foot", "-e", "nvim", "--", "{path}"]
```

There is no shell. There is therefore no quoting to get wrong, and a file
named `; rm -rf ~ #` is only a file with an odd name. `--` goes before the
path, so a file named `--config=…` is a path and not an option. ricedir
prefers a flatpak handler when the application is installed, because it adds
a sandbox. The first run writes a config from the programs you have.

ricedir shows `.desktop` files. It never runs them. The executable bit is not
a way to start a program.

When no handler matches, ricedir names the type. It then offers four choices:
choose a program, use `xdg-open` once, use `xdg-open` always, or cancel.
"Always" writes a real handler into your config.

Before any of this, a scan chain gives an opinion:

```toml
[[scan]]
name = "double extension"
match = '''\.(pdf|jpe?g|docx?)\.(exe|scr|js|sh)$'''
verdict = "block"

[[scan]]
name = "extension lies about the content"
kind = "magic-mismatch"
verdict = "warn"
```

This does not protect you from yourself. Anyone who can edit the config can
already run any program as you. It protects you from the *file*, which is the
part that came from somewhere else.

## Install

```sh
cargo install --git https://github.com/pawciobiel/ricedir
```

ricedir is not on crates.io. Publishing there needs a GitHub login with more
account access than this project needs.

## What it will not do

- **No system tray, no automount, no udisks.** ricedir does not speak D-Bus.
  It reads mounts from `/proc/self/mountinfo` and shows them.
- **No dragging files out to other applications.** winit cannot do this on
  Wayland. Dropping *into* ricedir works. Dragging *out* cannot.
- **No browsing inside an archive as though it were a folder.** Extract it,
  work, and archive it again.
- **No thumbnails at first.** When they arrive they will run in a sandboxed
  helper, never in this process. Media decoders are a common source of
  memory-safety faults, and a file manager is where untrusted media lands.
  ricedir does not compete with an image viewer.
- **No unattended agent work.** A machine that moves files with nobody
  watching has a filesystem already. It does not need this.

Look at these instead if ricedir is not what you want:
[Nautilus](https://apps.gnome.org/Nautilus/) and
[Dolphin](https://apps.kde.org/dolphin/) are finished file managers.
[Yazi](https://github.com/sxyazi/yazi) is a fast one in a terminal.
[COSMIC Files](https://github.com/pop-os/cosmic-files) is built on iced and
already exists.

## Contributing

Run all four checks before you send anything:

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

clippy runs `pedantic` and `nursery`. The exceptions are listed in
`Cargo.toml`, and each one gives its reason.

[`TODO.md`](TODO.md) is the design record. It is ordered by value and it says
what was decided and why. [`CLAUDE.md`](CLAUDE.md) is what an agent working on
this repository reads first.

## License

MIT
