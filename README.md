# ricedir

**A file manager you can watch an agent use.**
Mouse-driven, one TOML file, no stylesheet, no D-Bus. Built with
[iced](https://iced.rs) for Hyprland, sway and niri.

> **Nothing is drawn yet.** This is a design and a plan, not a program. There
> is no screenshot below because there is nothing to photograph.
> [`TODO.md`](TODO.md) is the plan, split into milestones and tasks, and it
> says what was decided and why. The first code is M1.

Say to your assistant *"select all the images in Pictures, make thumbnails and
move them to backup"*, and watch 240 files light up in the window before
anything moves, read the plan it proposes, accept it, and see the copy run in a
jobs panel with a cancel button and your agent's name on the row.

That is the whole idea. Everything else is a good file manager underneath it.

## Why another file manager?

Two reasons, and the second is the real one.

**Opening a file should not be a guessing game.** The usual way to come to harm
through a file manager is dull: something arrives from a site that should not
be trusted, it gets double-clicked, and the manager walks a chain — extension,
MIME database, a `.desktop` file's `Exec=` line, a shell to expand it — handing
the file to whatever that chain named, with whatever arguments the file's own
name produced. Every link is data an attacker touched. ricedir uses a list of
programs you wrote instead, and when there is no entry it asks rather than
guessing.

**An agent with a shell does not need a file manager.** It has `cp`. So a file
manager that an agent can drive is only worth building if driving it is
*better* than `cp` — and the thing that makes it better is that you are
watching. You see what got selected before anything is moved. You read the plan
and accept it. You get progress, a cancel button, an undo, and a log of which
agent did what. None of that exists at a shell prompt.

Which is why ricedir has no headless mode and never will.

## What it will do

| | |
| --- | --- |
| **Tiles over buffers** | A buffer is an open directory; a tile is a viewport onto one. One tile by default. Split when you want, close a tile without losing the buffer. Emacs' model, not tabs. |
| **Mouse first** | Double-click, right-click menus, a toolbar, and arrows that do the obvious thing. Bindings are a flat table you can change. No modes, no chords, no language to learn. |
| **Switchable layouts** | Detail rows, compact grid, large icons, with the detail columns chosen in the config. |
| **Background jobs** | Copy, move, delete and trash run on their own threads with real progress, pause, cancel and undo. A 40 GB copy does not stop you reading another directory. |
| **A list that does not care how big the directory is** | Only the visible rows are laid out. 100k entries scroll like 100. |
| **Agent-driven, in the open** | A JSON socket and an MCP server, so `claude`, Goose or a voice script can drive it — always visibly, always with a person accepting anything that writes. |

## Watching an agent work

ricedir serves MCP, so anything that speaks it can drive the window:

```sh
claude mcp add ricedir -- ricedir --mcp
```

The tools come in four kinds, and the difference is the design:

- **Reading** — `list_dir`, `stat`, `mime`, `search`. Answers questions.
- **Session** — `selection`, `cursor`, `buffers`, `visible`. This is what makes
  *"copy these two"* a sentence with a meaning. No filesystem MCP server has
  it, and it is most of why voice is worth wanting: speech is full of pointing
  words, and pointing words need somewhere to point.
- **View** — `go`, `set_selection`, `filter`, `split`. Changes what the window
  shows, touches no file, happens immediately. Watching the selection change is
  its own review, and it costs no dialogue.
- **File** — `copy`, `move`, `trash`, `transform`. Builds a plan and stops.
  Sources, destination, counts, bytes, conflicts, on screen. **A person always
  accepts it**; there is no setting that lets an agent accept its own plan.

Agent actions are paced so you can follow them, an agent's selection is tinted
differently from yours, a strip shows the calls as they arrive, every job says
who asked for it, and one switch detaches every agent at once.

ricedir contains no model, no prompt, no tool-call loop and no API key, and it
never will. It is a thing agents use, not an agent.

## Opening files

A handler is an argv vector, never a command string:

```toml
[[handler]]
mime = "video/*"
run = ["flatpak", "run", "io.mpv.Mpv", "--", "{path}"]

[[handler]]
mime = "text/*"
run = ["foot", "-e", "nvim", "--", "{path}"]
```

There is no shell, so there is no quoting to get wrong and a file called
`; rm -rf ~ #` is a file with a silly name. `--` goes before the path, so a
file called `--config=…` is a path and not an option. Flatpak handlers are
preferred where the app is installed, because they bring a sandbox nobody here
had to write, and the first run writes a config from what you actually have.

`.desktop` files are shown, never executed. The executable bit is not a route
to launching anything. When no handler matches, ricedir names the type and
offers to choose a program, use `xdg-open` once, or use `xdg-open` always —
and "always" writes a real handler into your config, so the fallback teaches
the config rather than becoming a permanent hole.

Before any of that, a scan chain gets an opinion:

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

None of this protects you from yourself — anyone who can edit the config can
already run anything as you. It protects you from the *file*, which is the part
that actually came from somewhere else.

## Install

Not yet. When there is something to install it will be
`cargo install --git https://github.com/pawciobiel/ricedir`, for the same
reason as ricebar: publishing to crates.io means signing in through GitHub and
granting that login access the project does not need.

## What it will not do

Worth knowing early:

- **No system tray, no automount, no udisks.** ricedir does not speak D-Bus.
  Mounts are read from `/proc/self/mountinfo` and shown; mounting a disk is
  something your desktop already does.
- **No dragging files out to other applications.** winit has no drag-out on
  Wayland, so dropping *into* ricedir works and dragging *out* cannot. Not a
  bug, and not fixable from here.
- **No browsing inside archives as though they were folders.** Extract, work,
  re-archive.
- **No thumbnails at first**, and never in this process when they arrive. Media
  decoders are the classic memory-safety hazard and a file manager is where
  untrusted media lands, so thumbnailing happens in a sandboxed helper or not
  at all. It is not competing with an image viewer.
- **No unattended agent work.** If you want a machine to move files with nobody
  watching, it has a filesystem and does not need this.
- **Nothing, yet.** It is a plan. [`TODO.md`](TODO.md) is honest about which
  parts are decided, which are guesses, and which depend on a survey of Linux
  tooling that has not happened.

Prior art worth your time if ricedir is not what you want:
[Nautilus](https://apps.gnome.org/Nautilus/) and
[Dolphin](https://apps.kde.org/dolphin/) if you want a finished file manager,
[Yazi](https://github.com/sxyazi/yazi) if you want a fast one in a terminal,
[COSMIC Files](https://github.com/pop-os/cosmic-files) if you want one built on
iced that already exists.

## Contributing

Not yet — there is no code to contribute to. When there is:

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

[`TODO.md`](TODO.md) is the design record, ordered by value, and
[`CLAUDE.md`](CLAUDE.md) is what an agent working on the repo should read
first.

## License

MIT
