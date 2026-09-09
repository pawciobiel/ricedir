//! A pointer that stays alive.
//!
//! `wlrctl` creates a virtual pointer, sends one event and destroys it again.
//! On a headless seat that flaps the seat's pointer capability, so a client
//! loses `wl_pointer` between the move and the click and never sees the click.
//! This one holds the pointer open for as long as it is reading a script from
//! stdin.
//!
//!   vpointer <width> <height> < script
//!
//! Commands, one per line: `move x y`, `click [left|right|middle]`,
//! `press`/`release`, `scroll <steps>`, `wait <ms>`, `#` for a comment.
//!
//! Taken from ricebar's `dev/record/vpointer` almost unchanged. Half of what
//! ricedir does can only be reached with a pointer -- the toolbar buttons, the
//! context menu, the rubber band, a double click -- so a test rig without one
//! can only exercise the keyboard, and every claim about the rest rests on
//! reading the code. Its sibling `dev/vkeyboard` exists for the same reason.

use std::io::BufRead;
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_output, wl_pointer, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1;
use wayland_protocols_wlr::virtual_pointer::v1::client::zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
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
            "wl_output" if state.output.is_none() => {
                state.output = Some(registry.bind(name, version.min(3), qh, ()));
            }
            "zwlr_virtual_pointer_manager_v1" => {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
            _ => {}
        }
    }
}

delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);

fn main() {
    let mut arguments = std::env::args().skip(1);
    let width: u32 = arguments
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1280);
    let height: u32 = arguments
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(720);

    let connection = Connection::connect_to_env().expect("WAYLAND_DISPLAY names no compositor");
    let mut queue = connection.new_event_queue();
    let handle = queue.handle();
    connection.display().get_registry(&handle, ());

    let mut state = State::default();
    queue.roundtrip(&mut state).expect("the registry answers");

    let manager = state
        .manager
        .clone()
        .expect("this compositor has no zwlr_virtual_pointer_manager_v1");

    let pointer: ZwlrVirtualPointerV1 = if manager.version() >= 2 {
        manager.create_virtual_pointer_with_output(
            state.seat.as_ref(),
            state.output.as_ref(),
            &handle,
            (),
        )
    } else {
        manager.create_virtual_pointer(state.seat.as_ref(), &handle, ())
    };

    queue.roundtrip(&mut state).expect("the pointer is made");

    let start = Instant::now();
    let stdin = std::io::stdin();

    for line in stdin.lock().lines().map_while(Result::ok) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or_default();
        let now = || start.elapsed().as_millis() as u32;

        match command {
            "move" => {
                let x: u32 = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let y: u32 = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                pointer.motion_absolute(now(), x, y, width, height);
                pointer.frame();
            }
            "click" | "press" | "release" => {
                let button = match words.next().unwrap_or("left") {
                    "right" => BTN_RIGHT,
                    "middle" => BTN_MIDDLE,
                    _ => BTN_LEFT,
                };

                if command != "release" {
                    pointer.button(now(), button, wl_pointer::ButtonState::Pressed);
                    pointer.frame();
                }

                if command == "click" {
                    let _ = queue.flush();
                    std::thread::sleep(Duration::from_millis(80));
                }

                if command != "press" {
                    pointer.button(now(), button, wl_pointer::ButtonState::Released);
                    pointer.frame();
                }
            }
            "scroll" => {
                let steps: f64 = words.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
                pointer.axis_source(wl_pointer::AxisSource::Wheel);
                pointer.axis_discrete(
                    now(),
                    wl_pointer::Axis::VerticalScroll,
                    steps * 15.0,
                    steps as i32,
                );
                pointer.frame();
            }
            "wait" => {
                let milliseconds: u64 = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                let _ = queue.flush();
                std::thread::sleep(Duration::from_millis(milliseconds));
            }
            other => eprintln!("vpointer: `{other}` is not a command"),
        }

        let _ = queue.flush();
        let _ = queue.roundtrip(&mut state);
    }
}
