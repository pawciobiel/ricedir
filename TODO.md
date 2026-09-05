# TODO

Ordered roughly by value. Notes record what was already established, so the
work does not have to be rediscovered. Milestones are the plan; the checkboxes
under each are the tasks.

Nothing is built yet. Everything below `## Decided before any code` was settled
in conversation on 2026-09-03 and should not be reopened without a reason.

## What ricedir is

A file manager for Wayland, in Rust, on iced. Mouse-first, with arrow keys that
do the obvious thing. It opens files with programs a config names, and when
nothing is named it asks rather than guessing. Long file work runs on threads
of its own, so a copy of 40 GB does not stop you looking at another directory.

And it is a file manager an agent can sit in front of, in both directions: it
serves MCP so an assistant can see the directory you are looking at and act on
the files you have selected, and it consumes MCP so a local model, a scanner or
a speech-to-text process can be plugged in without ricedir knowing what any of
them are. Say "copy these two to the backup drive" and something else does the
listening; ricedir's part is to know what "these" means, to show you the plan,
and to do it once you agree.

Sibling of [ricebar](https://github.com/pawciobiel/ricebar): one TOML file, no
stylesheet, no D-Bus, and it runs on Hyprland, sway and niri.

## Decided before any code

- [x] **Tiles and buffers, not tabs.** A *buffer* is an open directory. A
      *tile* is a viewport onto one. Emacs' model, and the reason it beats tabs
      here: a tile can be split, closed, or pointed at a different buffer
      without the buffer's listing, watcher, scroll position or selection being
      thrown away. One tile is the default, so ricedir works as a plain
      single-directory window without anyone opting out of anything.

      `iced_widget::pane_grid` already does tiled splitting with draggable
      dividers, so the tile half is mostly configuration rather than code.

      Buffers live in one flat `Vec<Buffer>` and tiles hold a `usize` into it,
      exactly as ricebar's bars hold indices into one module list. Two tiles
      showing the same directory then share one listing and one watcher.

- [x] **Places top left, jobs bottom left, tiles in the rest.** Fixed
      furniture, not a configurable dock: a file manager whose panels move
      around is a file manager nobody can be shown how to use. Widths are
      draggable and each panel can be hidden.

- [x] **Mouse first. No keymap language.** Double click opens, right click
      gives a context menu, a toolbar carries the common actions, and arrows,
      `Home`/`End`, `PageUp`/`PageDown`, `Enter`, `Backspace`, `F2`, `Delete`,
      `Ctrl+C`/`X`/`V` do what everyone already expects. Bindings are a flat
      TOML table so any of them can be changed, and that is as far as it goes:
      no modes, no chords, no which-key popup, no vim preset.

      The power-user route is the MCP surface, not a second keyboard language
      to learn. This was an explicit preference, not an oversight.

- [x] **Opening a file is the security boundary, and it is a list of
      programs.** No `.desktop` lookup, no shell. A config table maps a MIME
      type or a glob to an **argv vector**, and ricedir spawns exactly that
      vector with the path as one element after `--`. See `## The opener` for
      why every part of that sentence is load-bearing.

- [x] **`xdg-open` is the fallback, and it asks first.** A type with no
      handler used to mean nothing happened, which is a bad first hour with a
      program. Instead the window says the type has no handler and offers:
      choose a program, hand it to `xdg-open` this once, always use `xdg-open`
      for this type, or cancel. "Always" writes a real handler line into the
      config, so the fallback teaches the config rather than becoming a
      permanent hole, and the second time it is a named handler like any other.

      `fallback = "ask" | "xdg-open" | "none"`, defaulting to `ask`. The
      difference from the chain we are avoiding is only that the person is in
      it: `ask` means nothing runs that was not agreed to at the moment it ran.

      Three things do not change. The scan chain runs first, whichever handler
      wins. The fallback is refused outright for the types where handing over
      to the desktop's own resolution *is* the exploit — `.desktop`, `.sh`,
      `application/x-executable`, `application/x-shellscript` and anything else
      that would be run rather than opened. And first run writes handlers for
      the common types from what is actually installed, so the fallback should
      be rare rather than the normal path.

- [x] **Flatpak handlers are preferred where one is installed.** `flatpak run
      <app-id>` puts the program that parses the untrusted file in a sandbox
      that somebody else maintains, which is a better deal than anything
      ricedir can build itself. First run probes what is actually installed
      (`flatpak info <id>`) and writes the config accordingly, the same way
      ricebar probes `$PATH` before enabling a module.

      Present on this machine and worth defaulting to: `io.mpv.Mpv`,
      `org.kde.okular`, `org.videolan.VLC`, `org.gimp.GIMP`,
      `org.libreoffice.LibreOffice`, `org.xfce.ristretto`, `org.mozilla.firefox`.

- [x] **A scan chain runs before a file is opened.** An ordered list of checks
      in the config, each returning allow, warn or block: globs and regexes on
      the name, a magic-versus-extension mismatch test, size caps, and an
      external command whose exit code decides. Pluggable, because the useful
      backend differs per machine and none of it should be compiled in.

- [x] **Agents are a first-class surface, chosen over a keybinding DSL, and it
      goes both ways.** ricedir is an MCP *server* so an assistant can drive
      it, and an MCP *client* so ricedir can ask something else. Read-only by
      default, both ways. The whole of `## M3` and `## M4` is this.

- [x] **Every action has a name, and the name is the API.** Each menu item,
      toolbar button, key and agent request invokes one entry in an action
      registry — `open`, `copy`, `select`, `go`, `filter`, `split` — with typed
      arguments. One implementation, so a request cannot skip the confirmation,
      the job engine, the undo stack or the audit log by coming in through a
      different door. Cheap to do from the first commit and expensive to
      retrofit, so the registry lands in M1 even though nothing else uses it
      until M3.

- [x] **Being watched is the product, so there is no hidden mode.** The
      question that settles it: why would an agent with a shell use ricedir to
      copy a file at all? It would not. `cp` is faster and has fewer moving
      parts. The single reason to route through ricedir is that a person is
      watching it happen, can see what got selected before anything moved, and
      has the job in a panel with a cancel button.

      So a switch to hide agent activity is a switch that turns this into a
      slow `cp`, and it is not offered. **File tools refuse when no window is
      running** — the answer is "no ricedir is running; use the filesystem
      directly", not silent compliance. Reading tools still answer headless,
      which is harmless and sometimes useful.

- [x] **Watchable is a requirement, not a side effect.** An agent that fires
      twenty view actions a second is as opaque as no window at all. Four
      things follow, and they are design constraints rather than polish:
      view actions from an agent are **paced** to something a person can
      follow; the selection **animates** to its new state instead of snapping,
      because the movement is what shows you what happened; an agent's
      selection is **tinted differently** from your own, so you can tell who
      chose it; and a **trace strip** shows the last few calls arriving —
      `list_dir` → `set_selection` → `plan` — so the sequence is legible while
      it runs, not only afterwards in the audit log.

      This is also why M6's animation work is not merely eye candy: it is how
      an agent's actions become readable.

- [x] **One switch about agent activity, and it is not visibility.** The real
      annoyance is an agent yanking the directory you are reading.
      `agent-view = "follow" | "own-tile"`: follow means the agent moves your
      view and you watch over its shoulder; own-tile means it opens a tile of
      its own and works there, fully visible, without stealing your place.

- [x] **An agent sees what you see.** The tools that make this different from
      a filesystem MCP server are the *session* ones: what is selected, which
      directories are open, where the cursor is, what the filter says. That is
      what makes "copy these" resolvable, and it is what a voice command needs
      most, because speech is full of "these", "that one" and "here".

- [x] **An agent proposes; ricedir stages; you accept.** A mutating request
      does not move a byte. It builds a plan — every source, the destination,
      the counts, the total size, the conflicts — and shows it as something to
      read and accept or reject. The agent gets the plan back too, so it can
      see a clash and revise before you are ever asked. Config chooses
      `review-destructive | review-all`, defaulting to the first.

      This is the part that makes voice control of a file manager sane rather
      than alarming.

      **A person always accepts.** There is no setting that lets an agent
      accept its own plan, because deciding whether a copy needs a human is the
      agent's problem and not ricedir's: an agent that wants to work unattended
      has a filesystem and does not need us. What ricedir sells is the pause,
      so removing the pause removes the reason to be here.

- [x] **ricedir is not the agent, and does not contain one.** No model, no
      prompt, no tool-call loop, no API key, ever. The question was asked
      directly and the answer is worth writing down, because it will be asked
      again.

      A tool-call loop is the most commoditised software being written right
      now: model routing, streaming, retries, context handling. Anyone can
      build one and dozens of people are. What cannot be bought is a file
      manager with a window in front of it that can say what is selected, show
      a plan before it acts, run the work as a cancellable job and tell you
      afterwards which agent did it. That asymmetry decides where the effort
      goes.

      The practical consequence is a three-layer plan, in order of cost:

      **Existing agents, free.** `ricedir --mcp` and the `claude` CLI already
      on this machine, or Goose, which is open source, local-first and treats
      MCP servers as its extension mechanism. No agent code written at all.

      **A thin bridge, ours.** Perhaps 400 lines: record with `pw-record`,
      transcribe with whatever the M0 survey picked, send the text and
      ricedir's tool list to an OpenAI-compatible endpoint — `llama-server`,
      `ollama`, or a hosted one — and post the resulting calls to the socket.
      A loop, not a framework, and deliberately disposable. This exists so
      ricedir is demonstrable by voice without installing an agent first.

      **Extending somebody else's, only if the survey says so.** Growing our
      bridge into a real agent is the failure mode to avoid: it ages at the
      speed of the AI ecosystem rather than the speed of file managers, and it
      is three unrelated release cycles in one process. If the bridge starts
      wanting features, that is the signal to adopt Goose or a Wyoming
      pipeline instead of writing more.

- [x] **Voice is not built in.** ricedir has no audio dependency and never
      will. Speech-to-text is a separate process that posts text to ricedir's
      socket as an intent, which means any of whisper.cpp, faster-whisper,
      vosk or something not written yet plugs in without a line of ricedir
      changing. An example one ships in `dev/agents/`.

- [x] **Local agents are the default assumption, not the fallback.** A
      transport of `stdio`, `unix` or `http` covers a spawned child, a
      long-lived local daemon and a remote endpoint equally, and the config
      names agents rather than vendors. Nothing about the design prefers a
      cloud model.

- [x] **Every job says who asked for it**, and one visible switch detaches
      every agent at once. A file manager that a machine can drive needs a
      brake that a person can find without reading anything.

- [x] **File work never runs on the Elm loop.** Copy, move, delete and trash
      are blocking I/O on ordinary threads, with progress coming back through a
      `tokio::sync::mpsc` channel that a subscription drains. Not async tasks:
      there is nothing to overlap in a byte copy, and `spawn_blocking` would
      only borrow iced's runtime to do the same thing less clearly.

- [x] **Layouts switch at runtime.** Detail rows, compact grid and large icons
      behind one trait, cycled from the toolbar or a binding, remembered in the
      config. Columns in the detail layout are a config list, so `name`,
      `size`, `modified`, `mode`, `owner`, `type` can be chosen and reordered.

- [x] **The file list is a custom widget.** `iced_widget::table` builds an
      `Element` per cell up front — `cells: Vec<Element<..>>` in
      `table.rs` — so a directory with 100k entries would build 500k widgets
      per frame. The list lays out only the rows inside the viewport. This is
      the single largest piece of unknown work in M1 and should be built first.

- [x] **One TOML file, kebab-case keys, hot reload, and a trust check.** All of
      ricebar's config discipline carries over unchanged, including
      `#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]`, a
      hand-written `impl Default` per struct rather than per-field serde
      defaults, a broken config being reported and ignored rather than fatal,
      and refusing to run commands from a file that other users can write.

- [x] **No D-Bus.** Mounts come from `/proc/self/mountinfo`, trash from the
      freedesktop trash *directory* spec, which is plain file operations. No
      udisks, no automount, no tray, no StatusNotifierItem. It keeps the
      dependency list and the attack surface small, and it is the same line
      ricebar draws.

- [x] **Dependencies for M1.** `iced 0.14` with `tokio`, `image`, `svg` and
      `advanced`; `serde`; `serde_json`; `toml`; `jiff` for the modified
      column; `tokio` with `io-util`, `process`, `sync`, `time`. Then, when the
      milestone needs them: `notify` for directory watching (M1), `rustix` for
      the `*at` syscalls the job engine needs (M2), `freedesktop-icons` for
      icon themes (M6).

      Deliberately not taken: `clap` (ricebar's hand-rolled `Arguments` enum is
      40 lines and prints better errors), `anyhow`/`thiserror`
      (`Result<T, String>` for anything a user reads, `io::Result` for I/O),
      `log`/`tracing` (`eprintln!("ricedir: …")` plus an in-app notice line),
      `dirs` (eight lines of `std::env`), `tree_magic`/`libmagic` (a hundred
      lines of our own sniffer, and the system's `globs2` parsed directly).

      Left open on purpose: `rmcp`, the official Rust MCP SDK. M0 decides
      whether it is worth the dependency or whether JSON-RPC over stdio is
      small enough to write, given that the socket — not MCP — is the surface
      everything else is built on.

## M0 — the survey

Part of the reason for building this is to find out what is actually available
on Linux for the agent half, because none of it is installed here yet and none
of it is in daily use. That is a finding rather than a blocker, but it does mean
the choices below are made from reading rather than from experience, and each
one has to be tried before the plan leans on it.

Runs alongside M1, not before it: M1 needs none of it.

- [x] **What is on this machine today**, checked 2026-09-03. Present:
      PipeWire with `pw-record` and `parecord`, `arecord`, `ffmpeg`,
      `python3`, `uv`/`uvx`, `node`/`npx`, `socat`, `nc`, `jq`, `wl-copy`,
      `bwrap`, `flatpak` with fifteen apps, and the `claude` CLI. Absent:
      every speech-to-text engine, every local model runner, every MCP server,
      and every file scanner. Kernel 6.18 with Landlock enabled. No systemd,
      so nothing may assume `systemd-run` or a user unit.
- [ ] **Survey speech-to-text on Linux** and write down what each needs, how
      big the model is, whether it streams, and whether it runs on CPU alone.
      To look at: `whisper.cpp` (ships `whisper-cli` and `whisper-server`),
      `faster-whisper` (`uvx`-able), `sherpa-onnx` (streaming, good Linux
      story), `vosk` (small and offline), and Moonshine. Then the prior art
      for *voice control* specifically, which is the interesting part:
      **Numen** and **nerd-dictation**, both of which drive a Linux desktop by
      voice already and will have learnt things we are about to learn.
- [ ] **Look hard at the Wyoming protocol.** Home Assistant's voice services
      speak it, there are already Wyoming wrappers for faster-whisper and
      piper, and it is a small socket protocol rather than a framework. If it
      fits, ricedir's voice story becomes "point any Wyoming service at it"
      and we write nothing. Worth an hour before writing a line of our own.
- [ ] **Survey local model runners.** `llama.cpp`'s `llama-server` exposes an
      OpenAI-compatible HTTP API, which means no linking and no Python;
      `ollama` is easier to install and heavier; `mistral.rs` and `candle` are
      Rust. The decision to make is only "what does the `http` transport talk
      to first", so the bar is low.
- [ ] **Survey the open-source agents worth being a tool of**, since the
      decision above is to be driven rather than to drive. **Goose** first:
      open source, local-first, MCP servers are its extension mechanism, and it
      has both a CLI and a desktop app. Then **Open WebUI**, which already has
      voice in and out and reaches MCP through `mcpo`, so it is a voice front
      end we would not have to write. Then `mcphost`, LibreChat, Continue and
      Zed for how they present a tool list. The question to answer for each:
      how much of the bridge disappears if we adopt it.
- [ ] **Survey MCP on Linux as it actually stands.** The spec version to
      target, JSON-RPC over stdio versus streamable HTTP, which clients exist
      beyond `claude` (Claude Desktop, Zed, Continue, `mcphost`), and whether
      the official Rust SDK `rmcp` is worth the dependency or whether a couple
      of hundred lines of JSON-RPC is the better trade given the rest of this
      project's dependency policy. Record the spec version in the write-up,
      because it moves.
- [ ] **Survey scanners for the scan chain.** `clamd` with `clamdscan` for the
      classic case, and **YARA-X** — the Rust rewrite of YARA — for rules that
      a person can read and edit. Run as a subprocess either way, because an
      in-process parser for hostile input is the exact thing this program
      exists to avoid.
- [ ] **Find out what other file managers do about agents**, expecting the
      answer to be nothing. If that holds, say so in the README, and if it does
      not, learn from whoever got there first.
- [ ] **Research `xdg-desktop-portal` as a way in.** If ricedir could serve
      the `FileChooser` portal, every flatpak application's open dialogue
      becomes ricedir, which would be the single largest reason for anyone else
      to install it. The tension to resolve honestly: the portal is D-Bus, and
      `## Decided before any code` says no D-Bus. Worth costing before
      deciding, since a separate small bridge process is a way to have both.
- [ ] **Write it up in `docs/agents-on-linux.md`** with versions, dates, what
      was installed to test it, and what failed. Then install one
      speech-to-text engine and one local runner and record what actually
      worked, so the plan stops being based on reading.

## M1 — browse and open

Read-only on purpose. Nothing in this milestone can destroy a file, so the
foundations can be got wrong safely. A copy in M1 would be the riskiest code
landing before the list widget is even proven.

- [ ] **Cargo skeleton and the checks.** `Cargo.toml` as decided above,
      `src/main.rs` with ricebar's `Arguments::{Run, Handled}` shape, and
      `cargo build`/`test`/`clippy --all-targets -- -D warnings`/`fmt --check`
      all green from the first commit.
- [ ] **`src/config/`.** Port from ricebar and change what a file manager
      needs: `default_path()` for `$XDG_CONFIG_HOME/ricedir/config.toml`,
      never-fatal `load()`, `trustworthy()` on the file *and its parent*,
      `first_run::create()` writing a config plus the probed handler list, and
      `color.rs` verbatim (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`, 9 tests).
- [ ] **The action registry.** One `Action` enum with typed arguments, one
      `dispatch(action) -> Task<Message>`, and a name plus a one-line
      description per variant. The menus, the toolbar and the bindings table
      are all built from it, so a new action appears in all three at once and
      M3 gets its tool list for free. Nothing here talks to an agent yet.
- [ ] **The window.** `iced::daemon` rather than `application`, because a
      second ricedir window should be a second window and not a second process.
      Theme, font and font size from the config; `decorations` left on until M6.
- [ ] **The entry model.** `read_dir` on a worker thread, streamed to the Elm
      loop in chunks so a slow network mount paints progressively.
      `symlink_metadata`, so a broken symlink is shown as a symlink rather than
      vanishing. Natural sort written here rather than pulled in — digits
      compared as numbers, `file2` before `file10` — with directories first and
      a case-insensitive option.
- [ ] **The list widget.** Only viewport rows laid out and drawn. Uniform row
      height, so the visible range is arithmetic rather than a search. Needs a
      `Widget` impl with `advanced`, and needs `scrollable`'s
      `scroll_to(AbsoluteOffset)` to keep the keyboard cursor on screen —
      `AbsoluteOffset` and `RelativeOffset` are re-exported from
      `iced_widget::scrollable`, which is a thing ricebar found *not* to be
      true of `iced_runtime::scroll_to`, so check before assuming.
- [ ] **Selection.** Cursor plus an anchor: click sets both, `Ctrl+click`
      toggles one, `Shift+click` and `Shift+arrow` extend from the anchor,
      `Ctrl+A`, invert, and a rubber band from a drag on empty space. Stored as
      a `HashSet<usize>` into the buffer's entry vector, cleared on relist
      unless the entry survived it by name.
- [ ] **Layouts.** Detail rows, compact grid, large icons. One trait, chosen
      per buffer, defaulting from the config.
- [ ] **Tiles and buffers.** `pane_grid` for the tiles, a flat `Vec<Buffer>`
      addressed by index, split/close/focus, and a buffer list to point a tile
      somewhere else. Closing the last tile of a buffer keeps the buffer.
- [ ] **Places.** Home and the XDG user directories from
      `~/.config/user-dirs.dirs`, mounted filesystems from
      `/proc/self/mountinfo` (filtered: no `sysfs`, `proc`, `cgroup`, `tmpfs`
      under `/run`), and bookmarks in ricedir's own file so nothing is written
      into GTK's. Drag a directory onto the panel to bookmark it.
- [ ] **Path bar.** Breadcrumbs that are buttons, an editable path with
      completion, and back/forward/up with a per-buffer history.
- [ ] **Filter and sort.** A filter box that narrows as you type (substring by
      default, glob with a leading `:`), a hidden-files toggle, and a sort menu
      over name, size, modified, type and extension.
- [ ] **Context menu and toolbar.** Right click on an entry, on the selection,
      and on empty space, each with its own items. iced hands `update` no
      geometry, so the cursor position has to come from
      `iced::event::listen_with` tracking `Mouse(CursorMoved)`; the menu is
      then a `Stack` over the list rather than a real popup.
- [ ] **Watching.** `notify` on the visible buffers only, debounced ~100 ms and
      coalesced into "relist this directory". Beware the rename dance ricebar
      documents: editors and `mv` replace a file rather than writing it, and a
      watch on the old inode misses that entirely.
- [ ] **MIME resolution.** Parse `/usr/share/mime/globs2` (`weight:type:glob`,
      one per line, already sorted by weight) plus `aliases`, and fall back to
      a small hand-written magic sniffer for the couple of dozen signatures
      worth knowing. No libmagic: it is a C parser for hostile input, which is
      the one thing this program exists to avoid.
- [ ] **The scan chain.** `[[scan]]` entries, evaluated in order, first
      non-allow verdict wins. Kinds: `glob`, `regex`, `magic-mismatch`,
      `size`, `command`. `warn` shows a dialogue naming the rule that fired and
      offers open-anyway, reveal-in-terminal, or cancel; `block` refuses and
      says which rule refused.
- [ ] **The opener.** The handler table, flatpak preference, argv spawn with
      `kill_on_drop(true)`, stderr captured into the notice line, and a
      hard rule that a file is never executed because it is executable.
- [ ] **The no-handler dialogue.** What happens when nothing matches: name the
      type, offer choose-a-program, `xdg-open` once, `xdg-open` always, or
      cancel, and write the config on "always". Refuse the fallback for the
      run-rather-than-open types listed in `## The opener`. This is the first
      dialogue in the program, so it also settles what a dialogue looks like.
- [ ] **The notice line.** ricebar's `Bar::warn` idea: a strip that shows
      config errors, refused handlers and failed spawns, because a file manager
      started from a launcher has no terminal to print to.
- [ ] **The status line.** Entry count, selection count, selected size, free
      space on the buffer's filesystem, and the active filter.
- [ ] **The jobs panel, empty.** The panel and its layout, with nothing to put
      in it until M2. Cheap now, and it stops the sidebar being redesigned
      later.

## M2 — the job engine

- [ ] **The queue.** A job is a plan built before any byte moves: the full
      entry list, the total size, the conflict policy and the destination. One
      queue, a configurable number of workers, and per-job pause, resume and
      cancel. Cancelling leaves a partial file behind and says so, rather than
      pretending it can undo a half-written copy.
- [ ] **Progress that does not flood the loop.** Workers send bytes-done at
      most 30 times a second per job, coalesced in the channel drain. A
      progress message per file would make a copy of 200k small files slower
      than the copy itself.
- [ ] **Conflicts.** Skip, overwrite, keep both, newer only, and larger only,
      asked once with an apply-to-all, decided up front where the plan already
      knows there is a clash.
- [ ] **Not walking off the edge.** `*at` syscalls through `rustix` so a
      recursive delete cannot be redirected by a symlink swapped in mid-walk:
      `openat` with `O_NOFOLLOW|O_DIRECTORY` down the tree, `unlinkat` with
      `AT_REMOVEDIR`, and a device-number check so a recursion never crosses a
      filesystem boundary it was not asked to cross. Depth capped, and a
      directory loop reported rather than followed.
- [ ] **Fast paths.** `copy_file_range` for a copy on the same filesystem,
      reflink where the filesystem does it, `renameat` when a move stays on one
      device, and a plain read/write loop as the fallback. Free space checked
      against the plan's total before starting.
- [ ] **Trash, by the spec.** `~/.local/share/Trash/{files,info}`, a
      `.trashinfo` per entry with the original path and an ISO timestamp, a
      `$topdir/.Trash-$uid` for other filesystems, and a trash browser that can
      restore. No `gio trash`, no D-Bus.
- [ ] **Undo.** A stack of what actually happened, not of what was intended.
      Move, rename and trash undo cleanly; a copy's undo is "delete what was
      copied", offered but named plainly; a real delete has no undo and the
      dialogue says so.
- [ ] **Create, rename, link, permissions.** New directory, new empty file,
      rename in place, symlink and hard link, and a mode editor that shows both
      the `rwx` grid and the octal.
- [ ] **The jobs panel, filled.** One row per job: what, where to, a progress
      bar, rate, time left, and cancel. Errors collect into a list on the job
      rather than stopping the world with a modal, and a finished job stays
      until dismissed if it had any.

## M3 — the control surface

Read-only, and the foundation for everything in M4. The point is that an agent
drives ricedir through the *same* actions a person does, and that it can see
what the person is looking at.

- [ ] **The socket.** A line-delimited JSON socket at
      `$XDG_RUNTIME_DIR/ricedir/<display>.sock`, mode `0600`, with the peer's
      uid checked through `SO_PEERCRED` and a mismatch refused. Requests
      invoke actions from M1's registry; that is the whole protocol. Everything
      else in M3 and M4 is a shape on top of this.
- [ ] **`ricedir --mcp`.** An MCP server over stdio that forwards to the
      socket, so `claude mcp add ricedir -- ricedir --mcp` works and any MCP
      client gets the tool list. It is a translator, not a second
      implementation: MCP is one wire format for the socket and not the only
      one, which is what makes a non-MCP voice script a two-line shell
      pipeline instead of a project.
- [ ] **The reading tools.** `list_dir`, `stat`, `mime`, `search`,
      `read_text_head`, `scan_verdict`, `disk_usage`. Every path checked
      against the roots the config allows, every result capped in size and
      count, and the cap reported rather than the result silently truncated.
- [ ] **The session tools — the ones that matter.** `selection`, `buffers`,
      `tiles`, `cursor`, `visible`, `filter`, `places`, `jobs`. "Copy these to
      the backup drive" is only answerable because of these, and no filesystem
      MCP server has them.
- [ ] **Four kinds of tool, and only one of them needs a plan.** Tracing a
      real sentence through the design showed that treating "mutating" as one
      category is wrong, because making an agent get a plan approved before it
      may scroll is unusable.

      **Reading** (`list_dir`, `stat`, …) and **session** (`selection`,
      `cursor`, …) answer questions and change nothing. **View** tools —
      `go`, `set_selection`, `filter`, `sort`, `split`, `layout` — change what
      the window shows and touch no file, so they act immediately; they are
      visible on screen, which is its own review, and undone by looking away.
      Only **file** tools — `copy`, `move`, `trash`, `rename`, `mkdir`,
      `link`, `transform` — build a plan. The categories are a property of the
      action registry, so a new action declares its kind once and every
      transport enforces it.
- [ ] **An event stream.** Subscribe and receive what happens: a directory
      changed, the selection changed, a buffer opened, a job finished, a scan
      blocked something. Agents that react — sort a download when it lands,
      warn when something arrives that a rule dislikes — need this rather than
      a polling loop.
- [ ] **The audit log.** Every request appended to
      `$XDG_STATE_HOME/ricedir/agents.log`: timestamp, which agent, which
      action, the arguments, the verdict. Not optional, not configurable away,
      and readable from the jobs panel.
- [ ] **The trace strip.** The last few agent calls as they arrive, named and
      timestamped, above the status line. Testing M3 by hand is watching this,
      so it is worth building early rather than last: with `claude mcp add
      ricedir -- ricedir --mcp` and typed prompts, the strip is the whole
      debugging story before there is any voice involved.
- [ ] **Pacing.** A minimum gap between view actions from one agent, so a
      sequence is followable rather than a flicker. Config key, defaulting to
      something around 120 ms, and not applied to reading tools.
- [ ] **The brake.** One toggle in the window that detaches every agent
      immediately, plus pause and detach per agent, plus a visible mark
      whenever any agent is attached. Findable without documentation.

## M4 — agents that act

- [ ] **Staged plans.** A mutating request returns a plan and moves nothing:
      sources, destination, entry count, total bytes, the conflicts already
      detected, and what it will not do. `plan_show` puts it in the window,
      `plan_accept`/`plan_reject` decide, and an agent can read its own plan
      back and revise. Policy `review-destructive | review-all` from the
      config, a person always the one accepting, and a plan expires if nobody
      accepts it.
- [ ] **The writing tools, off by default.** `copy`, `move`, `trash`,
      `rename`, `mkdir`, `link`, `set_selection`, `go`, `open_path`, `split`.
      Enabled per tool in the config. Each one builds a plan, then a job in
      M2's engine, so it appears in the panel, is cancellable and undoes where
      the engine can. `delete` is not offered at all: an agent can trash, and a
      person empties the trash.
- [ ] **The intent bar.** One input at the top of the window: type what you
      want, it goes to the agent the config names for the `intent` role, the
      agent calls back through the reading and session tools, and what comes
      back is a plan to accept. The same endpoint takes an intent posted to the
      socket, which is how everything below plugs in.
- [ ] **The agent registry.** `[[agent]]` in the config: a `name`, a
      `transport` of `stdio` (an argv vector), `unix` (a socket path) or
      `http` (a URL), a `role` of `intent`, `scan`, `describe` or `rename`, and
      a capability grant listing exactly which tools that agent may call. An
      agent gets no capability it was not granted, and the grant is per agent
      rather than global.
- [ ] **Voice, as an example rather than a feature.** `dev/agents/voice.py`:
      `pw-record` into a chunk, a transcriber the user installed, and the text
      posted to the socket as an intent. PipeWire, `pw-record`, `ffmpeg`,
      `python3`, `uv` and `socat` are all present on this machine and no
      speech-to-text is, so the script must probe for one and say plainly what
      to install rather than assuming. Shipped the way ricebar ships
      `dev/scripts/`: embedded, written on first run, and never overwritten.
- [ ] **ricedir as an MCP client.** The other direction, once the registry
      exists: a `scan` role backend that gets asked about a file before it
      opens, a `describe` role for a "what is this?" action on the selection,
      and a `rename` role that proposes names for a bulk rename. Each is a
      plan or a verdict, never a direct write.
- [ ] **Attribution everywhere.** A job row, an undo entry and an audit line
      each name who asked: the person, or the agent by its config name. A jobs
      panel that cannot tell you which agent moved your files is not a jobs
      panel.

## M5 — finding and changing things in bulk

- [ ] **Search.** Name search on our own walker (so it respects the same root
      limits and cannot be talked into `/proc`), content search through `rg`
      when it is installed, results presented as a buffer like any other, so
      every layout, selection and operation works on them unchanged.
- [ ] **Bulk rename as an editable buffer.** The whole list of names in a text
      editor pane, edited freely, with a diff preview and a refusal to apply if
      the line count changed. This is `wdired`, and it is worth more than any
      pattern dialogue.
- [ ] **Archives.** Create and extract through allowlisted tools only, into a
      fresh directory, with the caps that make extraction safe: no `..` or
      absolute member paths, no symlink members pointing outside the root, a
      total-size cap, a compression-ratio cap and an entry-count cap. Browsing
      inside an archive is explicitly out of scope for now.
- [ ] **Properties, and directory sizes.** A panel with size, times, mode,
      owner, MIME type, link target, and the scan verdict. Recursive size
      computed as a job, since it is a full walk.
- [ ] **Compare two directories.** Same-name-different-content, only-in-left,
      only-in-right, from two tiles.
- [ ] **Transforms: allowlisted tools run over a selection.** Thunar's custom
      actions, kept safe the way handlers are. `[[transform]]` names an argv
      vector, what it accepts, and what it produces — resize an image, convert
      a video, strip metadata, make a thumbnail *file*. It runs as a job with
      progress and errors, and it builds a plan first because it writes.

      Found by tracing "create thumbnails and move them to backup": the verb
      an agent reaches for is not the thumbnail cache in M7, it is "make me
      some files". Without this the sentence has a hole in the middle, and the
      agent would have to shell out around ricedir to fill it, which is the one
      thing the whole design is trying to prevent.

## M6 — eye candy

Deliberately after correctness, but not optional: the reason to use this rather
than the one already installed is partly that it looks good.

- [ ] **Animation, using `iced::animation::Animation`.** It is in 0.14 as a
      re-export of `lilt`: `easing()`, `quick()`/`slow()`, `delay()`, `go(state,
      at)`, `is_animating(at)`, `interpolate(a, b, at)`. Worth animating: a tile
      appearing and closing, the selection block sliding to the cursor, a row
      lifting under the pointer, a job's bar and its arrival in the panel, the
      filter box unrolling, a breadcrumb changing, and a directory's rows fading
      in as the listing streams.
- [ ] **Subscribe to frames only while something is animating.**
      `iced::window::frames()` gives an `Instant` per redraw; a program that
      holds it open forever is a program that keeps a laptop's GPU awake for no
      reason. Gate it on any live `Animation`.
- [ ] **A reduced-motion switch**, and honour it properly rather than making
      the durations small.
- [ ] **Client-side decorations.** `decorations = false`, `transparent = true`,
      a titlebar we draw with the path in it, rounded corners and a shadow.
      Check on all three compositors: sway and niri prefer server-side, so the
      switch has to be a config key rather than an assumption.
- [ ] **Theme presets shipped.** Catppuccin, Gruvbox, Nord and Rosé Pine as
      commented blocks in the example config, the way ricebar proves theming by
      giving its two recorded bars different palettes.
- [ ] **Icon themes.** `icon-theme = "…"` resolved through
      `freedesktop-icons` rather than reimplementing the spec, with a Nerd Font
      glyph table as the fallback and `*-symbolic.svg` recoloured to follow the
      palette — a convention ricebar already settled.

## M7 — thumbnails, out of process

Last, and the reasoning is the whole feature: we are not competing with an
image viewer, and decoding a file from a dodgy site inside the file manager's
own address space undoes everything M1 was for.

- [ ] **A helper process per thumbnail**, `bwrap`-sandboxed with no network and
      a read-only bind of the one file, its output written to a pipe.
      `bwrap` is at `/usr/bin/bwrap` here, and `CONFIG_SECURITY_LANDLOCK=y`
      with landlock first in `CONFIG_LSM`, so the helper can drop filesystem
      access itself as a second layer.
- [ ] **Cache by the freedesktop thumbnail spec**, keyed on the URI's MD5 with
      the size and mtime stored in the PNG, so an existing cache is reused and
      ours is readable by anything else.
- [ ] **A hard cap on concurrent helpers**, and a per-file timeout.

## Tools we may end up building

Named here so the scope is visible. ricebar already has the precedent: its
recording rig needed a virtual pointer nobody had written, so `dev/record/`
holds a separate small crate that does one thing. The same rule applies —
build one only when the survey shows nothing does the job.

- [ ] **A voice agent.** Almost certainly needed as an example at least:
      record a chunk, transcribe it, post the text to ricedir's socket. Starts
      as `dev/agents/voice.py` in M4. Becomes its own crate only if it grows
      wake-word handling, push-to-talk and streaming, which it will if it turns
      out to be pleasant to use.
- [ ] **A Wyoming bridge**, if the survey says Wyoming is the ecosystem to
      join: a process speaking Wyoming on one side and ricedir's socket on the
      other, so any Wyoming speech service works with no ricedir changes.
- [ ] **A portal bridge**, if serving the `FileChooser` portal is worth it: a
      small D-Bus process that talks to ricedir over its socket, keeping D-Bus
      out of ricedir itself.
- [ ] **A scan shim**, only if `clamdscan` and `yr` turn out to have exit codes
      and output too awkward to map from config alone.
- [ ] **The thumbnailer helper** from M7, which has to be a separate binary
      regardless, because the whole point is that it is a different process in
      a sandbox.

## What other file managers do, and what of it we want

Surveyed for the feature set rather than for parity. The pattern is that GUI
managers are discoverable and slow to drive, TUI managers are the reverse, and
the interesting space is a GUI that a machine can also drive.

| | what it gets right | what we take |
| --- | --- | --- |
| **Nautilus** (GNOME Files) | Nobody is confused by it. Breadcrumbs, a places sidebar, one obvious way to do each thing. | The furniture, and the discipline of a default that needs no explanation. |
| **Dolphin** (KDE) | Split view, an information panel, an embedded terminal, and genuinely deep configuration. | Splitting, the information panel, and configurable detail columns. Not the settings dialogue with forty pages. |
| **Thunar** (XFCE) | Custom actions: user-defined commands with `%f`-style placeholders, per file type. | The idea, made safe — our handlers and actions are argv vectors, never shell strings. |
| **Nemo**, **PCManFM-Qt** | Dual pane and scripts, cheaply. | Nothing specific; they confirm splits and user actions are table stakes. |
| **COSMIC Files** | Rust on libcosmic, which is iced. Proof this stack can carry a file manager, and the closest neighbour by construction. | Confidence, and a reason to keep an eye on how it solved its list widget. |
| **Yazi** (TUI) | Everything async, so nothing blocks the UI; plugins in Lua; genuinely fast. | The async-everything posture, and the plugin surface as a first-class idea rather than a bolt-on. |
| **ranger**, **lf**, **joshuto**, **nnn**, **vifm** | Keyboard-driven, tiny, scriptable. | Their operation set, not their key language. |
| **Emacs Dired** | `wdired`: rename by editing the buffer. The best bulk-rename interface anyone has built. | Bulk rename as an editable buffer, in M5. |
| **Total Commander**, **Krusader**, **Double Commander** | The dual pane with a queue of background jobs, and archive handling. | The job queue with real progress, pause and cancel. Not the F-key legend, and explicitly not being a Midnight Commander clone. |
| **Directory Opus** | Configurable columns, saved layouts, a batch rename with a preview. | Column choice, layout switching, and a rename that previews before it acts. |
| **Files** (Windows), **Marta**, **Nimble Commander** | That a file manager can look good, and that this matters. | The animation and decoration budget in M6. |
| **Spacedrive** | A library and tags across devices, in Rust. | Nothing. It is a different program with a different premise, and its scope is a warning. |
| **fman** | A command palette as the primary interface. | Considered and set aside — the MCP surface is our version of "type what you want". |

What none of them do, and what ricedir is for: never open a file with something
the user was not asked about, and expose the whole manager to an assistant
through the same rails a person uses.

## Why this is not just an MCP filesystem server

Worth writing down, because it is the question anyone will ask, and because the
answer is what the design has to protect.

A filesystem MCP server gives a model `read_file`, `write_file` and
`list_directory` over some root. It is useful and it is not this. It has no
idea what you are looking at, so "these" and "here" mean nothing to it; it
writes when asked, so there is nothing to review before it does; it reports
nothing while it works, so a copy of 40 GB is a call that either returns or
does not; it has no undo; and it cannot tell you afterwards which of your
agents moved what.

ricedir has a window in front of all of it. That gives four things a plain
server cannot have:

- **Deixis.** `selection`, `cursor`, `visible` and `buffers` make "copy these
  two to the backup drive" a resolvable sentence. This is most of why voice is
  worth wanting: speech is full of pointing words, and pointing words need
  somewhere to point.
- **Review.** A mutating request produces a plan on screen — sources,
  destination, counts, bytes, conflicts — and nothing moves until it is
  accepted. The agent gets the plan too, so it can fix a clash before you are
  asked.
- **Progress and a brake.** Agent work is the same job in the same panel as
  yours, with the same bar, the same cancel and the same undo, attributed to
  whoever asked.
- **One set of rails.** Because every request goes through M1's action
  registry, a tool cannot reach a code path that skips the confirmation, the
  scan chain, the job engine or the audit log. There is no second door.

The socket is the real interface and MCP is a translation of it. That ordering
matters: it means a voice script piping JSON through `socat` is a first-class
client rather than a hack around the outside of something that only speaks MCP.

## A sentence, traced

*"Select all image files in my Pictures directory, create thumbnails and move
them to backup."* Written out because tracing it is what found the tool
categories and the missing `transform` action, and because it is the test any
change to the agent surface has to still pass.

Nothing here is ricedir being clever. An agent elsewhere hears the sentence and
makes the calls; ricedir's job is that each call has an obvious answer and that
the dangerous ones stop to ask.

1. `places` — resolves "my Pictures" against the XDG user directories and
   "backup" against the bookmarks and mounts. Reading. Ambiguity comes back as
   a list rather than a guess, and the agent asks which one.
2. `go` to that directory. A view tool, so it happens at once, and the person
   watching sees where it went.
3. `list_dir` with a MIME filter of `image/*`. Reading, capped, and the cap is
   reported if it bites.
4. `set_selection` over what came back. A view tool, so it happens at once —
   **and this is the quiet win**: 240 files light up on screen before anything
   irreversible is proposed. The person sees that the agent understood "image
   files" the same way they did, and they see it for free, with no dialogue.
5. `transform` with the `thumbnail` entry from the config. A file tool, so it
   builds a plan: 240 inputs, the argv it will run, where the output goes, the
   estimated size. Shown, accepted.
6. The job runs in M2's engine — progress, rate, cancel, errors collected —
   with the agent's name on the row.
7. `move` the outputs to backup. A file tool, so another plan: count, bytes,
   conflicts already checked, free space verified. Accepted.
8. Another job. Undo afterwards, because a move undoes cleanly.
9. Every call is in the audit log, with which agent made it.

Two places it can stop. The scan chain, if an input trips a rule. The plan
review, if what is on screen is not what was meant — and by then the person has
already seen the selection, which is the step most likely to have gone wrong.

## The opener

The reasoning, kept here because it is the design and it will be argued with.

The common way to be harmed by a file manager is not exotic. A file arrives
from a site that should not be trusted, it is double-clicked, and the manager
consults a chain — extension, then a MIME database, then a `.desktop` file's
`Exec=` line, then a shell to expand it — and hands the file to whichever
program that chain named, with whatever arguments the file's own name produced.
Every link in that chain is data the attacker had a hand in.

ricedir replaces the chain with a list:

- **A handler is an argv vector**, never a command string. `run = ["mpv",
  "--", "{path}"]`, and `{path}` is substituted as one element. There is no
  shell, so there is no quoting problem to get wrong, and a file called
  `; rm -rf ~ #` is a file with a silly name.
- **`--` goes before the path**, so a file called `--config=/…` is a path and
  not an option.
- **`xdg-open` is the fallback, not the mechanism.** It resolves handlers
  through exactly the chain above, so it is never what happens *first* and
  never what happens *silently*. When no handler matches, the window says so
  and offers it as one of four choices, and taking it permanently writes a
  handler into the config. `fallback = "none"` turns even that off.

  What this gives up, stated plainly: on the `ask` default, a person who
  answers "always" without reading has re-enabled the chain for that one type.
  What it buys is that the program is usable on its first run by someone who
  has not written a config yet, and that the moment they choose is a moment
  they were told what they were choosing. Silence would not have made them
  safer, only stuck.
- **`.desktop` files are data.** They are shown, never executed, and their
  `Exec=` line is never consulted.
- **The executable bit is not a route to launching anything.** Opening a file
  runs the handler for its type. Running a program is a separate, explicit
  action.
- **The handler's own program is resolved once** against `$PATH` at load time
  and the absolute path is kept, so a directory prepended to `$PATH` later
  cannot substitute a different binary; a program in a world-writable
  directory is refused.
- **A flatpak handler is preferred** when the app is installed, because it
  brings a sandbox we did not have to write.
- **The config is the trust boundary**, so it gets ricebar's check: if the file
  or its parent directory is writable by group or other, no command in it runs,
  and the refusal is visible in the window rather than only on stderr.
- **The scan chain runs first**, and it is where a machine-specific opinion —
  a regex, a scanner, later a model — gets to say allow, warn or block.

None of this is a privilege boundary against the *user*: someone who can edit
the config can already run anything as themselves. It is a boundary against the
*file*, which is the thing that actually arrives from elsewhere.

## Rejected

- [x] **Not tabs.** Tiles over buffers does everything a tab strip does and
      composes with splitting. Deciding this now avoids building both.
- [x] **Not a keybinding language.** Bindings are a flat table of action to
      key, nothing more. No modes, no chords, no leader key, no which-key. The
      power surface is MCP.
- [x] **Not a Midnight Commander clone.** No F-key legend, no dual pane as the
      only shape, no 1990s furniture. Said plainly because "keyboard-driven file
      manager" makes people assume it.
- [x] **Not D-Bus, and so no tray, no udisks, no automount.** Same line
      ricebar draws. Mounting a disk is a thing the desktop already does.
- [x] **Not browsing inside archives as though they were directories.** It
      means a virtual filesystem layer, and every operation in the program has
      to learn about it. Extract, work, re-archive.
- [x] **Not a MIME chain, not `libmagic`.** `globs2` is a text file with three
      fields; a magic sniffer for the signatures that matter is a hundred lines
      of ours instead of a C parser for hostile input.
- [x] **Not thumbnails in process, ever.** Media decoders are the classic
      memory-safety hazard, and a file manager is where untrusted media lands.
      Out of process and sandboxed, or not at all.
- [x] **Not `gio`, `trash-put` or `rsync` as the engine.** Shelling out for the
      core operations means no real progress, no cancel, and no undo. They are
      fine as optional handlers; they are not the implementation.
- [x] **Not a tree pane in M1.** A collapsible tree in the sidebar is wanted,
      but it needs the list widget to be settled first.
- [x] **Not a chat window bolted onto a file manager.** The intent bar is one
      line that produces a plan, not a conversation panel with scrollback. If
      you want to talk to a model, talk to it in the thing built for talking to
      models and let it drive ricedir through the socket.
- [x] **No audio in ricedir.** No microphone, no ALSA, no PipeWire client, no
      speech-to-text linked in. Speech is text by the time it reaches the
      socket, and whatever did that is somebody else's process.
- [x] **No vendor anywhere in the code.** The config names agents and
      transports. There is no provider enum, no API-key handling in ricedir,
      and no path that assumes a cloud model. An agent that wants a key reads
      its own.
- [x] **No unattended agent writes, and so no `auto` mode.** An earlier draft
      had `auto` accept a plan on the person's behalf. Cut: whether a copy
      needs a human is the agent's judgement to make, not a ricedir setting,
      and an agent that decides it does not need one has a filesystem already.
      Keeping it would also have left the one path where files move with
      nobody having agreed to that particular move — the nearest surviving
      relative of the headless mode rejected above. One enum variant saved, and
      one fewer thing to explain.
- [x] **Not MCP as the only protocol.** The socket comes first and MCP
      translates to it, because the spec is young and moving and because a
      shell script with `socat` should be able to drive this.
- [x] **No headless mode, and no switch to hide agent activity.** An invisible
      ricedir tool call is a worse `cp`. Being watched is the reason the tools
      exist, so file tools refuse when there is no window rather than quietly
      obliging. Saves a config key, a code path and an argument.

## Known constraints (not bugs)

Verified against the crate sources in `~/.cargo/registry` on 2026-09-03, at
iced 0.14.0 / iced_widget 0.14.2 / iced_winit 0.14.0 / winit 0.30.13.

- `iced_widget::table` holds `cells: Vec<Element<..>>` and builds all of them.
  It is for a settings pane, not a directory. The file list must be a widget of
  ours that lays out only what is visible.
- **iced reports no widget geometry to `update`.** ricebar needed a custom
  `widget::Operation` to find out where a module had been drawn. For a context
  menu the cheaper route is `iced::event::listen_with` on
  `Mouse(CursorMoved)`, keeping the last position in state.
- **`mouse_area` has `on_right_press`, `on_double_click` and `on_move(Point ->
  Message)`** but no variant that reports where a press happened, which is why
  the cursor has to be tracked separately.
- **winit has no drag-out on Wayland.** `window::Event::FileDropped(PathBuf)`
  and `FileHovered` exist, so dropping *into* ricedir works; dragging a file
  from ricedir to another application does not, and cannot until winit grows
  it. Say so in the README rather than letting people find out.
- **`iced::window::frames()` is the animation clock** (`Subscription<Instant>`
  from `RedrawRequested`). It must be gated on something actually animating.
- **`Font::with_name` takes `&'static str`** while the family comes from the
  config, so the string is leaked once and memoised — ricebar's
  `config::typeface` does exactly this.
- **The fallback font family is fixed when iced starts**, so that one key
  cannot hot-reload.
- **`notify` and rename semantics.** Editors and `mv` replace a file rather
  than writing it, so a watch on an inode misses the change. Watch the
  directory, not the file, and treat a vanished entry as possibly mid-rename.
- **inotify is a limited resource**: 110313 watches and 128 instances on this
  machine. Watching every buffer a long session accumulates will hit it, so
  only visible buffers get a watch and the rest relist on becoming visible.
- **ricebar's layer-shell traps do not apply.** ricedir is an ordinary
  xdg-toplevel through winit, so `exclusive_zone`, `keyboard_interactivity`,
  `to_layer_message`'s appended variants and `iced_layershell`'s grabbing
  popups are all somebody else's problem. Its *iced* findings still apply.
- **Text cannot be measured outside a renderer.** ricebar hit this repeatedly.
  Column widths therefore come from the config and from a glyph-count estimate,
  or from a `Widget` impl that has a renderer to hand — which the list widget
  will.
- **The MCP specification moves.** Anything built directly against it will need
  revisiting, which is exactly why the socket is the stable surface and
  `--mcp` is a translator. Record the spec version being targeted in
  `docs/agents-on-linux.md` and in the code that implements it.
- **Nothing in the agent half has been used in anger yet.** No speech-to-text,
  no local runner and no MCP server is installed on this machine, so every
  choice in M0 is made from reading. Treat those entries as hypotheses until
  the write-up says otherwise.
