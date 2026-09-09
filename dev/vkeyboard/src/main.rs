//! A keyboard that stays alive, for testing a Wayland client without one.
//!
//! Nothing on this machine can send a key press into a headless compositor.
//! `wlrctl keyboard` has no `key` action, and `wtype`, `ydotool` and `dotool`
//! are not installed. That gap hid a real bug once already: the file list had
//! no double click, so every pointer-driven test of "open this" did nothing
//! at all, and it took a screenshot to notice.
//!
//! The sibling of ricebar's `vpointer`, and it exists for the same reason:
//! the capability has to stay up between the events, so this holds one
//! virtual keyboard open while it reads a script from stdin.
//!
//!   vkeyboard < script
//!
//! Commands, one per line:
//!
//!   key <name>      press and release, e.g. `key Down`, `key Return`
//!   press <name>    hold
//!   release <name>  let go
//!   type <text>     the rest of the line, character by character
//!   wait <ms>
//!   # a comment
//!
//! Names are the small set below rather than all of xkb: this is for driving
//! a file manager, and a table of forty is easier to read than a dependency
//! on libxkbcommon.

use std::io::{BufRead, Write};
use std::os::fd::AsFd;
use std::time::Duration;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

/// The keymap this keyboard announces.
///
/// Written out and handed over as a file descriptor, which is how the
/// protocol takes one. A plain `us` layout: everything below is a keycode in
/// it, so the table and the map have to agree.
const KEYMAP: &str = r#"xkb_keymap {
    xkb_keycodes { include "evdev+aliases(qwerty)" };
    xkb_types    { include "complete" };
    xkb_compat   { include "complete" };
    xkb_symbols  { include "pc+us+inet(evdev)" };
};
"#;

/// Linux evdev codes. The protocol wants these, not xkb keysyms, and they are
/// offset by 8 from the keycodes the compositor reports.
fn code(name: &str) -> Option<u32> {
    let found = match name {
        "Escape" => 1,
        "Backspace" => 14,
        "Tab" => 15,
        "Return" | "Enter" => 28,
        "Ctrl" | "Control" => 29,
        "Shift" => 42,
        "Alt" => 56,
        "Space" => 57,
        "F2" => 60,
        "Home" => 102,
        "Up" => 103,
        "PageUp" => 104,
        "Left" => 105,
        "Right" => 106,
        "End" => 107,
        "Down" => 108,
        "PageDown" => 109,
        "Insert" => 110,
        "Delete" => 111,
        "Slash" => 53,
        "Backslash" | "backslash" => 43,
        "Grave" | "Backtick" => 41,
        "Equal" => 13,
        "LeftBracket" => 26,
        "RightBracket" => 27,
        "Semicolon" => 39,
        "Apostrophe" => 40,
        "Comma" => 51,
        "Period" => 52,
        "Minus" => 12,
        // Letters and digits, so `type` can spell a path.
        "a" => 30, "b" => 48, "c" => 46, "d" => 32, "e" => 18, "f" => 33,
        "g" => 34, "h" => 35, "i" => 23, "j" => 36, "k" => 37, "l" => 38,
        "m" => 50, "n" => 49, "o" => 24, "p" => 25, "q" => 16, "r" => 19,
        "s" => 31, "t" => 20, "u" => 22, "v" => 47, "w" => 17, "x" => 45,
        "y" => 21, "z" => 44,
        "1" => 2, "2" => 3, "3" => 4, "4" => 5, "5" => 6,
        "6" => 7, "7" => 8, "8" => 9, "9" => 10, "0" => 11,
        _ => return None,
    };
    Some(found)
}

/// Which keycode types this character, and whether it needs shift.
fn typed(character: char) -> Option<(u32, bool)> {
    if character == ' ' {
        return Some((57, false));
    }
    if character.is_ascii_uppercase() {
        return code(&character.to_ascii_lowercase().to_string()).map(|found| (found, true));
    }
    match character {
        '/' => Some((53, false)),
        '.' => Some((52, false)),
        '-' => Some((12, false)),
        '_' => Some((12, true)),
        '\\' => Some((43, false)),
        '`' => Some((41, false)),
        ',' => Some((51, false)),
        _ => code(&character.to_string()).map(|found| (found, false)),
    }
}

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };

        match interface.as_str() {
            "wl_seat" if state.seat.is_none() => {
                state.seat = Some(registry.bind(name, version.min(7), qh, ()));
            }
            "zwp_virtual_keyboard_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ignore ZwpVirtualKeyboardV1);

const PRESSED: u32 = 1;
const RELEASED: u32 = 0;

fn main() {
    let connection = Connection::connect_to_env().expect("no Wayland display");
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();

    let display = connection.display();
    display.get_registry(&qh, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).expect("registry roundtrip");

    let (Some(seat), Some(manager)) = (state.seat.clone(), state.manager.clone()) else {
        eprintln!("vkeyboard: the compositor offers no zwp_virtual_keyboard_manager_v1");
        std::process::exit(1);
    };

    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());

    // The keymap goes over a file descriptor, so it has to exist as one.
    let mut file = tempfile();
    file.write_all(KEYMAP.as_bytes()).expect("write keymap");
    file.flush().expect("flush keymap");
    keyboard.keymap(1, file.as_fd(), KEYMAP.len() as u32);
    queue.roundtrip(&mut state).expect("keymap roundtrip");

    // A plain function rather than a closure: a closure that bumps `time`
    // borrows it for its whole life, and every `keyboard.key(time, ..)` below
    // then cannot read it.
    fn tick(queue: &mut wayland_client::EventQueue<State>, state: &mut State, time: &mut u32) {
        *time += 10;
        queue.flush().expect("flush");
        let _ = queue.roundtrip(state);
    }

    let mut time = 0u32;

    for line in std::io::stdin().lock().lines() {
        let line = line.expect("read stdin");
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));

        match command {
            "key" | "press" | "release" => {
                let Some(found) = code(rest.trim()) else {
                    eprintln!("vkeyboard: no key called `{}`", rest.trim());
                    continue;
                };

                if command != "release" {
                    keyboard.key(time, found, PRESSED);
                    tick(&mut queue, &mut state, &mut time);
                }
                if command != "press" {
                    keyboard.key(time, found, RELEASED);
                    tick(&mut queue, &mut state, &mut time);
                }
            }

            "type" => {
                for character in rest.chars() {
                    let Some((found, shifted)) = typed(character) else {
                        eprintln!("vkeyboard: cannot type `{character}`");
                        continue;
                    };

                    if shifted {
                        keyboard.modifiers(1, 0, 0, 0);
                    }
                    keyboard.key(time, found, PRESSED);
                    tick(&mut queue, &mut state, &mut time);
                    keyboard.key(time, found, RELEASED);
                    tick(&mut queue, &mut state, &mut time);
                    if shifted {
                        keyboard.modifiers(0, 0, 0, 0);
                    }
                }
            }

            // Modifiers are a separate message from the keys they modify, so
            // `ctrl a` is one line rather than three.
            "ctrl" | "shift" | "alt" => {
                let Some(found) = code(rest.trim()) else {
                    eprintln!("vkeyboard: no key called `{}`", rest.trim());
                    continue;
                };

                let mask = match command {
                    "ctrl" => 4,
                    "shift" => 1,
                    _ => 8,
                };

                keyboard.modifiers(mask, 0, 0, 0);
                tick(&mut queue, &mut state, &mut time);
                keyboard.key(time, found, PRESSED);
                tick(&mut queue, &mut state, &mut time);
                keyboard.key(time, found, RELEASED);
                tick(&mut queue, &mut state, &mut time);
                keyboard.modifiers(0, 0, 0, 0);
                tick(&mut queue, &mut state, &mut time);
            }

            "wait" => {
                queue.flush().expect("flush");
                let ms: u64 = rest.trim().parse().unwrap_or(100);
                std::thread::sleep(Duration::from_millis(ms));
                time += ms as u32;
            }

            other => eprintln!("vkeyboard: unknown command `{other}`"),
        }
    }

    queue.flush().expect("flush");
}

/// A file to put the keymap in.
///
/// `/dev/shm` because the compositor has to mmap it, and an unlinked file
/// keeps nothing behind afterwards.
fn tempfile() -> std::fs::File {
    let path = format!("/dev/shm/vkeyboard-{}", std::process::id());
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .expect("open keymap file");
    let _ = std::fs::remove_file(&path);
    file
}
