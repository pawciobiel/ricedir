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

- [x] **One TOML file, kebab-case keys, and a trust check.** All of
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

- [x] **Cargo skeleton and the checks.** `Cargo.toml` as decided above,
      `src/main.rs` with ricebar's `Arguments::{Run, Handled}` shape, and
      `cargo build`/`test`/`clippy --all-targets -- -D warnings`/`fmt --check`
      all green from the first commit.
- [x] **`src/config/`.** Port from ricebar and change what a file manager
      needs: `default_path()` for `$XDG_CONFIG_HOME/ricedir/config.toml`,
      never-fatal `load()`, `trustworthy()` on the file *and its parent*,
      `first_run::create()` writing a config plus the probed handler list, and
      `color.rs` verbatim (`#rgb`/`#rgba`/`#rrggbb`/`#rrggbbaa`, 9 tests).
- [x] **The action registry, and a crate boundary to go with it.** One
      repository, three crates: `ricedir-protocol` holds the messages,
      `ricedir` is the window, `ricedir-mcp` is the translator.

      `ricedir-mcp` does not depend on `ricedir`. That is the whole reason for
      the split. "File tools refuse when no window is running" was a sentence
      in `CLAUDE.md`, and a sentence is only as strong as whoever reads it
      next; now a call to `ricedir::open::spawn` from the MCP program is
      `error[E0433]`. Checked by writing that call on purpose and reading the
      error.

      The registry itself: the list widget reports what happened to it,
      `app::translate` says which action that is, `action::dispatch` checks
      the kind, `app::carry_out` does the work. `Action::of` is the only place
      a wire request becomes an action.

      **Kinds live in the protocol crate, not in the window.** Both sides read
      the same `Kind`, so the wire and the window cannot disagree about which
      requests need a person. A test asserts a request keeps its kind through
      the conversion.

      Done before the socket rather than with it, as `## Decided before any
      code` asked. At 5,808 lines it took an afternoon.

- [x] **The window.** `iced::daemon` rather than `application`, because a
      second ricedir window should be a second window and not a second process.
      Theme, font and font size from the config; `decorations` left on until M6.
- [x] **The entry model.** `read_dir` on a worker thread, streamed to the Elm
      loop in chunks so a slow network mount paints progressively.
      `symlink_metadata`, so a broken symlink is shown as a symlink rather than
      vanishing. Natural sort written here rather than pulled in — digits
      compared as numbers, `file2` before `file10` — with directories first and
      a case-insensitive option.
- [x] **The list widget.** Only viewport rows laid out and drawn. Uniform row
      height, so the visible range is arithmetic rather than a search. A
      `Widget` impl of ours, needing only `size`, `layout` and `draw` — the
      other nine trait methods are defaulted.

      **It owns its own scroll offset** rather than sitting inside a
      `scrollable`. Both work, and the deciding argument is the keyboard:
      if the `scrollable` owns the offset, keeping the cursor on screen costs
      a round trip — `shell.publish` from the widget, then
      `iced::widget::operation::scroll_to` from `update`, landing a frame
      late. Held ArrowDown through a long directory is exactly where that
      shows. Owning the offset in `tree::State` makes it a clamp in the same
      `update` that moved the cursor. It also leaves room for variable row
      heights later, which a `scrollable` cannot express, since it knows only
      one content rect.

      The cost is handling `mouse::Event::WheelScrolled` and drawing a
      scrollbar ourselves. `scrollable.rs:855-899` is the reference for the
      wheel arithmetic: `Lines { x, y } => -Vector::new(x, y) * 60.0` and
      `Pixels { x, y } => -Vector::new(x, y)`.
- [x] **Selection, most of it.** Cursor plus an anchor: click sets both,
      `Ctrl+click` toggles one, `Shift+click` and `Shift+arrow` extend from the
      anchor, `Ctrl+A`. A `HashSet<usize>` into the buffer's entry vector, kept
      across a relist by name rather than by index.
- [x] **Selection, the rest.** A rubber band from a drag on empty space, and
      `Ctrl+I` to invert. A press on a row still selects; only a press on
      empty space starts a band, so there is room later for a drag from a row
      to mean something else.

      The band travels as **ranges, not indices**: a drag over 100k rows must
      not put 100k numbers on the queue every time the pointer moves. In the
      list layouts it is a row range; in the grid it is a row range and a
      column range, and the widget sends the stride because only it knows the
      real width.
- [x] **Layouts: list, detail and icons.** `Ctrl+1`, `Ctrl+2`, `Ctrl+3` pick
      one and the backtick cycles; the context menu names the next one.
      `[list] layout` sets the one a window opens with.

      Not a trait, in the end. The three differ in what a cell holds and how
      many sit side by side, and a *list is a grid one cell wide* -- so
      `grid(width)` returns how many are across and how tall a line is, and
      the visible range, the hit test, the reveal and the scrollbar are each
      written once instead of three times. A trait would have made three
      copies of that arithmetic and one of them would have been wrong.

      Chosen per window rather than per buffer, which is what the entry
      originally said. Per buffer means the view changes as you move between
      tiles, which is disorienting; nobody has asked for it and it is one
      field away if somebody does.
- [x] **Tiles and buffers.** `pane_grid` for the tiles, a flat `Vec<Buffer>`
      addressed by index. `Ctrl+\\` splits right, `Ctrl+-` splits down,
      `Ctrl+W` closes a tile, Tab and Shift-Tab move between them, and the
      focused tile is edged in the accent. Closing a tile keeps its buffer.
- [x] **A buffer list, to point a tile at a directory that is already open.**
      The other half of the emacs model. Splitting gives the new tile a buffer
      of its own, which is what a file manager usually wants; this is how two
      tiles come to *share* one listing and one watcher.
      - [x] `Action::ShowBuffer { buffer }` — it points the focused tile,
            so no `tile` argument is needed
      - [x] a list to pick from, showing each open directory and how many
            tiles show it. `$HOME` is written `~`, and the panel is 360 wide
            rather than the menus' 230: at that width every second line
            wrapped, because these lines are paths.
      - [x] close a buffer no tile is showing
      - [x] `Ctrl+B` to open the list. Digits to pick are not built; the
            list is a menu and a click does.

      **Closing uses `swap_remove`.** Tiles hold indices into `App.buffers`,
      so a plain `remove` would shift every index after the hole and every
      tile would be looking at the wrong directory. `swap_remove` moves one
      element -- the last -- so exactly one fixup is needed, and any tile
      pointing at the old last index is told. Proved in the rig by closing
      buffer 0 while the only tile showed buffer 1: the tile followed.

      **The moved buffer is relisted.** Its listing task, if one is still
      running, is addressed to the index it used to have, and its chunks
      would be dropped into a hole. Closing a buffer is a deliberate,
      occasional act, so one extra directory read costs nothing anybody
      feels.

      **A menu opened by a key has nowhere to sit.** `app.pointer` is the
      window's corner until the mouse first moves, which is where the buffer
      list appeared -- half of it off screen. It is placed rather than
      followed now: below the toolbar and clear of the sidebar.

- [x] **A buffer nobody is looking at no longer goes stale.** `ShowBuffer` is
      the only way a hidden buffer becomes visible again, so it is the only
      place that has to relist. Proved in the rig: a file made in a hidden
      buffer's directory was there the moment the buffer came back.
- [x] **Places.** Home and the XDG user directories from
      `~/.config/user-dirs.dirs`, mounted filesystems from
      `/proc/self/mountinfo` (filtered: no `sysfs`, `proc`, `cgroup`, `tmpfs`
      under `/run`), and bookmarks in ricedir's own file so nothing is written
      into GTK's.
- [x] **Add to favourite places, from the menu.** `Action::Bookmark { path }`,
      offered on a directory by name and on the current directory otherwise.
      The panel refreshes and the notice line says which path went in.
- [x] **Add to favourite places, the other ways in.** Removal first: without
      it a mistake is permanent.
      - [x] `Action::Unbookmark { path }`, and rewriting the file without
            that line. Comments and blank lines survive -- the file is one a
            person may have edited, and a removal that tidied it would throw
            their notes away. Written to a temporary file and renamed over the
            real one, so an interrupted removal leaves the old list rather
            than half a list. The rewriting is a pure function, `without`, so
            it can be tested without touching `XDG_CONFIG_HOME`, which is
            process-global and would make two tests fight.
      - [x] a menu on the places panel, which is where removal belongs.
            Only a bookmark offers it: the home directory and a mounted disk
            are not ours to take out of the list.
      - [x] `Ctrl+D` on the focused tile, which is what a browser taught
            everybody. `Action::Bookmark` grew a `buffer` and an optional
            `path`: `None` means the directory being shown. One action rather
            than two, because the only difference is which path -- and
            `translate` has no path to hand, only a buffer index.
      - [x] drag a directory from the list onto the panel. A press on a row
            both selects and may begin a drag, so the two are told apart by
            the pointer moving 6 px. Dragging a row that is *not* selected
            picks up that one; dragging one that is picks up the whole
            selection. Files are dropped from the set before the drag
            starts, because a file cannot be a place and picking one up
            would promise a drop that is refused when it lands.
      - [x] reorder them, through `Move up` and `Move down` in the places
            menu. The file is an order and nothing respected it.
            `places::reordered` is a pure function, so it is tested without
            touching `XDG_CONFIG_HOME`.

            Comments stay where they are and the bookmark lines move around
            them. Deciding which line a comment belongs to is a question the
            file format cannot answer, so it is not asked.
      - [x] reorder them *by dragging*.

            A `button` captures a press, so `mouse_area`'s `on_press` never
            fired and a place could not be a drag source. A place is drawn
            instead, with the hover colour, the hand cursor and the press
            done by hand. Opening moved to the release: until the button
            comes up the press may still become a drag.

            **A drop marker pushed in mid-drag cannot be hit.** Adding the
            line only while dragging moved the row out from under the
            pointer, which took the line away, which moved it back. The gap
            is always there and only its colour changes.

            The strip below the last place is how something reaches the end,
            which "in front of the row you are over" cannot say. Always
            there: a target that appears mid-drag is one nobody finds.

            Every release is heard, not only the ones over the panel: a drag
            let go anywhere else has to be called off, and `mouse_area`
            reports a release only over itself.
- [x] **Path bar, the breadcrumbs half.** Each component is a button, built
      from `Path::components` rather than by splitting the string, so a name
      with a slash-looking character in it cannot fool them. Back, forward and
      up work off a per-buffer history.
- [x] **Path bar, the text half.** The two-faced design is in `## M6`, which
      keeps the animation and the prettier separators; this is the half that
      works.
      - [x] a text face that shows the path and can be typed into. It opens
            with the path already selected, so the common thing -- take a
            piece of this path -- needs no typing and no drag from one end.
      - [x] `Ctrl+L` to reach it, `Escape` to leave, and a `\u{f044}` button
            in the bar that is never ambiguous
      - [x] a click on the bar but not on a crumb. The gap after the last
            crumb is a `mouse_area` filling the rest of the bar, so on a short
            path the target is most of the bar rather than a sliver.
      - [x] completion on Tab, to the prefix every match shares -- what a
            shell does, and what stops Tab guessing between two directories.
            Directories only: the bar goes to a directory, and completing to a
            file would fill in something that cannot be submitted. One match
            gets a trailing slash so the next Tab looks inside it.
      - [x] a path that does not exist says so and keeps what was typed.
            Going nowhere and clearing the box would look like the keystroke
            was lost.

      **The list stops taking keys while the box is up**, which the filter box
      deliberately does not do. `text_input` captures neither the vertical
      arrows nor Tab, so filtering and navigating at once is free -- but a
      path being typed wants Tab for completion, and with the list live it
      switched tiles instead. That leaves nothing to report Escape or Tab, so
      both come from `event::listen_with`; `update` ignores them unless the
      box is actually open.

      **`~` is the only expansion.** No `$VAR`, no globbing, no command
      substitution. A path bar that ran a shell would be a shell prompt with a
      file manager attached. `under` takes home as an argument so it can be
      tested: `HOME` is process-global and the harness runs in threads, the
      same reason `places::user_dirs` is split that way.

      **A stale notice outlived the thing it complained about.** "`/s` is not
      a directory" was still on screen after a later move succeeded, reading
      as though the new directory were the problem. Arriving somewhere clears
      it.
- [x] **Filter and sort.** A filter box that narrows as you type (substring by
      default, glob with a leading `:`), a hidden-files toggle, and a sort menu
      over name, size, modified, type and extension.
- [x] **A bindings table, with defaults a Windows user already knows.**
      Built. The keys used to be written into `widget/list.rs`; they are read
      from the config now. `## Decided before any code` settled the shape: a
      flat table, no modes, no chords of two keys, no leader key.

      ```toml
      [keys]
      "ctrl+c" = "copy"
      "f2" = "rename"
      "alt+left" = "back"
      ```

      The defaults. Windows and GNOME agree on most of these, which is the
      point: somebody's first hour should cost them nothing.

      | key | does |
      | --- | --- |
      | `Enter`, double click | open |
      | `Backspace`, `Alt+Up` | up one level |
      | `Alt+Left` / `Alt+Right` | back / forward |
      | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | copy / cut / paste |
      | `Delete` | trash |
      | `Shift+Delete` | delete, with the dialogue that says it cannot be undone |
      | `F2` | rename |
      | `Ctrl+A` | select all |
      | `Ctrl+Shift+N` | new folder |
      | `F5` | relist |
      | `Ctrl+H` | show hidden files |
      | `Ctrl+F`, `/` | filter |
      | `Ctrl+L` | edit the path |
      | `Ctrl+D` | add to favourite places |
      | `Escape` | put away whatever is in front |

      **`Ctrl+C` is copy, not `Ctrl+Shift+C`.** A terminal needs the shift
      because `Ctrl+C` already means interrupt there; a file manager has no
      such clash, and every other window on the desktop uses the plain one.
      `Ctrl+Shift+C` is worth having, but for **copy the path as text**, which
      is what several file managers already use it for and what people
      actually reach for when they want to paste a path into a terminal.

      Two things this gets from the action registry for free. The right-hand
      side of each line is an action name, so a typo names an action that does
      not exist and is **reported** rather than silently doing nothing. And a
      binding cannot reach anything the registry does not offer, so the keys
      cannot become a second door either.

      The file-operation rows need M2's actions before they can be bound to
      anything. The rest can be read from the config as soon as somebody wants
      to change one.

      - [x] `Bound::name()` and `from_name`, so a line of config finds one
      - [x] parse `"ctrl+shift+n"` into a key and a set of modifiers
      - [x] the default table, in code, so an empty config still works
      - [x] a key the table does not know is reported by name, not ignored.
            The notice line says `keys: \`quti\` is not an action`.
      - [x] `Ctrl+Shift+C` for copy the path. The plain `Ctrl+C` is left
            free, and becomes copy in M2.
      - [x] a way to see the list: `?`, and an item in the toolbar menu

      Done in `src/keys.rs`. Three things worth keeping:

      **A binding cannot reach past the registry.** `Bound` holds only the
      actions that take no argument, because a key has no pointer and no row
      to name. `Open` acts on the cursor, which is the keyboard's own idea of
      "this one".

      **`?` needed both keys iced reports.** A press gives `key` with no
      modifiers applied and `modified_key` with them applied, so `Shift+/`
      arrives as `/` in the first and `?` in the second. The lookup tries the
      plain one, then the modified one with shift dropped. Looking at only
      the first made `?` unbindable, which the rig found at once -- the menu
      simply did not open.

      **Tab and the arrows are not in the table.** A table that can unbind
      them can leave a window whose tiles cannot be reached, or a list nobody
      can walk.

      `"none"` on the right-hand side removes a default, which is the only
      way to give a key back to the window manager.

      Left for M2, when the actions exist: `Ctrl+C`, `Ctrl+X`, `Ctrl+V`,
      `Delete`, `Shift+Delete`, `F2` and `Ctrl+Shift+N`.

- [x] **Compared against eight other file managers.** mc was read from its
      own `/etc/mc/mc.default.keymap`; Nautilus gave up its action names but
      not its accelerators, which live in a compiled GResource. The rest is
      from the documentation and should be checked before anyone acts on it.

      There are two families. Dolphin, Nautilus, Thunar, PCManFM and COSMIC
      Files follow Windows Explorer. Krusader, Total Commander and mc follow
      Norton Commander: `F3` view, `F4` edit, `F5` copy, `F6` move,
      `F7` mkdir, `F8` delete, `F10` quit. ricedir is in the first family,
      which is what "defaults a newcomer already knows" has to mean.

      `Ctrl+L`, `Ctrl+H`, `Ctrl+A`, `Alt+Left`/`Alt+Right`, `Backspace` and
      `Alt+Up` all already matched. Two things came out of it:
      - [x] `Ctrl+T` splits right. Every one of the five desktop managers
            opens a tab with it, and ricedir has no tabs. A second view side
            by side is the nearest thing, and the key now does something
            rather than nothing.
      - [x] `Ctrl+R` relists, beside `F5`. Dolphin uses `F5`; Nautilus,
            Thunar and PCManFM use `Ctrl+R`. Both are bound, because the
            table maps many chords to one action and nobody should have to
            know which family ricedir came from.

      Not taken: `Ctrl+1`/`2`/`3` stay as they are, although every other
      manager puts icons on `Ctrl+1` and ricedir puts the list there.

      Also noted and not acted on: `Ctrl+I` is Dolphin's filter bar and is
      invert-selection here; both orthodox managers use numpad `*` to invert.

- [x] **Icons is the default layout, and a switch is remembered.**
      Icons because that is what every other file manager opens with, and a
      newcomer should not have to find the switch.

      Remembered in `$XDG_STATE_HOME/ricedir/state.toml`, not in the config.
      The config is a file a person writes and comments; this is a file the
      program writes. Writing one into the other loses the comments the
      first time it happens.

      **A config that names a layout wins over the remembered one.** Somebody
      who wrote `layout = "detail"` meant it, and a stray keystroke must not
      quietly overrule the file they edited. serde cannot tell a missing key
      from one set to the default, so `parse` reads the text again as a
      plain value to find out whether the key is there.

      One value, not one per directory. A view remembered per folder is what
      Windows Explorer does, and it is why people say their folders "change
      by themselves".

- [x] **The context menu.** Right click opens it where the click landed, with
      Open, Copy path, Add to places, Show hidden files and Relist. A click
      anywhere else puts it away, and so does Escape.
- [x] **A toolbar**, at the right of each tile's path bar.
      - [x] a layout switch, whose glyph is the layout it will give you
      - [x] a sort menu: the field, and which way round
      - [x] a hidden-files toggle that lights up when they are showing
      - [x] a relist button
      - [x] split right and split down, as two buttons
      - [x] a last button that opens all of it as a menu, in words: a narrow
            tile clips the buttons, and a glyph nobody recognises is not a
            button anybody presses
      - [x] close this tile
      - [x] per tile rather than one bar across the window. A single bar has
            to answer "which tile does this act on", and the honest answer --
            whichever has the keyboard -- is one more thing to know before
            pressing a button.
      - [ ] a jobs button, once there are jobs

- [x] **The menu differs by where the click landed.** One menu was drawn
      whatever was under the pointer. Three now:
      - [x] on an entry: Open, Open in a new tile, Copy path, and "Add
            <name> to places" for a directory. Open with and Properties wait
            for the dialogue work in M6.
      - [x] on empty space: Show hidden, Relist, Copy path, "Add this
            directory to places". Paste and New folder land here in M2.
      - [x] on a place in the sidebar: Open, Open in a new tile, and Remove
            from places for a bookmark.

      **Empty space was not a menu at all before.** A right click past the
      last row fell into the branch that starts a rubber band, which only
      handles the left button, so nothing happened. `list::Action::Menu` now
      carries `row: Option<usize>`, and `None` is a *different* menu rather
      than an absent one: paste, a new folder and the hidden-files switch are
      about the directory, and empty space is where people look for them.
      A test asserts that such a click selects nothing -- picking the nearest
      row instead is how a menu ends up acting on a file nobody pointed at.

      **`button` has no right press**, and `mouse_area`'s does not say where
      it happened, so a place in the sidebar reads the tracked pointer: the
      same position, one event earlier.

      **"Open in a new tile" is one action, not two.** `Split` followed by
      `Go` would need the second to name a buffer the first had only just
      made. `Action::OpenBeside { path }` splits and lists in one go, sharing
      the split with `Action::Split` through a helper.

      All three were checked in the rig with the pointer -- the first thing
      `dev/pointer.sh` was good for.
- [x] **Watching.** `notify` on the visible buffers only, debounced 120 ms and
      coalesced into "relist this directory". Beware the rename dance ricebar
      documents: editors and `mv` replace a file rather than writing it, and a
      watch on the old inode misses that entirely.

      Confirmed live: a file made in a terminal appears with no refresh.
- [x] **Change the list, do not rebuild it.** Done, and the four hazards below
      turned out to be two hazards, one non-problem and one that was already
      a live bug.

      **One rule, and no pairing.** For every path the events named, read it
      again and let the answer decide: it exists, so insert or replace; it
      does not, so take it out. That covers create, remove, a metadata change
      and every shape of rename, and applying the same event twice changes
      nothing.

      Pairing was expected to be the hard part and is not needed at all.
      Measured with a probe against notify 8 on this machine, an in-directory
      rename arrives *three* ways at once -- `Name(From)`, `Name(To)` and a
      synthesised `Name(Both)` carrying both paths, all sharing a tracker id
      -- while a move out gives only `From` and a move in only `To`.
      Re-reading is right for all five without knowing which happened.

      **Overflow was already being ignored.** notify reports it as
      `EventKind::Other` with `Flag::Rescan`, and `changes_a_listing` drops
      `Other` outright -- so the case that "turns a clever optimisation into a
      bug people cannot reproduce" was live in the debounced-relist code
      before any of this. `need_rescan()` is checked first now.

      **The queue filling is its own kind of overflow, and the first fix for
      it was wrong.** Each `touch` emits `Create` *and* `Modify(Metadata)`, so
      900 new files is 1800 events and the channel fills. The first attempt
      answered a failed `try_send` by sending `Rescan` down the same full
      channel, which failed the same way and lost it silently: a directory of
      900 files settled at 218 rows and stayed there. It is an
      `Arc<AtomicBool>` now -- the one piece of genuinely shared mutable
      state in ricedir -- set on a failed send and read when the burst
      settles. Verified: 1 row, one burst, one `Rescan`, 901 rows.

      **A change arriving mid-relist was also being dropped.** `reconcile`
      declines then, because the entries it would touch are about to be
      replaced and the replacement was read before the change happened.
      Declining now marks the buffer `stale` and `Message::Listed(Done)`
      reads the directory once more.

      **What it costs.** Measured on 60,000 entries, ten single-file saves:
      **67 CPU ticks reconciled against 160 relisted**, with an idle baseline
      of 0. The syscalls are gone -- one `stat` instead of 60,000 -- and what
      remains is `rebuild_from`: a near-sorted re-sort plus rebuilding
      `visible` and the selection, all O(n). Going surgical on those indices
      is the next win and was not taken, because the sort is cheap on a
      near-sorted vector and rebuilding by name is what makes the cursor and
      the selection survive without any index arithmetic to get wrong.

      The real win is not the ticks: it is a network mount, where a `stat` is
      milliseconds rather than microseconds and 60,000 of them per saved file
      is the difference between usable and not.

      **A trap the rig taught, again.** `wait-for-window.sh` returns on a
      mapped window, and a buffer is only watched once its *first listing*
      has finished. A burst fired before that happens with no watch in place,
      nothing arrives, and the listing is legitimately wrong for ever. The
      first three runs of the overflow test failed this way and looked like a
      fault in the code.

  *The entry as it was written before the work, kept for the record. It is
  not a task.* Watching works, and what it does
      with a change is blunt: `src/watch.rs` throws the event away and asks for
      a whole new listing. `Relist` in the menu does the same thing by hand.
      Fine for a directory of thirty. For one of 100,000 it is a full
      `read_dir`, 100,000 `symlink_metadata` calls and a full sort because one
      file was touched — a measured 114 CPU ticks each time somebody saves.

      The event already says what happened. Using it means an insert or a
      remove at a position found by binary search, and a re-`stat` of one path.

      **Four things that make this harder than it sounds:**

      - **A rename is two events**, `From` then `To`, and they have to be
        paired to avoid a row vanishing and reappearing at the bottom. notify
        pairs them on some backends and not on others; check which.
      - **inotify can overflow.** When the queue fills, the kernel sends
        `IN_Q_OVERFLOW` and *drops* events. That case must fall back to a full
        relist, or the listing quietly stops matching the disk. This is the one
        that turns a clever optimisation into a bug people cannot reproduce.
      - **A metadata change is not a listing change.** Touching a size or a
        mtime needs one `stat` of one row, not a resort — unless the sort is
        by size or by date, in which case the row moves.
      - **The sort order decides where a new row goes.** Natural sort by name
        is a binary search; sort by mtime means the insert point moves as
        clocks tick.

      Keep the full relist as the fallback for overflow, for an unreadable
      event, and for `Relist` in the menu, which is what somebody reaches for
      when they think the listing is wrong.

      That last paragraph is the one that survived contact unchanged. The
      full relist is the fallback for a rescan, for too large a burst, and
      for F5.
- [x] **A relist no longer blanks the tile.** It used to replace the whole
      `Buffer`, so a directory of 60,000 showed nothing for about a second --
      which reads as the program losing the files rather than as it working.

      `Buffer.arriving` collects the replacement out of sight and
      [`Buffer::finish`] swaps it in when `Update::Done` lands. A buffer with
      nothing showing yet has nothing to keep, so it still streams straight
      into `entries` and paints as the chunks arrive: that is what makes a
      slow mount bearable and it was worth not losing.

      **The swap has to remember by name before it happens.** The cursor and
      the selection are indices into `entries`, so reading them after the
      vector is replaced points at whatever landed in those slots. `rebuild`
      split into `remembered` and `rebuild_from` for this. A relist now keeps
      both, which the old code threw away every time -- a watch event used to
      move your cursor back to the top.

      **Feedback is a count, not a spinner.** `60000 items, 1 selected ·
      refreshing, 27136 read…`. Every chunk is already a `Message::Listed`,
      so the number repaints itself with no timer subscription and no
      animated glyph whose font support would have to be checked. It also
      says how big the directory is turning out to be, which a spinner
      cannot.

      Measured in the rig on 60,000 entries: fourteen frames taken flat out
      across one F5, and the old listing is up in every one of them.

      Not a modal, deliberately. The tile being refreshed stays usable, the
      other tiles are not involved, and on a first listing a modal would
      cover the rows as they stream in.

- [x] **A reading bar, for the mounts that are slow.** The count alone was
      the wrong answer, and the question "how would this behave on an FTP or
      sshfs mount" is what showed it: the count moves when a chunk lands, so
      on a slow mount it moves rarely and on a hung one never. That is
      exactly when a still window has to be told apart from a dead one.

      A lit block sliding along twelve squares, in the status line, driven by
      `iced::time::every(80ms)` -- subscribed only while a read is out, so an
      idle window wakes for nothing. Quads and no glyph, so there is no font
      support to check, and no rotation, which iced has no transform for.

- [x] **Reading a changed path was on the Elm loop, which was a real fault.**
      `reconcile` called `Entry::read` -- a blocking `symlink_metadata` --
      inside `update`. Microseconds on a local disk, tens of milliseconds
      each over sshfs, and for ever on a hung NFS mount, with the window
      frozen behind it. It broke the project's own rule that file work never
      runs on the loop, and it went in unnoticed an hour before the slow
      mount question found it.

      `buffer::examine` reads the paths on a thread and hands back
      `(path, what it is now)` pairs; `Message::Examined` applies them, with
      the generation checked the way a listing chunk is.

- [x] **F5 was a promise the toolbar made and nothing kept.** The relist
      button's tooltip said `(F5)` and no key was bound. Now it is.
- [x] **MIME resolution.** Parse `/usr/share/mime/globs2` (`weight:type:glob`,
      one per line, already sorted by weight) plus `aliases`, and fall back to
      a small hand-written magic sniffer for the couple of dozen signatures
      worth knowing. No libmagic: it is a C parser for hostile input, which is
      the one thing this program exists to avoid.
- [x] **The scan chain.** `[[scan]]` entries, evaluated in order, first
      non-allow verdict wins. Kinds: `glob`, `regex`, `magic-mismatch`,
      `size`, `command`. `warn` shows a dialogue naming the rule that fired and
      offers open-anyway, reveal-in-terminal, or cancel; `block` refuses and
      says which rule refused.
- [x] **The opener.** The handler table, flatpak preference, argv spawn with
      `kill_on_drop(true)`, stderr captured into the notice line, and a
      hard rule that a file is never executed because it is executable.
- [x] **The no-handler dialogue.** What happens when nothing matches: name the
      type, offer choose-a-program, `xdg-open` once, `xdg-open` always, or
      cancel, and write the config on "always". Refuse the fallback for the
      run-rather-than-open types listed in `## The opener`. This is the first
      dialogue in the program, so it also settles what a dialogue looks like.
- [x] **The notice line.** ricebar's `Bar::warn` idea: a strip that shows
      config errors, refused handlers and failed spawns, because a file manager
      started from a launcher has no terminal to print to.
- [x] **The status line.** Entry count, selection count, selected size, free
      space on the buffer's filesystem, and the active filter.
- [x] **The jobs panel, empty.** The panel and its layout, with nothing to put
      in it until M2. Cheap now, and it stops the sidebar being redesigned
      later.

## M1 stage 1: what was measured

Recorded because the gate is the whole reason the list widget is shaped the
way it is, and because the next person to be tempted by `iced_widget::table`
should have to argue with a number.

Headless sway, 1280x800, software rendering, release build, 2026-09-05.
The tree comes from `dev/make-test-tree.sh`.

- [x] **Frame cost does not follow the directory.** 200 scroll frames, timed
      by the process's own utime plus stime:

      | entries | per frame |
      | --- | --- |
      | 1,000 | 12.55 ms |
      | 100,000 | 12.25 ms |

      A hundredfold more entries, and the same frame. That is the claim in
      `## Known constraints`, now with a number against it. The 12 ms itself is
      llvmpipe in a headless session and says nothing about a real GPU; the
      flatness is the point.

      **Measure this properly or not at all.** The first attempt compared 200
      scroll steps of three notches each, and made the small directory look
      1.6x cheaper -- because 1,000 entries is only 24,000 px tall, so it hit
      the bottom partway through and the rest of the scrolls became no-ops
      that skipped their redraw. Any future run has to keep both directories
      off their end stop.

- [x] **Memory does not follow the directory either.** 215 MB resident at
      100,000 entries against 225 MB at 30. It is all renderer and font atlas;
      the entries themselves do not show up against that.
- [x] **Idle costs nothing.** Zero ticks over five seconds with a window open,
      so nothing is redrawing on a timer.
- [x] **Listing 100,000 entries costs 114 ticks**, against 48 for a small
      directory. Mostly the 100k `symlink_metadata` calls, which is the floor.

      It was 794 ticks before the sort was throttled. `extend` re-sorted every
      accumulated entry on each of the 196 chunks, which is quadratic; sorting
      at most ten times a second cut it sevenfold and nothing about the
      listing looks different. Do not put the sort back in the chunk handler.

- [x] **The cursor bug the screenshot caught.** A 100k directory opened with
      the cursor on `file17`. `rebuild` follows the cursor by name so it stays
      on its entry across a relist, but it ran from the first chunk, so it
      latched onto whichever of the first 512 entries happened to sort first
      and then rode it down the listing. The cursor is now only followed once
      somebody has deliberately placed it.

- [x] **Keyboard is verified now, and it took a tool.** Nothing installed
      here can inject a key press into a headless compositor: `wlrctl
      keyboard` has no `key` action, and `wtype`, `ydotool` and `dotool` are
      all absent. `dev/vkeyboard` is the sibling of ricebar's `vpointer` and
      exists for the same reason -- the capability has to stay up between
      events -- and `dev/keys.sh` drives it.

      It earned its keep immediately. Arrows and `End` were fine, but the
      first keyboard screenshot showed two things every pointer test had
      missed: a 250-character name **wrapped onto a second line** and painted
      over the row below it, and a filename containing newlines was drawn
      **three rows tall**. Both are legal Linux names and both are in
      `dev/make-test-tree.sh` on purpose; neither had ever been looked at.

      The fixes: text is laid out with unbounded width so a long name is
      clipped rather than wrapped, since even `Wrapping::None` breaks a line
      when it is given a real width; and every control character in a name is
      replaced before it is drawn.

      `dev/make-test-tree.sh` had a bug of its own, found the same way: it
      creates a directory at mode 000, which `rm -rf` cannot descend into, so
      running it twice failed halfway and left a tree with no marker file. It
      now chmods before removing -- and the `[ -e "$ROOT" ] && chmod` that was
      the obvious way to write it exits the whole script under `set -e` on
      every first run, which is its own small lesson.

## M1 stage 2: what the trust check turned out to get wrong

Both inherited from ricebar, and both only visible once something was gated on
the answer. ricebar has the same two and has never been bitten, because it
parses nothing whose presence depends on being trusted.

- [x] **`/dev/null` is mode 0666, so it is not a neutral stand-in for "a file
      that exists".** ricebar's config tests all pass `/dev/null` as the path,
      which means every one of them has been running with `trusted: false`.
      Harmless there. Here it silently emptied the handler table before a test
      could look at it. Tests that care about trust write a real file at 0600
      into the temp directory instead.

- [x] **A sticky directory is not a writable one.** `/tmp` and `/var/tmp` are
      both 1777, and the parent check refused any config inside either --
      which would have ruled out the `-c /tmp/rig.toml` way of testing that
      `CLAUDE.md` documents. The sticky bit is exactly the rule that says only
      an owner may rename or unlink their own entries, so the replace-the-file
      attack the check exists to stop cannot happen there. `trustworthy` now
      makes the exception; ricebar's version still does not.

## M1 stage 2: the four opening cases, run

Driven by pointer in a headless sway on 2026-09-06, against a directory built
to be nasty. Screenshots were taken of each, and the point of writing them
down is that these four are what the whole opener exists for -- any change to
`src/open/` has to still pass them.

- [x] **A `.desktop` file is data.** Double-clicked `evil.desktop`, whose
      `Exec=` line would have touched a file. Nothing ran, and the dialogue
      said "evil.desktop would be run, not opened". Verified by the absence of
      the file `Exec=` would have created, not by reading the screen.
- [x] **A double extension is refused, by name.** `invoice.pdf.exe` was
      blocked, and the dialogue named the rule -- `double extension` -- quoted
      its pattern, and said to edit it in the config if that was wrong.
- [x] **A name that lies is caught.** `invoice.pdf` holding an ELF header
      raised the `magic-mismatch` rule as a warning, with `Open it anyway` and
      `Cancel`. This is the one the whole `mime` module exists for.
- [x] **An unknown type asks.** `mystery.bin` offered a program box,
      `xdg-open` once, `xdg-open` always, and cancel. Choosing "always" wrote
      `glob = "*.bin"` with `run = ["xdg-open"]` into the config and said so in
      the notice line, which is the loop that makes the fallback teach the
      config rather than become a hole.

Two things the run found that no test had:

- [x] **The list had no double click.** Only `Enter` activated a row, and
      nothing can inject a key into a headless compositor, so the first three
      cases silently did nothing at all. `mouse_area` has `on_double_click`,
      but this list is a `Widget` rather than a tree of them, so the presses
      are counted in its own `tree::State`. Modifiers came with it: a press
      carries none, so they are kept from the last `ModifiersChanged`, which
      is what `slider` does -- and that is Ctrl-click and Shift-click working
      as a side effect.
- [x] **"Always" could not keep its promise for an unknown type.**
      `mystery.bin` has no MIME type at all, and the first version keyed a
      remembered handler on the type alone: it would have run the program and
      quietly written nothing. It now falls back to the extension, and says
      plainly when there is neither.

## M1 stage 3: what the keyboard found

- [x] **A box that appears without focus swallows what is typed into it.**
      Pressing `/` put the filter box on screen and the following `file1`
      nowhere: the box was drawn but nothing had told it to take the keyboard.
      `iced::widget::operation::focus(id)` unfocuses everything else in the
      same traversal, so nothing has to be asked to let go first. Only visible
      because the keys were actually sent.

- [x] **`text_input` really does leave the vertical arrows alone.** The API
      notes said so and the design leans on it, so it was worth confirming
      rather than trusting: with `file1` typed and the box holding its own
      text cursor, two `Down` presses moved the *list* cursor two rows. There
      is no focus to arbitrate between the filter and the list, and no
      mechanism needs building to do it.

- [x] **A capturing closure cannot identify a subscription.** Watching each
      visible buffer meant one subscription per index, and
      `.map(move |()| Message::Changed(index))` fails to compile at all --
      iced hashes the closure to identify the subscription and rejects one
      that captures. `.with(*index)` then a non-capturing map is the shape,
      which is the same rule `CLAUDE.md` already recorded for ricebar's
      modules.

- [x] **A relist must not clear the filter.** Watching a directory that is
      being filtered would otherwise wipe the box on every change to it, which
      is precisely when somebody is looking for something.

- [x] **The breadcrumbs leaked, briefly.** Each component was a
      `&'static str` from `Box::leak`, in a `view` that runs every frame.

      A test measures it now: resident memory after 2,000 frames against
      40,000 frames later. Put the leak back and it fails at 186 bytes a
      frame. Two seconds to run, and the first evidence that nothing else
      leaks per frame.

## M1 stage 3: the places panel

- [x] **An allow-list of filesystems, not a deny-list.** This machine mounts
      twenty-one things and three of them are worth showing. A deny-list needs
      a new entry every time somebody invents a filesystem, and shows it
      wrongly until then. Docker's overlays are excluded by mount point rather
      than by type, since they are real `ext4` and there are dozens.

- [x] **`mountinfo` has a variable number of fields before ` - `.** The
      filesystem type is found by walking to the separator, never by counting
      columns. The mount point is field five, and the kernel writes a space in
      it as `\040`, so `/media/My Backup` needs unescaping before it is a path.

- [x] **A user directory pointing at `$HOME` means there is none.** That is
      what the spec says, and drawing it puts Home in the list twice.

- [x] **`PUBLICSHARE` is not "Publicshare".** Two of the spec's names are one
      word that reads as two, so they are written out rather than folded.

- [x] **A one-pixel divider needs a height.** `container(Space::new())` with a
      background and no height collapses to nothing, which is a divider that
      is not there. Confirmed the fix by counting pixels rather than looking:
      one column of `#45475a` at x=180.

## M1 stage 3: the icons

- [x] **The codepoints were harvested, not looked up.** `lsd --icon always`
      was run over a directory holding one file per extension, and the glyphs
      it printed were read back with a script. `CLAUDE.md` says to verify a
      codepoint by rendering it rather than from memory, and this is the
      cheapest way to obey that: the table now matches the tool the user
      already looks at every day.

      The pasted listing that started this work arrived with every glyph
      missing, which is the same rule from the other side. Private-use
      codepoints do not survive a copy and paste.

- [x] **The glyphs have their own font.** `[list] icon-font`, defaulting to
      `Symbols Nerd Font`, which is glyphs and nothing else. The names keep
      the window's family, so a readable face and a face that has the icons
      can be two different things. Setting `icons = false` draws none at all,
      which is what a machine with no Nerd Font wants rather than a column of
      empty boxes.

      This needed the widget's renderer bound to be
      `text::Renderer<Font = iced::Font>` rather than left open. The first
      attempt downcast an `Any` to reach the concrete type and did not
      compile; naming the type is both shorter and true.

- [x] **A link is a link before it is a directory.** A working symlink to a
      directory was drawn as a plain folder, hiding the one thing its name
      cannot tell you. Caught by a test that was written before the code was
      read back. `lsd` shows the link glyph too.

## M1 stage 3: three layouts from one piece of arithmetic

- [x] **A list is a grid one cell wide.** Saying that once turned three
      layouts into one set of sums. `grid(width) -> (across, line_height)` is
      the whole difference between them, and the visible range, the hit test,
      the reveal and the scrollbar all read it. The alternative -- a trait with
      an implementation each -- would have been three copies of the same
      arithmetic with three chances to get the scroll clamp wrong.

- [x] **The grid hit test was checked by clicking, not by reading.** A click
      at (574, 346) selected `file.patch` and nothing else, which is the cell
      that was under the pointer. A hit test that is off by one column is
      invisible in a screenshot of a selection.

- [x] **The mode column has to read like `ls -l`.** It is the one field people
      compare against another tool, so `drwxr-xr-x` and `lrwxrwxrwx` are unit
      tested rather than eyeballed.

- [x] **A date this year drops the year.** `9 Sep 09:19` against
      `9 Sep  2025`, which is what `ls -l` does and for the same reason: the
      year is noise on a file saved this morning.

## M1 stage 3: tiles

- [x] **A split makes a new buffer, not a shared one.** The decided item says
      two tiles on one directory share a listing and a watcher, and they do --
      but that is what happens when a tile is *pointed at* an existing buffer,
      not what a split should do. A file manager splits to get two directories
      side by side, and a split that moved both panes together would be
      useless. The sharing move needs a buffer list, which is not built.

- [x] **The last tile cannot be closed.** A window with no tiles shows nothing
      and offers no way back.

- [x] **The focused tile is edged in the accent.** With two tiles and no mark,
      nothing on screen says which one the keyboard will reach.

- [x] **The rig's display number is not `wayland-2`.** Restarting sway while
      an old socket is still on disk gives the new one `wayland-3`, and a
      client sent at the old number looks exactly like a client that will not
      start -- which cost a few minutes. `dev/rig.sh` asks the compositor
      which display it is on and prints the environment to `eval`.
- [x] **Waiting for a clock is not waiting.** The rig slept four seconds for a
      sway that is ready in 0.28, and five for a ricedir that maps a window in
      0.5. Every visual check cost nine seconds and should have cost one.
      Measured, then replaced with polling: 0.73 s from nothing to a
      screenshot. `dev/wait-for-window.sh` is the half that waits for the
      window, because a shot taken before the first frame is a shot of the
      wallpaper and looks exactly like a rendering bug.

## M1 stage 3: the rubber band, and a layer that hid it

- [x] **A primitive issued after `with_layer` returns is drawn underneath it.**
      The band was painted just after the closure that clips the rows, and it
      appeared only in the strip below the last row. Everywhere else the rows
      were on top of it.

      What made this expensive: a quad at 20% alpha that is mostly hidden
      looks exactly like a quad that was never drawn. Three attempts went into
      checking the geometry, the state and the event delivery -- all of which
      were right -- before a **solid red fill** settled in one run that it was
      painting, and a pixel scan found it at y 678..728 instead of 250..730.

      Reach for the loud colour sooner. "Is it drawn?" and "is it visible?"
      are different questions and the cheap experiment separates them.

- [x] **The band carries geometry, not a selection.** The widget changes its
      own scroll offset and nothing else; ranges go out and the application
      turns them into indices, so the cost follows what is selected rather
      than what the directory holds.

## M1 stage 3: three faults a real session found

Reported from Hyprland on 2026-09-09, after the first install. Everything else
held up: tiles, splitting, resizing, the three layouts, copy path, the places
panel, adding a favourite, and Okular opening a PDF through flatpak.

- [x] **Two tiles on one directory relisted each other forever.** The
      flickering after a split. inotify reports a directory *read* as
      `Access(Open)`, so a relist opens the directory, the other tile's watch
      fires, it relists, and round it goes -- measured at 33 events in four
      seconds. One tile never starts the loop, because nothing reads the
      directory again after the first listing.

      `watch.rs` now ignores any event that cannot change a listing. Idle CPU
      after a split went from 7 ticks in four seconds to 0, and a file made in
      a terminal still appears in both tiles.

      The first guess was wrong and worth recording: the watch set was a
      `HashSet`, whose order changes between calls, so the subscription batch
      was shuffled after every update. That is a real fault and it is fixed --
      it is a `BTreeSet` now -- but it was not this one. Measuring beat
      reasoning again.

- [x] **A click did not move the keyboard to the tile it landed in.** The list
      widget captures its own presses, so `pane_grid`'s `on_click` never saw
      them. You could select a row in one tile and find the arrows moving the
      other tile's cursor -- which is the "select-bar is on the top folder and
      difficult to do anything" in the report. Anything done to a tile now
      focuses it first.

- [x] **`emacs` is not necessarily a window.** The generated config opened
      text with `emacs -- {path}`, and the build here is emacs-nox: it links
      no GUI toolkit, so from a launcher it has no terminal, starts, and dies
      without drawing. Nothing on `$PATH` says which build is installed, and
      `vim`, `vi` and `nano` have the same trap.

      A text editor on Linux is usually a terminal program, so the
      terminal pairs come first now and only editors that are certainly their
      own window -- `gnome-text-editor`, `gedit`, `kate`, `mousepad`, `code`
      -- are offered bare.

- [x] **A handler that starts and immediately dies says nothing.** `spawn`
      reported only a process that would not *start*. emacs-nox started and
      then exited, and ricedir said nothing -- which looks like a file that
      will not open.

      The child is watched for 400 ms, then detached. A non-zero exit inside
      that window is reported with the first line it printed. Exit 0 is
      success: `xdg-open` and `flatpak run` both hand over and return.

      stderr was piped and never read. A player writing more than a pipe
      holds would have blocked on it. It is drained to a sink now.

      Rig: a failing handler names what it printed; one that keeps running
      says nothing.

## M1 stage 3: the toolbar, and where a menu goes

- [x] **The view-wide switches moved out of the context menu.** Layout, hidden
      files, relist, split and close are on the toolbar now. A right click
      used to open a list of settings rather than a list of things to do to
      the file under the pointer, and a button can also *show* whether a thing
      is on, which a menu item cannot.

- [x] **A button does not know where it is.** iced tells `update` no geometry,
      so a menu opened from the toolbar had nowhere to go and appeared in the
      top-left corner. The pointer is tracked with `event::listen_with` -- the
      documented answer -- and a menu from a button opens under the hand that
      opened it. The `fn` handed to `listen_with` must not capture, so it is a
      bare function.

- [x] **A menu is clamped to the window.** The toolbar is at the right-hand
      end of the bar, so its menu ran off the edge and wrapped its longest
      line instead of simply moving left. `config.window` is the size the
      window *opened* at; `window::resize_events` is the only thing that says
      what it is now.

- [x] **Two instances on one display paint nothing.** A blank screenshot
      during this work turned out to be two ricedirs fighting over the same
      Wayland display, not a rendering fault -- which is the single-instance
      guard in `## M3` arriving as a real symptom rather than a nicety.

## M1 stage 3: the divider, and glyphs whose names lie

- [x] **A 1px gap between tiles was invisible.** `pane_grid` leaves the gap
      empty and shows whatever is behind it, and behind it was the same colour
      as the tiles. The grid sits on a `muted` container now and the gap is 4px,
      which is both visible and wide enough to grab. Each tile paints its own
      background, or the divider colour would show through all of it.

- [x] **`split-horizontal` and `split-vertical` do not say which is which.**
      The codicon names describe neither the divider nor the direction the
      panes end up in. Both were rendered and the screenshot magnified before
      either was believed: `U+EB56` is a box divided left and right,
      `U+EB57` top and bottom. Guessing had a 50% chance and no way to notice
      being wrong except somebody pressing the button.

## M1 stage 3: a browser's bar, and a view per buffer

- [x] **The bar reads left to right the way a browser's does.** Relist first,
      then back and forward, then the path; the buttons at the right, and the
      menu last of all wearing `U+F0C9`, the bars a browser puts in that
      corner. None of this is better in the abstract -- it is better because
      most hands already know it, and a file manager is not the place to
      teach a new arrangement.

- [x] **The layout button stopped wearing the menu's glyph.** It had `U+F0C9`
      for the list view, which is the hamburger. Now `U+F03A` for the list,
      `U+F0CE` for the detail table and `U+F009` for the icons -- three
      glyphs that show the arrangement they switch to. Every codepoint was
      rendered before it was believed, the same rule the divider taught.

- [x] **The layout belongs to the buffer, not to the config.** It was read
      from `config.list.layout`, so switching the view in one tile switched
      every tile. Two tiles wanting different views is the ordinary case -- a
      wide detail listing beside a grid of pictures -- and the switch made the
      second tile useless for the thing it was opened for. `Buffer` carries a
      `layout` now, `Buffer::new` takes the config's as a starting value, a
      split copies its parent's, and `Action::Layout` names the buffer it
      acts on. Measured in the rig with three tiles: one switched to icons,
      two stayed lists.

- [x] **Hidden files followed the layout out of the config.** Same reasoning,
      same shape: a tile opened on a `.config` is no reason for the tile beside
      it to fill up with `.git` and `.cache`. Both settings now live in one
      `config::View`, held as `Buffer.view`, so the next per-buffer setting is
      a field rather than another argument at every call -- the sort is the one
      that will ask. `Buffer`'s private `view: Vec<usize>` became `visible` to
      make room for the name.

      Measured in the rig with two tiles: the left showed 8 items with the
      dotfile and its button lit, the right stayed at 7.

- [x] **The filter box and the path bar's text face followed the window, not
      the buffer.** Reported from a real session: type a path, split the
      tile, type in the new one, go back, and the first path could not be
      edited and the bar flashed.

      `App` held one `typing_path` and one `filtering` for the whole window,
      so the draft went to whichever tile took the keyboard next, and the
      edit button read the other tile's state. Both are on `Buffer` now.
      `path_bar` is handed `focused` by the tile that draws it: two tiles can
      show one buffer and both would have claimed it.

      Three more faults fell out of the same report.

- [x] **Every list said "the button came up" on every release.** The list
      published `Dropped` so a drop on the places panel would end the drag.
      A widget hears every event, not only its own, so with two tiles both
      lists said it, and `Message::List` focuses the tile it came from. A
      click on a tile focused it and gave it straight back -- the flash.

      Gone, action and all. The subscription hears every release, whether or
      not a list is under the pointer.

- [x] **A tile whose box holds no keyboard is a dead tile.** The path bar's
      text face stops the list taking keys, because Tab is completion there.
      Move the keyboard away and back and the `text_input` has lost iced's
      focus while the list is still standing aside: nothing takes a
      keystroke. Every change of tile hands the keyboard back.

- [x] **Closing the filter box cleared every filter in the window.** It
      looped over all the buffers, which un-narrowed the listing in the tile
      beside it. Its own buffer only.

- [x] **clippy `pedantic` and `nursery` are on.** 102 warnings the first time.
      Most were style opinions, and 17 of those are now `allow`ed in
      `[workspace.lints.clippy]` with the argument beside each one -- the
      pixel-to-index casts the list widget is made of, `too_many_lines` on a
      `match` over every message, `match_same_arms` where the arms are kept
      apart by their comments.

      What it was worth doing for is the handful that were real:
      `open::spawn` was `async` with nothing to await; `mime::better` returned
      an `Option` that was always `Some`; `translate` returned an `Option` that
      was always `Some`, and its caller had a dead `else` branch;
      `Buffer::new(start.clone(), ..)` cloned a path nothing used again;
      `draw_glyph_sized` took a `&self` it never read; `icon_font` took
      `&Option<String>` and cloned out of it. Thirteen small functions became
      `const fn`, and the protocol's four wire types now derive `Eq`.

      One near-miss worth keeping: `FileList::grid` divided by a cell width
      that goes negative in a tile narrower than one cell. The cast saturates
      to zero so it was never wrong, but nothing said so; it clamps now.

- [x] **ricebar's `vpointer` is here, as `dev/vpointer` and `dev/pointer.sh`.**
      Nothing could click until now, so the toolbar, the context menu, the
      rubber band and the double click were all verified by reading. This was
      wanted the moment a button had to be pressed to test anything, and the
      hidden-files switch is the first thing proved with it.

- [x] **The rig loses the first key of every script.** The compositor gives a
      new virtual keyboard its seat and keymap on a round trip of its own, and
      a key sent before that arrives nowhere -- not refused, not logged. Two
      bindings, `Ctrl+\` and `Ctrl+-`, looked broken for half an hour and were
      both fine; a `KeyPressed` probe showed nothing reaching the widget at
      all. `dev/keys.sh` now starts its example with a `wait`, and says why.

## M2 — the job engine

- [x] **The queue.** A job is a plan built before any byte moves: the full
      entry list, the total size, the conflict policy and the destination. One
      queue, a configurable number of workers, and per-job pause, resume and
      cancel. Cancelling leaves a partial file behind and says so, rather than
      pretending it can undo a half-written copy.

      `jobs/mod.rs`. Copy only, because it only ever creates. Reached from
      the entry menu as "copy to the other tile", which needs no clipboard.
      `[jobs] workers` defaults to two: a copy is bound by the disk.

      - Parents come before their children in the step list.
      - The walk is on a thread. 100,000 entries would hold the window.
      - Pause and cancel are an `Arc<AtomicU8>`, as in `watch.rs`. Cancel
        beats pause, or a held job could never be stopped.
      - A destination that exists is counted and skipped. Skip, overwrite and
        keep both are a question for a person -- see the conflicts item. A
        directory that exists is merged into.
      - A directory cannot be copied into itself or into anything inside it.
      - `create_new`, so a destination made between the plan and the write is
        not overwritten.

      **The `Kind::File` guard in `dispatch` is gone.** It refused every
      writing action, a person's included. The wire still cannot ask for one:
      `Action::of` makes no `File` action, and a test says so. M3 must add an
      origin at `dispatch` before it opens the socket.

      Rig: 57 MB byte for byte with the link copied as a link; 3 GB to 32%,
      53%, done; held at 1.3 GB; cancelled at 1.8 GB and named the part file.
- [x] **Progress that does not flood the loop.** Workers send bytes-done at
      most 30 times a second per job. A message per file would make a copy of
      200k small files slower than the copy itself.

      Throttled at the source rather than coalesced in the drain: five jobs
      is then 150 messages a second, which costs nothing, and the drain has
      nothing left to do. A worker never touches `App`. It owns a
      `tokio::sync::mpsc::Sender` and nothing else.
- [x] **Delete, for good, before the trash exists.** A file manager that
      cannot remove anything is not usable daily.

      `Shift+Delete`, which is what every desktop means by "not to the
      trash". Plain `Delete` stays unbound until there is one. A dialogue
      asks first and says there is no undo.

      **One step per thing chosen, not one per file.** Removing entries one
      at a time by path can be redirected: swap a directory half way down for
      a symlink between the plan and the delete, and `a/b/c` resolves
      somewhere else. `std::fs::remove_dir_all` uses `openat` and `unlinkat`
      against that race. The plan still walks the tree, to say how big it is.

      The cost: progress and cancel land between things, not inside one. Per
      entry waits for the item below.

      Rig: a symlink was unlinked and its target stayed; cancel changed
      nothing; a directory went with its contents.

- [x] **Conflicts.** Skip, overwrite, keep both, newer only and larger only,
      asked once for the whole job. The plan already knows every clash, so
      the question comes before any byte moves.

      **A clashing step is kept, not dropped.** It carries whether the source
      is newer and whether it is bigger, both measured during the walk.
      `Plan::resolve` then answers without a second walk and without a stat
      on the Elm loop. Its bytes stay out of the total until the answer says
      it will happen, so the bar is right for "skip" without doing anything.

      A job with clashes waits in `State::Asking`. Nothing is guessed, and
      cancelling the dialogue cancels the job.

      Rig: `notes.txt` onto a `notes.txt` gave `notes (2).txt` holding the
      source, and left the original alone.
- [x] **Move.** `Work::Move`, beside copy and delete.

      On one filesystem it is a `rename`: one step for a whole tree, whatever
      it holds, and no byte read. Across two it cannot be, so it becomes the
      copy steps followed by a removal of each source.

      **`rename` replaces a file at the destination without a word**, so a
      move has to catch a clash itself rather than leave it to the syscall.
      It is checked again at the step, in case something arrived while the
      job waited to be answered.

      **A failed copy removes nothing.** One failure anywhere drops every
      removal in a cross-filesystem move. Leaving a file in both places beats
      leaving it in neither, and working out which source a failure belonged
      to would mean the plan carrying a tree of its own.

- [x] **Drag between tiles.** Onto a directory puts it in there; anywhere
      else in the tile puts it in that tile's directory.

      **A dialogue asks copy, move or cancel.** GTK and Qt read a modifier
      instead. Both work: Ctrl copies and Shift moves without the question
      being drawn. The dialogue is the default because copy and move are not
      the same mistake, and a modifier nobody knows about is not an answer.
      Read at the drop, not at the press -- people reach for it after they
      have started.

      **The widget reports what it last *sent*, not what it was given.**
      Comparing against the drawn marker looks right and is not: moving onto
      the empty space of a tile that draws no marker is `None` against
      `None`, so nothing was published and the drop did nothing at all.

      A drop target is not an action on the tile it crosses. Every other
      message from a list focuses its tile; this one must not, or letting go
      would act on the wrong tile's selection.

      Inside this window only. winit cannot drag out to another application
      on Wayland.
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
- [ ] **Screenshots, into `docs/`.** Moved here from M1: pictures of a
      program that can only look at files would need taking again. The
      sequence belongs in a script beside `dev/rig.conf`.

## Decided: how a job tells the window what changed

Raised on 2026-09-09, settled on 2026-09-10, before M2 is built rather than
during it.

**The decision, in one line.** Everything is a `Message`; `App.jobs` is where
`update` keeps what the messages said; `update` is also the one place that
emits a `protocol::Event` into a `tokio::sync::broadcast` for the socket.

**Most of the problem dissolved on inspection.** A job largely does not need
to tell the window what changed, because inotify already does. A copy writes
files into a directory; if that directory is visible it is watched, so
`watch.rs` fires, debounces 120 ms and relists. A delete makes rows vanish the
same way. A destination nobody is looking at needs nothing at all, and
`ShowBuffer` relists it when it comes back into view.

What inotify cannot see is the only thing needing a new path, and all of it is
*in-flight* state rather than change:

- a file being deleted -- nothing arrives until it is gone, so the greying out
  has no event
- a file filling -- `Create` then `Modify`, but no "37% of the way through"
- job lifecycle: started, paused, failed, finished
- a scan verdict that blocked something

**The traffic splits in two, and conflating them is what made every option
look bad.**

| | progress / in-flight | changes |
| --- | --- | --- |
| how often | 30/sec/job | one per file |
| dropping one | fine, only the latest matters | fatal, the listing diverges |
| ordering | irrelevant | required |
| who reads it | jobs panel, row decoration | list, socket, agents |

Progress as messages is where the 200k flood comes from; changes as a polled
value is how an agent misses an event. Throttled at the source the way `## M2`
already asks -- 30 a second per job -- five jobs is 150 messages a second,
which is nothing. The 200k figure only ever applied to one message per *file*.

**Why not the other three.** They were written up as four parallel options and
they are not parallel:

- *A registry the widgets read during `view`* cannot tell the socket, and `##
  M3` rejects a polling loop for agents in as many words. Its stated cost was
  overblown, though: the list is virtualised, so "which rows are affected" is
  about forty lookups a frame and not 100k. That makes it the right *storage*
  for progress, which is what `App.jobs` now is -- storage, not a second
  transport.
- *An event bus with subscribers* collapses into the message loop. An iced
  widget cannot subscribe to anything: the runtime calls `update` and `draw`
  and that is the whole interface. For the list to subscribe it would hold a
  receiver and poll it in `view`, which is the registry with extra steps. The
  bus is genuinely needed only for the consumer outside the loop -- socket
  clients -- which is exactly where the broadcast goes.
- *Per-entry state on the buffer* is a representation question, orthogonal to
  all of it, and wrong here anyway: `Entry` is rebuilt by the listing thread on
  every relist, and a job *causes* relists, so the flags would flicker unless
  re-applied each time. The in-flight map is keyed by path and lives with the
  job.

**Why `broadcast` and not another mpsc.** Every subscriber gets its own copy
and its own cursor, and a reader that falls behind the ring gets
`RecvError::Lagged(n)` rather than blocking the sender. So a wedged agent is
told "you missed n events, resync" instead of stalling the window, which is
the property that matters when the readers are other people's programs. The
brake in `## M3` then costs nothing: detaching an agent is dropping its
receiver.

**Kept deliberately separate.** The audit log and the trace strip are fed from
the *request* side, not from events. They record what was asked, including
requests that were refused, and a refusal produces no event.

**The cost accepted.** Until `## M1`'s "change the list, do not rebuild it"
lands, a copy into a directory of 100,000 still costs a full relist per 120 ms
burst. That is an argument for finishing that item, not against this design.

**What this asks of the code.** `protocol::Event` is new -- the protocol crate
is request and response today and has no third shape. It has to be `Clone` for
the broadcast, and it is the same enum whether the change came from inotify,
from a job or from a person, so an agent cannot tell who moved the file and
does not need to.

### What was considered, for the record

**What is wanted.** A copy or a delete of something large shows progress in the
jobs panel. The rows it touches show it too: a file being deleted greys out, a
file being written appears and fills. Copying twenty files updates the listing
as each one lands, without a relist per file.

**Why it needs deciding first.** Three things must all see the same events and
none of them may reach into the others: the jobs panel, the file list, and the
socket. A job that calls `buffer.rebuild()` directly couples the engine to the
widgets, and an agent watching over the socket would see nothing.

**The options, as they were written down before the decision.** Kept so the
argument is not reopened from scratch, and because two of the "against" lines
turned out to be wrong -- see the decision above.

- [x] **Everything through `Message`.** A job sends progress into the Elm loop
      and `update` puts it where it belongs. *For:* one path, no new
      machinery, and the socket taps the same place the window does. Against:
      `update` grows a branch per event kind, and a copy of 200k files means
      200k messages unless they are coalesced -- which M1's listing already
      had to learn.
- [ ] **A job registry the widgets read.** Jobs write into `App.jobs`, and the
      panel and the list read it during `view`. *For:* no fan-out at all, and
      a widget takes what it needs. Against: "which rows are affected" becomes
      a lookup on every frame, and nothing tells the socket anything happened.
- [ ] **An event bus with subscribers.** A `broadcast` channel; the panel, the
      list and the socket each subscribe. *For:* the parts genuinely do not
      know about each other, and M3's event stream is the same channel.
      Against: two ways to move information around inside one program, and
      iced already has one.
- [ ] **A per-entry state on the buffer.** Each `Entry` gains "being copied",
      "being deleted", "arriving". *For:* the list draws from what it already
      has, and greying a row is a colour. Against: it is the *job's* state
      living in the *buffer*, and two jobs touching one file need an answer.

**What to work out before choosing:** how the socket's event stream in M3 is
fed, since whatever answers that probably answers this too; and whether a
progress update is a message at all or a value the next frame reads.

Both questions turned out to have the same answer, which is what settled it.
The socket is fed from `update`, and a progress update is a message *and* a
value the next frame reads -- the message carries it, `App.jobs` holds it,
`view` reads it. They were never alternatives.

## M3 — the control surface

Read-only, and the foundation for everything in M4. The point is that an agent
drives ricedir through the *same* actions a person does, and that it can see
what the person is looking at.

- [ ] **The socket.** A line-delimited JSON socket at
      `$XDG_RUNTIME_DIR/ricedir/<display>.sock`, mode `0600`, with the peer's
      uid checked through `SO_PEERCRED` and a mismatch refused. Requests
      invoke actions from M1's registry; that is the whole protocol. Everything
      else in M3 and M4 is a shape on top of this.
- [ ] **The socket is also the single-instance guard.** One process per
      display, as many windows as you like. `ricedir /some/path` with one
      already running hands the path over and exits, and a window opens there
      -- the way a browser opens a URL in the browser you already have.

      No lock file: the socket is the lock. Starting up is
      - [ ] try to connect
      - [ ] connected: send the path, exit 0
      - [ ] refused: unlink the stale socket, bind, become the instance
      - [ ] `--separate` to force a second process anyway, for testing

      The race is two starting at once, both failing to connect and both
      binding. Bind decides it: the loser connects to the winner instead.

      **Why one process and not many.** The socket is one per display and an
      agent asks it to act on "the selection" -- with two processes there are
      two answers and no way to say which was meant. `iced::daemon` was chosen
      in M1 for this: a second window is a second window, not a second
      program. Copying between windows then works because there is one
      clipboard and one set of buffers, which two processes would not have.

      Two displays stay separate, which is right: a nested or headless session
      is a different session, and the rig already relies on that.
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

      Shape settled in `## Decided: how a job tells the window what changed`:
      one `protocol::Event`, emitted from `update` and only from `update`,
      into a `tokio::sync::broadcast`. Same event whichever way the change
      arrived — inotify, a job, or a person — so an agent cannot tell who
      moved the file and has no reason to care. A subscriber that falls behind
      the ring is told `Lagged(n)` and resyncs; it cannot stall the window,
      which is the property that matters when the readers are somebody else's
      programs.
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

- [ ] **Mounting from ricedir, and slow mounts generally.** Raised while
      talking about the reading bar, and moved here from `## M1`. It is not
      browsing, which already works; it is the machinery around a mount that
      answers slowly or not at all, and the first sub-item is a real fault
      rather than a feature.

      Mounted sshfs, NFS and FTP already appear in the places panel: they
      come from `/proc/self/mountinfo` like anything else, and nothing in the
      listing path cares what filesystem it is reading. So *browsing* them
      works today. What is missing is everything around it.
      - [ ] a read that never returns. A hung mount blocks `readdir` in
            uninterruptible sleep, so the listing thread cannot be killed and
            each relist leaks another one. A deadline, and a listing that
            says "this mount is not answering" rather than reading for ever.
      - [ ] mount and unmount from the places panel. `sshfs`, `mount.nfs`
            and `curlftpfs` are ordinary programs and fit the argv rule
            exactly, so this needs no D-Bus and no udisks -- which is the
            usual reason a file manager grows a daemon dependency.
      - [ ] remembering a connection, which means a host and a user in the
            config and the password left to the ssh agent or to `~/.netrc`.
            No credential storage in ricedir.
      - [ ] a bigger `CHUNK` and a longer `SETTLE` when the buffer is on a
            network filesystem, since both are tuned for a local disk.

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

- [x] **Hide and show the left panel.** `F9`, which is where Nautilus and
      Dolphin put it, and a toolbar button. Remembered in `state.toml`.

      The divider goes with it, or the window keeps a rule down its side with
      nothing behind it. `state.toml` holds one table, so writing one field
      means writing them all -- see `remember_view`.

      The glyphs are `cod-layout_sidebar_left` and its `_off`, read out of
      the font's own glyph names with `fontTools` rather than guessed. That
      is the cheap way to obey the rule about never guessing a codepoint.

- [ ] **An icon beside each place and each bookmark.** A house, a folder, a
      disk. `icon.rs` already holds the glyphs.

- [ ] **A third width: the panel as a thin strip of icons.** A small bar above
      PLACES with a chevron at its right end. `<` narrows the panel to the
      icons alone; `>` opens it again. F9 keeps doing gone and back.

      Three widths, so `State::sidebar` stops being a `bool`. Make it an enum
      before writing any of this, or the state file gets a second field that
      means the same thing.

      Needs the icons above first: a thin strip with no icons in it is a
      column of clipped words. A name on hover as well, or the strip says
      nothing.

- [ ] **Devices on the left panel, and a switch for them.** Mounted disks
      show today. An *unmounted* one does not, and a USB stick that is
      plugged in and not yet mounted is exactly when somebody opens a file
      manager.

      Two settings: whether the group appears at all, and which devices count
      -- an allow-list, the way `places::REAL` already filters filesystems.
      Fifteen loop devices would otherwise bury the one that matters.

      From `/sys/block` and `/proc/partitions`, not udisks: no D-Bus.
      Mounting one is `M3`; this only shows that it is there.

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
- [ ] **A path bar with two faces.** Breadcrumbs to look at, plain text to
      take a piece of. One click turns it from one into the other.

      Today it is buttons and only buttons, so there is no way to copy half a
      path. That is a thing people do constantly: they want
      `~/workspace/rust` out of a longer path to paste into a
      terminal, and a row of buttons gives them nothing to drag across.

      **The two faces:**

      - *Resting.* Breadcrumbs, drawn as nicely as the rest of the eye candy
        allows: a separator that is a shape rather than a slash, the last
        component brighter than its parents, a fade on the left when the path
        is too long for the bar.
      - *Touched.* The same path as one line of selectable text. Drag across
        any part of it, `Ctrl+C`, done. Editable too, with completion, which
        is the half `## M1` claimed and never had.

      **What turns it over.** A click on the bar but *not* on a crumb, because
      a click on a crumb already means "go there" and must keep meaning it.
      `Ctrl+L` goes straight to text, which is what a browser taught everyone.
      `Escape` and losing focus go back.

      That split is the whole risk in this item: two meanings for one click,
      told apart only by where it lands. The gap after the last crumb is the
      obvious target, and it is small on a short path. Worth trying a small
      dedicated button at the right-hand end as well, so there is a place to
      click that is never ambiguous.

      **The animation is the point of putting this in M6.** Turning over
      should be a movement, not a swap: the crumb text is in the same place in
      both faces, so it can slide rather than blink. `iced::animation` on the
      separators fading out and the background of the text field fading in.

      **To check before building.** Whether `text_input` can be made
      selectable without being editable, for the case where somebody wants to
      copy a path but not change it. If it cannot, the answer is that the text
      face is simply always editable, which is no loss.

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

## Last, if at all — reload the config without a restart

Moved here on request, from the settled list at the top where it sat as though
it were part of the config work. It is not: everything else in that entry is
built, and this is a separate feature with a poor ratio.

- [ ] **Watch the config file and apply a new one in place.** A file manager is
      started and stopped all day, so a restart costs a second and this costs
      a mechanism: every buffer holds a sort and a layout taken from the config
      at the time it opened, and a reload has to decide which of those to
      overrule and which to leave. The font family is the one key that can
      never reload at all — `Font::with_name` takes a `&'static str` and iced
      fixes the fallback family at start — so even a working reload has an
      exception to explain.

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
- **`Widget::layout` is given no viewport** — its signature is
  `(tree, renderer, limits)`. Nor can `children()`/`diff()` depend on
  visibility, since they run before layout. So virtualisation through child
  `Element`s is not possible without a `responsive`-style relayout hack; a
  widget that *draws* its rows has no such problem, which settles the design.
- **`viewport: &Rectangle` in `draw` is comparable to `layout.bounds()`
  directly**, no transform needed. `Column::draw`
  (`iced_widget-0.14.2/src/column.rs:316-328`) is the culling idiom to copy.
  Inside a `scrollable` the viewport slides down the content rect by the
  translation, and `draw` additionally intersects it with the ancestor
  viewport while `update` does not.
- **0.14 renamed `Widget::on_event` to `update`**, it takes `&Event` and
  returns nothing, and capture is `shell.capture_event()` rather than a
  returned `event::Status`. Only `size`, `layout` and `draw` are required.
- **`scrollable::Id` does not exist.** Use `iced::widget::Id`. The scrolling
  helpers are `iced::widget::operation::{scroll_to, snap_to, snap_to_end,
  scroll_by}` (from `iced_runtime`), *not* `iced_widget::scrollable::*`, which
  re-exports only the `AbsoluteOffset`/`RelativeOffset` types.
- **`iced_runtime::task::blocking` is not reachable through `iced`.**
  `iced::task` re-exports only `Handle` and `Task`. It would have been ideal
  for the directory scan — it runs a closure on a real thread and streams
  results back — so the listing gets a hand-rolled `std::thread` plus a
  `tokio::sync::mpsc` channel instead. `iced::widget::operation::*` *is*
  reachable, because `iced::widget` does `pub use iced_runtime::widget::*`.
- **`Task::abortable()` returns a `Handle`.** That is how an in-flight
  directory scan is cancelled when someone navigates away before it finishes.
- **`event::listen_with` takes a bare `fn` pointer, not a closure**, so it
  cannot capture anything. Relevant to tracking the cursor for context menus.
- **`text_input` does not capture ArrowUp, ArrowDown, PageUp, PageDown or
  Tab** — it handles only Enter, Backspace, Delete, Home, End, the horizontal
  arrows and Escape. So the filter box can keep text focus while the list
  handles vertical navigation, and there is no fight to arbitrate. Escape
  unfocuses it for free.
- **Focus is a widget operation**: `iced::widget::operation::focus(id)`
  unfocuses everything else in one traversal. Nothing in the runtime binds Tab
  to `focus_next`; that has to be wired by hand.
- **`fill_text` re-shapes the text every frame.** For a list, cache a
  `Paragraph` per visible row in `tree::State` and use `fill_paragraph`.
- **`keyboard::Event::KeyPressed` carries `repeat: bool`**, which is how a
  held ArrowDown through 100k rows gets throttled.
- **`iced::daemon` opens no window by itself and does not exit when the last
  one closes.** A `window::open` task is needed to get the first window, and
  `iced::exit()` to stop. `window::open` hands back the `Id` synchronously
  alongside the task, so state can be keyed on it before the window exists.
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
