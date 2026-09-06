//! The file list: a widget that draws only the rows you can see.
//!
//! iced's own `table` builds an `Element` per cell, and a directory can hold
//! 100k entries. This draws rows straight onto the renderer instead, so the
//! cost of a frame follows the height of the window rather than the size of
//! the directory.
//!
//! It owns its scroll offset rather than sitting inside a `scrollable`.
//! `Widget::layout` is handed no viewport and `children()` runs before layout,
//! so laying out only the visible rows is not possible with child elements at
//! all — and owning the offset also makes "keep the cursor on screen" a clamp
//! in the same `update` that moved the cursor, rather than a widget operation
//! that lands a frame later.

use iced::advanced::renderer::{self};
use iced::advanced::text::{self};
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse};
use iced::alignment;
use iced::keyboard::{self, key};
use iced::{Color, Element, Event, Length, Pixels, Point, Rectangle, Size, window};

use crate::buffer::Buffer;
use crate::config;
use crate::entry::{Entry, Kind};

/// How far one notch of a wheel scrolls, matching `iced_widget::scrollable`.
const LINE: f32 = 60.0;

/// Width of the scrollbar, and the gap it leaves against the right edge.
const BAR: f32 = 6.0;

/// Gap between the left edge and the name.
const PADDING: f32 = 8.0;

/// Width reserved for the size column.
const SIZE_COLUMN: f32 = 90.0;

/// What the list wants the application to do about a click or a key.
///
/// The widget changes only its own scroll offset. Everything that touches the
/// buffer goes back as one of these, so the action registry stays the single
/// door through which a selection changes -- the same door an agent uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// A plain click: select this row alone.
    Select(usize),
    /// Ctrl-click: add or remove this row.
    Toggle(usize),
    /// Shift-click or shift-arrow: select from the anchor to this row.
    Extend(usize),
    /// The keyboard moved the cursor without changing the selection.
    Cursor(usize),
    /// Enter, or a double click: open the row.
    Activate(usize),
    /// Backspace: go to the parent directory.
    Leave,
    SelectAll,
    /// Right click, with where it happened, so a menu can be put there.
    Menu {
        row: usize,
        at: Point,
    },
}

/// How long after a click a second one on the same row counts as a double.
///
/// The widget counts these itself: `mouse_area` has `on_double_click`, but
/// this list is a `Widget` rather than a tree of them, so nothing above is
/// watching the presses.
const DOUBLE: std::time::Duration = std::time::Duration::from_millis(400);

/// Scroll offset and what the cursor was, kept across frames.
#[derive(Debug, Default)]
struct State {
    offset: f32,
    /// The cursor as of the last frame. A change means the cursor moved, from
    /// the keyboard or from an agent, and the offset should follow it.
    seen_cursor: usize,
    /// When the last left press landed, and on which row.
    clicked: Option<(usize, std::time::Instant)>,
    /// A mouse press carries no modifiers, so they are kept from the last
    /// `ModifiersChanged` -- the same thing `slider` does.
    modifiers: keyboard::Modifiers,
}

pub struct FileList<'a, Message> {
    buffer: &'a Buffer,
    theme: &'a config::Theme,
    row_height: f32,
    text_size: f32,
    /// Whether this list has the keyboard. The application decides, from which
    /// tile is focused: iced's own focus machinery is not needed, and
    /// `text_input` never captures the vertical arrows anyway, so a filter box
    /// and this list can both be live without fighting.
    focused: bool,
    on_action: Box<dyn Fn(Action) -> Message + 'a>,
}

impl<'a, Message> FileList<'a, Message> {
    pub fn new(
        buffer: &'a Buffer,
        theme: &'a config::Theme,
        list: &config::List,
        text_size: f32,
        focused: bool,
        on_action: impl Fn(Action) -> Message + 'a,
    ) -> Self {
        Self {
            buffer,
            theme,
            row_height: list.row_height.max(1.0),
            text_size,
            focused,
            on_action: Box::new(on_action),
        }
    }

    /// Total height of every row, drawn or not.
    fn content_height(&self) -> f32 {
        self.buffer.rows() as f32 * self.row_height
    }

    /// The furthest the list can be scrolled without showing empty space.
    fn max_offset(&self, height: f32) -> f32 {
        (self.content_height() - height).max(0.0)
    }

    /// Which rows fall inside a viewport of this height at this offset.
    ///
    /// Arithmetic rather than a search, which is what a uniform row height
    /// buys: the cost of a frame does not grow with the directory.
    fn visible(&self, offset: f32, height: f32) -> std::ops::Range<usize> {
        let rows = self.buffer.rows();
        if rows == 0 {
            return 0..0;
        }

        let first = (offset / self.row_height).floor().max(0.0) as usize;
        let last = (((offset + height) / self.row_height).ceil() as usize).min(rows);

        first.min(rows)..last
    }

    /// Which row is under a point, if any.
    fn row_at(&self, point: Point, bounds: Rectangle, offset: f32) -> Option<usize> {
        let row = ((point.y - bounds.y + offset) / self.row_height).floor();
        if row < 0.0 {
            return None;
        }

        let row = row as usize;
        (row < self.buffer.rows()).then_some(row)
    }

    /// Move the offset so a row is fully on screen, if it is not already.
    ///
    /// Only as far as it has to: scrolling a row into view should not also
    /// recentre the list under someone who was reading it.
    fn reveal(&self, offset: f32, row: usize, height: f32) -> f32 {
        let top = row as f32 * self.row_height;
        let bottom = top + self.row_height;

        let offset = if top < offset {
            top
        } else if bottom > offset + height {
            bottom - height
        } else {
            offset
        };

        offset.clamp(0.0, self.max_offset(height))
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for FileList<'_, Message>
where
    Renderer: text::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        // Filling means `layout` hands back the viewport rather than the
        // content, which is exactly what the visible-range arithmetic wants.
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let state = tree.state.downcast_mut::<State>();
        let rows = self.buffer.rows();

        match event {
            // Every frame: follow a cursor that moved, and re-clamp an offset
            // that a relist or a resize has left past the end.
            Event::Window(window::Event::RedrawRequested(_)) => {
                if state.seen_cursor != self.buffer.cursor {
                    state.seen_cursor = self.buffer.cursor;
                    state.offset = self.reveal(state.offset, self.buffer.cursor, bounds.height);
                }
                state.offset = state.offset.clamp(0.0, self.max_offset(bounds.height));
            }

            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !cursor.is_over(bounds) {
                    return;
                }

                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };

                let moved = (state.offset + lines).clamp(0.0, self.max_offset(bounds.height));
                if moved != state.offset {
                    state.offset = moved;
                    shell.capture_event();
                    shell.request_redraw();
                }
            }

            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                let Some(point) = cursor.position_over(bounds) else {
                    return;
                };
                let Some(row) = self.row_at(point, bounds, state.offset) else {
                    return;
                };

                let action = match button {
                    mouse::Button::Right => Some(Action::Menu { row, at: point }),

                    mouse::Button::Left => {
                        let now = std::time::Instant::now();
                        let again = state
                            .clicked
                            .is_some_and(|(last, at)| last == row && now - at < DOUBLE);

                        // A double click opens; the first of the pair has
                        // already selected the row, which is what every file
                        // manager does and what makes the selection visible
                        // before anything happens to it.
                        state.clicked = (!again).then_some((row, now));

                        Some(if again {
                            Action::Activate(row)
                        } else if state.modifiers.command() {
                            Action::Toggle(row)
                        } else if state.modifiers.shift() {
                            Action::Extend(row)
                        } else {
                            Action::Select(row)
                        })
                    }

                    _ => None,
                };

                if let Some(action) = action {
                    shell.publish((self.on_action)(action));
                    shell.capture_event();
                }
            }

            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }

            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if !self.focused || rows == 0 {
                    return;
                }

                let page = (bounds.height / self.row_height).floor().max(1.0) as usize;
                let cursor_row = self.buffer.cursor;

                let moved = match key {
                    keyboard::Key::Named(key::Named::ArrowDown) => {
                        Some((cursor_row + 1).min(rows - 1))
                    }
                    keyboard::Key::Named(key::Named::ArrowUp) => Some(cursor_row.saturating_sub(1)),
                    keyboard::Key::Named(key::Named::PageDown) => {
                        Some((cursor_row + page).min(rows - 1))
                    }
                    keyboard::Key::Named(key::Named::PageUp) => {
                        Some(cursor_row.saturating_sub(page))
                    }
                    keyboard::Key::Named(key::Named::Home) => Some(0),
                    keyboard::Key::Named(key::Named::End) => Some(rows - 1),
                    _ => None,
                };

                if let Some(row) = moved {
                    // Shift turns any movement into an extension, which is
                    // what every list in every desktop already does.
                    let action = if modifiers.shift() {
                        Action::Extend(row)
                    } else {
                        Action::Select(row)
                    };
                    shell.publish((self.on_action)(action));
                    shell.capture_event();
                    return;
                }

                let action = match key {
                    keyboard::Key::Named(key::Named::Enter) => Some(Action::Activate(cursor_row)),
                    keyboard::Key::Named(key::Named::Backspace) => Some(Action::Leave),
                    keyboard::Key::Character(character)
                        if character.as_str() == "a" && modifiers.command() =>
                    {
                        Some(Action::SelectAll)
                    }
                    _ => None,
                };

                if let Some(action) = action {
                    shell.publish((self.on_action)(action));
                    shell.capture_event();
                }
            }

            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };

        let state = tree.state.downcast_ref::<State>();
        let offset = state.offset.clamp(0.0, self.max_offset(bounds.height));

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                ..renderer::Quad::default()
            },
            self.theme.background.color(),
        );

        // Clip to the list, so a row half off the bottom is cut rather than
        // drawn over whatever is below.
        renderer.with_layer(visible, |renderer| {
            for row in self.visible(offset, bounds.height) {
                let Some(entry) = self.buffer.at(row) else {
                    continue;
                };

                let top = bounds.y + (row as f32 * self.row_height) - offset;
                let rectangle = Rectangle {
                    x: bounds.x,
                    y: top,
                    width: bounds.width,
                    height: self.row_height,
                };

                self.draw_row(renderer, entry, row, rectangle);
            }
        });

        self.draw_scrollbar(renderer, bounds, offset);
    }

    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::None
        }
    }
}

impl<Message> FileList<'_, Message> {
    fn draw_row<Renderer: text::Renderer>(
        &self,
        renderer: &mut Renderer,
        entry: &Entry,
        row: usize,
        bounds: Rectangle,
    ) {
        let selected = self.buffer.is_selected(row);
        let under_cursor = row == self.buffer.cursor;

        // The cursor is drawn even when the list is not focused, but dimmer,
        // so a split view still says where each side would resume.
        if selected || under_cursor {
            let fill = if under_cursor && self.focused {
                self.theme.accent.color()
            } else {
                self.theme.muted.color()
            };

            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    ..renderer::Quad::default()
                },
                fill,
            );
        }

        let colour = match entry.kind {
            Kind::Link { broken: true, .. } => self.theme.urgent.color(),
            Kind::Directory
            | Kind::Link {
                directory: true, ..
            } => self.theme.accent.color(),
            _ => self.theme.foreground.color(),
        };
        // On the cursor the fill is the accent, so accent text would vanish.
        let colour = if under_cursor && self.focused {
            self.theme.background.color()
        } else {
            colour
        };

        let name = if entry.kind.is_directory() {
            format!("{}/", one_line(&entry.name))
        } else {
            one_line(&entry.name)
        };

        self.draw_text(
            renderer,
            name,
            Rectangle {
                x: bounds.x + PADDING,
                width: (bounds.width - SIZE_COLUMN - BAR - PADDING * 2.0).max(0.0),
                ..bounds
            },
            colour,
            text::Alignment::Left,
        );

        if !entry.kind.is_directory() {
            self.draw_text(
                renderer,
                human_size(entry.size),
                Rectangle {
                    x: bounds.x + bounds.width - SIZE_COLUMN - BAR - PADDING,
                    width: SIZE_COLUMN,
                    ..bounds
                },
                if under_cursor && self.focused {
                    self.theme.background.color()
                } else {
                    self.theme.dim.color()
                },
                text::Alignment::Right,
            );
        }
    }

    /// Draw one string inside a rectangle.
    ///
    /// `fill_text` reshapes on every frame, which would matter if the row
    /// count mattered -- but only the rows on screen are ever drawn, so this
    /// is a few dozen strings a frame rather than a directory's worth.
    fn draw_text<Renderer: text::Renderer>(
        &self,
        renderer: &mut Renderer,
        content: String,
        bounds: Rectangle,
        colour: Color,
        align_x: text::Alignment,
    ) {
        if bounds.width <= 0.0 {
            return;
        }

        let position = match align_x {
            text::Alignment::Right => Point::new(bounds.x + bounds.width, bounds.center_y()),
            _ => Point::new(bounds.x, bounds.center_y()),
        };

        renderer.fill_text(
            text::Text {
                content,
                // Unbounded width. A row is one line, and a name too long for
                // it must run off the edge and be clipped, not wrap: with a
                // real width even `Wrapping::None` breaks a 250-character
                // name across two lines and over the row below it.
                bounds: Size::new(f32::INFINITY, bounds.height),
                size: Pixels(self.text_size),
                line_height: text::LineHeight::default(),
                font: renderer.default_font(),
                align_x,
                align_y: alignment::Vertical::Center,
                shaping: text::Shaping::Advanced,
                // A file name is one line. Wrapping it would push the rest of
                // the row out of a fixed-height layout.
                wrapping: text::Wrapping::None,
            },
            position,
            colour,
            bounds,
        );
    }

    fn draw_scrollbar<Renderer: renderer::Renderer>(
        &self,
        renderer: &mut Renderer,
        bounds: Rectangle,
        offset: f32,
    ) {
        let content = self.content_height();
        if content <= bounds.height {
            return;
        }

        let visible = (bounds.height / content).clamp(0.0, 1.0);
        // A thumb that shrinks with the directory becomes impossible to grab,
        // so it stops at a size a pointer can find.
        let height = (bounds.height * visible).max(24.0);
        let travel = bounds.height - height;
        let progress = offset / self.max_offset(bounds.height).max(1.0);

        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    x: bounds.x + bounds.width - BAR,
                    y: bounds.y + travel * progress.clamp(0.0, 1.0),
                    width: BAR,
                    height,
                },
                border: iced::Border {
                    radius: (BAR / 2.0).into(),
                    ..iced::Border::default()
                },
                ..renderer::Quad::default()
            },
            self.theme.muted.color(),
        );
    }
}

impl<'a, Message, Theme, Renderer> From<FileList<'a, Message>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: text::Renderer + 'a,
{
    fn from(list: FileList<'a, Message>) -> Self {
        Self::new(list)
    }
}

/// A filename as one line of text.
///
/// Every byte except `/` and NUL is legal in a Linux filename, newlines and
/// tabs included, and `dev/make-test-tree.sh` makes some on purpose. Drawn
/// as they are, a name with two newlines in it is three lines tall and paints
/// over the rows either side of it.
fn one_line(name: &str) -> String {
    if !name.chars().any(|c| c.is_control()) {
        return name.to_owned();
    }

    // U+2400 SYMBOL FOR NULL and its neighbours would be prettier, but a font
    // that has them is not a font anyone is guaranteed to have.
    name.chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// A size a person can read at a glance.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];

    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes} B")
    } else if size < 10.0 {
        format!("{size:.1} {}", UNITS[unit])
    } else {
        format!("{size:.0} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole performance claim rests on this: the number of rows drawn
    /// follows the height of the window, not the size of the directory.
    #[test]
    fn only_a_windowful_is_ever_visible() {
        let rows = visible_range(100_000, 24.0, 0.0, 600.0);
        assert!(
            rows.len() <= 27,
            "drew {} rows for a 600px list",
            rows.len()
        );

        let deep = visible_range(100_000, 24.0, 1_000_000.0, 600.0);
        assert!(
            deep.len() <= 27,
            "drew {} rows for a 600px list",
            deep.len()
        );
    }

    /// Scrolling to the very end must not ask for a row that is not there.
    #[test]
    fn the_last_page_stays_inside_the_directory() {
        let rows = visible_range(10, 24.0, 240.0, 600.0);
        assert!(rows.end <= 10, "range {rows:?} runs past the last entry");
    }

    /// An empty directory has no rows to draw and must not underflow.
    #[test]
    fn an_empty_directory_draws_nothing() {
        assert_eq!(visible_range(0, 24.0, 0.0, 600.0), 0..0);
    }

    /// A name with a newline in it is three lines tall if it is drawn as it
    /// is, and paints over the rows either side. Legal on Linux, and in the
    /// test tree on purpose.
    #[test]
    fn a_name_is_always_one_line() {
        assert_eq!(one_line("plain.txt"), "plain.txt");
        assert_eq!(one_line("with\na\nnewline").lines().count(), 1);
        assert!(!one_line("with\ta\ttab").contains('\t'));
        assert!(!one_line("bell\u{7}here").chars().any(char::is_control));
        // Anything that is not a control character survives untouched,
        // including the ones that make a name awkward for a shell.
        assert_eq!(one_line("zażółć-gęślą.txt"), "zażółć-gęślą.txt");
        assert_eq!(one_line("; rm -rf ~ #"), "; rm -rf ~ #");
    }

    /// Sizes are read at a glance, so the units matter more than the digits.
    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1024), "1.0 K");
        assert_eq!(human_size(1024 * 1024 * 4), "4.0 M");
        assert_eq!(human_size(1024 * 1024 * 100), "100 M");
    }

    /// The arithmetic under [`FileList::visible`], without a buffer to build.
    fn visible_range(
        rows: usize,
        row_height: f32,
        offset: f32,
        height: f32,
    ) -> std::ops::Range<usize> {
        if rows == 0 {
            return 0..0;
        }
        let first = (offset / row_height).floor().max(0.0) as usize;
        let last = (((offset + height) / row_height).ceil() as usize).min(rows);
        first.min(rows)..last
    }
}
