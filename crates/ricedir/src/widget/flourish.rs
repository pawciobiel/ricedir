//! What a drag looks like: the thing in your hand, and what happens when you
//! let go.
//!
//! A drag moves a file without anything on screen saying so. Only the target
//! tile changes colour, and the thing being moved sits where it always was.
//! This draws the missing half: the icon under the pointer, where it went,
//! and a burst at the place it landed.
//!
//! **Quads and text only.** Every effect here is `fill_quad` with animated
//! numbers -- a `Quad` carries a border radius and a real blurred `shadow`,
//! which is enough for a rounded icon, a glow and a ring. Stars and arrows
//! would need the `canvas` feature and a second drawing model; particles
//! would need `wgpu` and would cost the software renderer. Neither is worth
//! it for this.
//!
//! **Nothing here is a `Widget` that takes events.** [`Flourish`] fills the
//! window and passes every event through, because it sits over the tiles and
//! must not eat a click. It also never reads a buffer: `update` puts what is
//! happening into [`State`], and this draws that and nothing else.

use std::time::{Duration, Instant};

use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Text};
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse};
use iced::animation::{Animation, Easing};
use iced::{Color, Element, Event, Length, Point, Rectangle, Size, Vector};

use crate::config;

/// How long the icon takes to reach the row it was dropped on.
const FLIGHT: Duration = Duration::from_millis(220);

/// How long a refused drop takes to spring back to where it started.
const RETURN: Duration = Duration::from_millis(320);

/// How long the ring and the dust last.
const BURST: Duration = Duration::from_millis(420);

/// How long a tile shakes for.
const SHAKE: Duration = Duration::from_millis(260);

/// How far a tile moves at the worst of a shake.
const SHAKE_WIDTH: f32 = 7.0;

/// How many times it crosses the middle while it does.
const SHAKE_TIMES: f32 = 3.5;

/// The side of the icon carried under the pointer.
const GHOST: f32 = 56.0;

/// How many specks of dust a burst throws.
const SPECKS: usize = 28;

/// How far the furthest speck gets.
const SPRAY: f32 = 78.0;

/// How far the ring opens out.
const RING: f32 = 64.0;

/// Everything being animated over the window.
///
/// One of each: a person has one pointer, so there is one thing in the air.
/// Bursts are a list because letting go twice quickly should show both rather
/// than have the second cut the first off.
#[derive(Debug, Default)]
pub struct State {
    /// What is under the pointer right now, while a drag is on.
    pub held: Option<Held>,
    /// Let go, but not yet agreed to. The icon waits where it landed while a
    /// dialogue asks, so what is being decided about stays on screen.
    pub pending: Option<Pending>,
    /// A drop that is still travelling, or springing back.
    pub flight: Option<Flight>,
    pub bursts: Vec<Burst>,
    pub shakes: Vec<Shake>,
}

impl State {
    /// Whether anything still needs a frame.
    ///
    /// The window subscribes to `window::frames()` only while this is true.
    /// A program that holds that subscription open for ever keeps a laptop's
    /// GPU awake for nothing.
    pub fn busy(&self, at: Instant) -> bool {
        self.held.is_some()
            || self.pending.is_some()
            || self.flight.as_ref().is_some_and(|one| one.busy(at))
            || self.bursts.iter().any(|one| one.busy(at))
            || self.shakes.iter().any(|one| one.busy(at))
    }

    /// Drop whatever has finished playing.
    pub fn tidy(&mut self, at: Instant) {
        if self.flight.as_ref().is_some_and(|one| !one.busy(at)) {
            self.flight = None;
        }
        self.bursts.retain(|one| one.busy(at));
        self.shakes.retain(|one| one.busy(at));
    }

    /// How far a tile is pushed sideways, if it is shaking.
    pub fn shake(&self, buffer: usize, at: Instant) -> f32 {
        self.shakes
            .iter()
            .find(|shake| shake.buffer == buffer)
            .map_or(0.0, |shake| shake.offset(at))
    }

    /// Let go on a tile that will take it. The icon waits there.
    ///
    /// Nothing bursts yet. A drop is a question until somebody answers it --
    /// copy or move, and then what to do about a name already taken -- and a
    /// burst over an unanswered dialogue says the file has already moved.
    pub fn asked(&mut self, on: Point, tile: usize) {
        let Some(mut held) = self.held.take() else {
            return;
        };
        held.at = on;
        self.pending = Some(Pending { held, at: on, tile });
    }

    /// The work is really starting: land the icon, burst, and shake the tile.
    pub fn accepted(&mut self, at: Instant) {
        let Some(pending) = self.pending.take() else {
            return;
        };

        self.flight = Some(Flight::to(&pending.held, pending.at, at));
        self.bursts.push(Burst::at(pending.at, at));
        self.shakes.retain(|shake| shake.buffer != pending.tile);
        self.shakes.push(Shake {
            buffer: pending.tile,
            since: at,
        });
    }

    /// Answered with "no", or the plan came to nothing: spring back, quietly.
    pub fn declined(&mut self, at: Instant) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.flight = Some(Flight::back(&pending.held, at));
    }

    /// A drop that went nowhere at all: the icon springs back and nothing
    /// bursts. Nobody is being asked anything, so this needs no answer.
    pub fn refused(&mut self, at: Instant) {
        let Some(held) = self.held.take() else {
            return;
        };
        self.flight = Some(Flight::back(&held, at));
    }
}

/// Let go, and waiting for a person to say what it meant.
#[derive(Debug, Clone)]
pub struct Pending {
    held: Held,
    /// Where it was let go, which is where it will land or burst.
    at: Point,
    /// Which tile takes the shake.
    tile: usize,
}

/// The icon in your hand.
#[derive(Debug, Clone)]
pub struct Held {
    /// The glyph for what was picked up, already resolved by the caller: this
    /// module never reads an entry.
    pub glyph: Option<char>,
    pub label: String,
    /// How many things are held. Drawn as a badge when it is more than one.
    pub count: usize,
    /// Where the pointer is now.
    pub at: Point,
    /// Where the drag began, for a refused drop to spring back to.
    pub from: Point,
    /// Scales the icon up as it is picked up.
    pub grown: Animation<bool>,
}

impl Held {
    pub fn new(
        glyph: Option<char>,
        label: String,
        count: usize,
        pointer: Point,
        at: Instant,
    ) -> Self {
        // Started here, not merely built: an `Animation` that is never told
        // to go stays at the value it was made with, so the icon would come
        // up at its small size and sit there.
        let mut grown = Animation::new(false)
            .easing(Easing::EaseOutBack)
            .duration(Duration::from_millis(170));
        grown.go_mut(true, at);

        Self {
            glyph,
            label,
            count,
            at: pointer,
            from: pointer,
            grown,
        }
    }
}

/// The icon after it has been let go: on its way somewhere, or coming back.
#[derive(Debug, Clone)]
pub struct Flight {
    pub glyph: Option<char>,
    pub label: String,
    pub count: usize,
    pub from: Point,
    pub to: Point,
    /// Whether this is a spring back rather than an arrival. A refused drop
    /// takes longer and overshoots, so the two read differently without a
    /// word being written anywhere.
    pub refused: bool,
    pub since: Instant,
    pub eased: Animation<bool>,
}

impl Flight {
    fn to(held: &Held, on: Point, at: Instant) -> Self {
        Self::new(held, held.at, on, false, FLIGHT, Easing::EaseOutCubic, at)
    }

    fn back(held: &Held, at: Instant) -> Self {
        Self::new(
            held,
            held.at,
            held.from,
            true,
            RETURN,
            Easing::EaseOutElastic,
            at,
        )
    }

    fn new(
        held: &Held,
        from: Point,
        to: Point,
        refused: bool,
        over: Duration,
        easing: Easing,
        at: Instant,
    ) -> Self {
        let mut eased = Animation::new(false).easing(easing).duration(over);
        eased.go_mut(true, at);

        Self {
            glyph: held.glyph,
            label: held.label.clone(),
            count: held.count,
            from,
            to,
            refused,
            since: at,
            eased,
        }
    }

    fn busy(&self, at: Instant) -> bool {
        self.eased.is_animating(at)
    }

    /// Where the icon is now, and how big.
    fn now(&self, at: Instant) -> (Point, f32) {
        let along = self.eased.interpolate(0.0, 1.0, at);
        let point = Point::new(
            self.from.x + (self.to.x - self.from.x) * along,
            self.from.y + (self.to.y - self.from.y) * along,
        );

        // An arrival shrinks into the row it landed on; a spring back stays
        // its own size, because it is going home rather than being absorbed.
        let size = if self.refused {
            1.0
        } else {
            (1.0 - along).mul_add(0.7, 0.3)
        };
        (point, size)
    }
}

/// A ring and a spray of dust, where something landed.
#[derive(Debug, Clone)]
pub struct Burst {
    pub at: Point,
    pub since: Instant,
    dust: Vec<Speck>,
}

impl Burst {
    fn at(at: Point, since: Instant) -> Self {
        Self {
            at,
            since,
            dust: (0..SPECKS).map(Speck::number).collect(),
        }
    }

    fn busy(&self, at: Instant) -> bool {
        at.duration_since(self.since) < BURST
    }

    fn along(&self, at: Instant) -> f32 {
        (at.duration_since(self.since).as_secs_f32() / BURST.as_secs_f32()).clamp(0.0, 1.0)
    }
}

/// One speck of dust, thrown from the middle of a burst.
///
/// The spray is worked out from the speck's number rather than from a random
/// source, so a burst looks the same every time it is drawn -- `draw` runs
/// many times per burst and dust that jumped between frames would be snow.
#[derive(Debug, Clone, Copy)]
struct Speck {
    angle: f32,
    reach: f32,
    size: f32,
}

impl Speck {
    fn number(which: usize) -> Self {
        // The golden angle, which is what stops a fixed spray falling into
        // visible spokes the way a plain division does.
        let turn = which as f32 * 2.399_963_2;
        // Two coprime multipliers, so reach and size do not march in step
        // with the angle and give the spray a pattern.
        let reach = 0.45 + ((which * 7 % 11) as f32 / 11.0) * 0.55;
        let size = 2.0 + ((which * 5 % 7) as f32 / 7.0) * 3.0;
        Self {
            angle: turn,
            reach,
            size,
        }
    }
}

/// A tile being knocked sideways.
#[derive(Debug, Clone, Copy)]
pub struct Shake {
    pub buffer: usize,
    pub since: Instant,
}

impl Shake {
    fn busy(&self, at: Instant) -> bool {
        at.duration_since(self.since) < SHAKE
    }

    /// A sine that dies away. Sideways only: a tile that moves both ways
    /// reads as the window being dragged rather than as something landing.
    fn offset(&self, at: Instant) -> f32 {
        let along = at.duration_since(self.since).as_secs_f32() / SHAKE.as_secs_f32();
        if along >= 1.0 {
            return 0.0;
        }
        let swing = (along * SHAKE_TIMES * std::f32::consts::TAU).sin();
        swing * SHAKE_WIDTH * (1.0 - along)
    }
}

/// The overlay itself: draws [`State`] over the window and takes no events.
pub struct Flourish<'a> {
    state: &'a State,
    theme: &'a config::Theme,
    icons: Option<iced::Font>,
    text_size: f32,
    /// One clock for the whole frame, so nothing drifts against anything else.
    now: Instant,
}

impl<'a> Flourish<'a> {
    pub fn new(state: &'a State, config: &'a config::Config, icons: Option<iced::Font>) -> Self {
        Self {
            state,
            theme: &config.theme,
            icons: config.list.icons.then_some(icons).flatten(),
            text_size: config.window.font_size,
            now: Instant::now(),
        }
    }

    /// The icon, its glow, and the badge, centred on a point.
    fn draw_ghost<Renderer: text::Renderer<Font = iced::Font>>(
        &self,
        renderer: &mut Renderer,
        at: Point,
        scale: f32,
        glyph: Option<char>,
        label: &str,
        count: usize,
    ) {
        let side = GHOST * scale;
        let box_at = Rectangle {
            x: at.x - side / 2.0,
            y: at.y - side / 2.0,
            width: side,
            height: side,
        };

        let accent = self.theme.accent.color();

        // The glow is the quad's own shadow rather than a second quad: a
        // blurred shadow is done on the GPU and costs one draw either way.
        renderer.fill_quad(
            Quad {
                bounds: box_at,
                border: iced::Border {
                    color: accent,
                    width: 1.0,
                    radius: (8.0 * scale).into(),
                },
                shadow: iced::Shadow {
                    color: Color { a: 0.55, ..accent },
                    offset: Vector::new(0.0, 2.0),
                    blur_radius: 26.0 * scale,
                },
                ..Quad::default()
            },
            Color {
                a: 0.92,
                ..self.theme.background.color()
            },
        );

        if let (Some(glyph), Some(font)) = (glyph, self.icons) {
            renderer.fill_text(
                Text {
                    content: glyph.to_string(),
                    bounds: Size::new(side, side),
                    size: (side * 0.46).into(),
                    font,
                    align_x: text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    line_height: text::LineHeight::default(),
                    shaping: text::Shaping::Basic,
                    wrapping: text::Wrapping::None,
                },
                Point::new(at.x, at.y - side * 0.08),
                accent,
                box_at,
            );
        }

        // The name under it, so what is held is readable and not only a
        // coloured square.
        let strip = Rectangle {
            x: at.x - side,
            y: at.y + side / 2.0,
            width: side * 2.0,
            height: self.text_size * 1.6,
        };
        renderer.fill_text(
            Text {
                content: label.to_owned(),
                bounds: strip.size(),
                size: (self.text_size * 0.9).into(),
                font: iced::Font::default(),
                align_x: text::Alignment::Center,
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::None,
            },
            Point::new(at.x, strip.y),
            self.theme.foreground.color(),
            strip,
        );

        if count > 1 {
            let badge = 20.0 * scale;
            let corner = Rectangle {
                x: at.x + side / 2.0 - badge / 2.0,
                y: at.y - side / 2.0 - badge / 2.0,
                width: badge,
                height: badge,
            };
            renderer.fill_quad(
                Quad {
                    bounds: corner,
                    border: iced::Border {
                        radius: (badge / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..Quad::default()
                },
                accent,
            );
            renderer.fill_text(
                Text {
                    content: count.to_string(),
                    bounds: corner.size(),
                    size: (badge * 0.6).into(),
                    font: iced::Font::default(),
                    align_x: text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    line_height: text::LineHeight::default(),
                    shaping: text::Shaping::Basic,
                    wrapping: text::Wrapping::None,
                },
                corner.center(),
                self.theme.background.color(),
                corner,
            );
        }
    }

    /// The ring and the dust.
    fn draw_burst<Renderer: renderer::Renderer>(&self, renderer: &mut Renderer, burst: &Burst) {
        let along = burst.along(self.now);
        let fading = (1.0 - along).powi(2);
        let accent = self.theme.accent.color();

        // Two rings, the second a little behind, which reads as one ring
        // with weight rather than as a circle being scaled.
        for (which, lag) in [(0usize, 0.0f32), (1, 0.18)] {
            let ring = (along - lag).max(0.0) / (1.0 - lag);
            if ring <= 0.0 {
                continue;
            }
            let radius = RING * ring * if which == 0 { 1.0 } else { 0.62 };
            let width = (5.0 * (1.0 - ring)).max(1.0);

            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: burst.at.x - radius,
                        y: burst.at.y - radius,
                        width: radius * 2.0,
                        height: radius * 2.0,
                    },
                    border: iced::Border {
                        color: Color {
                            a: fading * 0.9,
                            ..accent
                        },
                        width,
                        radius: radius.into(),
                    },
                    ..Quad::default()
                },
                Color::TRANSPARENT,
            );
        }

        // Dust. One quad each, thrown out and pulled down, so it falls rather
        // than floating away in a circle.
        for speck in &burst.dust {
            let out = SPRAY * speck.reach * along.sqrt();
            let drop = 34.0 * speck.reach * along * along;
            let side = speck.size * (1.0 - along * 0.6);

            renderer.fill_quad(
                Quad {
                    bounds: Rectangle {
                        x: speck.angle.cos().mul_add(out, burst.at.x) - side / 2.0,
                        y: speck.angle.sin().mul_add(out, burst.at.y) + drop - side / 2.0,
                        width: side,
                        height: side,
                    },
                    border: iced::Border {
                        radius: (side / 2.0).into(),
                        ..iced::Border::default()
                    },
                    ..Quad::default()
                },
                Color {
                    a: fading,
                    ..accent
                },
            );
        }
    }
}

impl<Message, Renderer> Widget<Message, iced::Theme, Renderer> for Flourish<'_>
where
    Renderer: text::Renderer<Font = iced::Font>,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<()>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    /// Nothing. This sits over every tile, and a widget that captured a press
    /// here would make the window unusable while it was on screen.
    fn update(
        &mut self,
        _tree: &mut Tree,
        _event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        _shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
    }

    /// Not `Interaction::None`: the pointer belongs to whatever is underneath,
    /// and this must not turn it back into an arrow over a row.
    fn mouse_interaction(
        &self,
        _tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        mouse::Interaction::None
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &iced::Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        // One layer for the lot. Everything here belongs above the tiles, and
        // a primitive issued after `with_layer` returns goes underneath it --
        // which is how the rubber band came to be drawn below the last row.
        let bounds = layout.bounds();
        renderer.with_layer(bounds, |renderer| {
            for burst in &self.state.bursts {
                self.draw_burst(renderer, burst);
            }

            if let Some(flight) = &self.state.flight {
                let (at, scale) = flight.now(self.now);
                self.draw_ghost(
                    renderer,
                    at,
                    scale,
                    flight.glyph,
                    &flight.label,
                    flight.count,
                );
            }

            // Waiting to be answered. Drawn at full size and not following
            // anything: it has been let go, and it sits where it landed so
            // the dialogue is plainly about it.
            if let Some(pending) = &self.state.pending {
                self.draw_ghost(
                    renderer,
                    pending.at,
                    1.0,
                    pending.held.glyph,
                    &pending.held.label,
                    pending.held.count,
                );
            }

            if let Some(held) = &self.state.held {
                let scale = held.grown.interpolate(0.6, 1.0, self.now);
                self.draw_ghost(
                    renderer,
                    held.at,
                    scale,
                    held.glyph,
                    &held.label,
                    held.count,
                );
            }
        });
    }
}

impl<'a, Message, Renderer> From<Flourish<'a>> for Element<'a, Message, iced::Theme, Renderer>
where
    Message: 'a,
    Renderer: text::Renderer<Font = iced::Font> + 'a,
{
    fn from(flourish: Flourish<'a>) -> Self {
        Self::new(flourish)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    fn held(count: usize) -> Held {
        Held::new(
            None,
            String::from("one.txt"),
            count,
            Point::new(10.0, 10.0),
            Instant::now(),
        )
    }

    /// Nothing animating means no frames subscription, which is what stops a
    /// file manager keeping a laptop's GPU awake while it sits there.
    #[test]
    fn an_idle_flourish_asks_for_no_frames() {
        let state = State::default();
        assert!(!state.busy(Instant::now()));
    }

    /// Holding something is enough on its own: the icon follows the pointer.
    #[test]
    fn holding_something_needs_frames() {
        let state = State {
            held: Some(held(1)),
            ..State::default()
        };
        assert!(state.busy(Instant::now()));
    }

    /// Letting go asks a question. Nothing bursts until it is answered: a
    /// burst over an open dialogue says the file has already moved.
    #[test]
    fn letting_go_asks_and_does_not_burst() {
        let now = Instant::now();
        let mut state = State {
            held: Some(held(2)),
            ..State::default()
        };

        state.asked(Point::new(400.0, 200.0), 1);

        assert!(state.held.is_none(), "it is not in your hand any more");
        assert!(state.pending.is_some(), "it is waiting to be answered");
        assert!(state.flight.is_none(), "and it has not gone anywhere");
        assert!(state.bursts.is_empty(), "nothing has burst");
        assert!(state.shakes.is_empty(), "and nothing has shaken");
        assert!(state.busy(now), "but the icon is on screen, so frames run");
    }

    /// Answered with "no": the icon goes home and still nothing bursts.
    #[test]
    fn a_declined_drop_goes_home_without_a_burst() {
        let now = Instant::now();
        let mut state = State {
            held: Some(held(1)),
            ..State::default()
        };

        state.asked(Point::new(400.0, 200.0), 1);
        state.declined(now);

        assert!(state.pending.is_none());
        assert!(state.flight.is_some_and(|one| one.refused));
        assert!(state.bursts.is_empty());
        assert!(state.shakes.is_empty());
    }

    /// Answered with "yes": all three at once, and every one of them ends.
    #[test]
    fn accepting_starts_the_flight_the_burst_and_the_shake() {
        let now = Instant::now();
        let mut state = State {
            held: Some(held(2)),
            ..State::default()
        };

        state.asked(Point::new(400.0, 200.0), 1);
        state.accepted(now);

        assert!(state.pending.is_none());
        assert!(state.flight.is_some());
        assert_eq!(state.bursts.len(), 1);
        assert_eq!(state.shakes.len(), 1);
        assert!(state.busy(now));

        // Well past the longest of them.
        let later = at(2_000);
        state.tidy(later);
        assert!(!state.busy(later), "and nothing is left running");
        assert!(state.bursts.is_empty());
        assert!(state.shakes.is_empty());
    }

    /// A refused drop springs back and sets off nothing else: a burst would
    /// say something landed when nothing did.
    #[test]
    fn a_refused_drop_only_springs_back() {
        let now = Instant::now();
        let mut state = State {
            held: Some(held(1)),
            ..State::default()
        };

        state.refused(now);

        assert!(state.flight.is_some_and(|one| one.refused));
        assert!(state.bursts.is_empty(), "nothing landed, so nothing bursts");
        assert!(state.shakes.is_empty());
    }

    /// The shake dies away rather than stopping dead, and it only ever moves
    /// the tile it belongs to.
    #[test]
    fn a_shake_dies_away_on_its_own_tile() {
        let now = Instant::now();
        let state = State {
            shakes: vec![Shake {
                buffer: 1,
                since: now,
            }],
            ..State::default()
        };

        assert_eq!(state.shake(0, now), 0.0, "the other tile stays put");

        let early = state.shake(1, at(40)).abs();
        let late = state.shake(1, at(230)).abs();
        assert!(early > late, "{early} should be further than {late}");
        assert_eq!(state.shake(1, at(400)), 0.0, "and it stops");
    }

    /// The spray is worked out from a speck's number, so a burst looks the
    /// same every frame. Dust that moved between frames would be snow.
    #[test]
    fn the_dust_is_the_same_every_time_it_is_drawn() {
        let first = Burst::at(Point::ORIGIN, Instant::now());
        let again = Burst::at(Point::ORIGIN, Instant::now());

        let angles: Vec<f32> = first.dust.iter().map(|speck| speck.angle).collect();
        let same: Vec<f32> = again.dust.iter().map(|speck| speck.angle).collect();
        assert_eq!(angles, same);
        assert_eq!(first.dust.len(), SPECKS);
    }
}
