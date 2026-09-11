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

/// Width kept for the glyph, and the gap after it.
const ICON: f32 = 22.0;

/// Width reserved for the size column.
const SIZE_COLUMN: f32 = 90.0;

/// Width reserved for the modified column, in the detail layout.
const WHEN_COLUMN: f32 = 130.0;

/// Width reserved for the mode column, in the detail layout.
const MODE_COLUMN: f32 = 90.0;

/// One cell of the icon grid.
const CELL: iced::Size = iced::Size::new(110.0, 92.0);

/// What the list wants the application to do about a click or a key.
///
/// The widget changes only its own scroll offset. Everything that touches the
/// buffer goes back as one of these, so the action registry stays the single
/// door through which a selection changes -- the same door an agent uses.
// Not `Copy`: `Band` carries two ranges. Everything here is made once per
// event, so cloning one is not worth a thought.
#[derive(Debug, Clone, PartialEq)]
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
    /// Alt-Left and Alt-Right, through the buffer's own history.
    Back,
    Forward,
    /// Show or hide the names that start with a dot, in this tile.
    ShowHidden,
    /// Put the path of what is selected on the clipboard.
    CopyPath,
    SelectAll,
    /// `/` or Ctrl-F: show the filter box.
    Filter,
    /// Ctrl-1, Ctrl-2, Ctrl-3, or the backtick to cycle.
    Layout(Option<config::Layout>),
    /// Split the tile: right with Ctrl-\, down with Ctrl--.
    SplitRight,
    SplitDown,
    /// Ctrl-w closes the tile, Tab moves between them.
    CloseTile,
    NextTile,
    PreviousTile,
    /// Escape: put away whatever is showing.
    Escape,
    /// Add the directory being shown to the favourite places.
    Bookmark,
    /// Show the list of open directories, to point this tile at one.
    Buffers,
    /// Show which key does what.
    Keys,
    /// Remove what is selected, for good. Asks first.
    Delete,
    /// Show or hide the panel down the left.
    Sidebar,
    /// Turn the path bar over to its text face.
    TypePath,
    /// Read the directory again.
    Relist,
    /// Right click, with where it happened, so a menu can be put there.
    ///
    /// `None` means the click landed past the last row. That is a different
    /// menu, not an absent one: "paste here" and "new folder" are about the
    /// directory, and empty space is where people look for them.
    Menu {
        row: Option<usize>,
        at: Point,
    },
    /// A rubber band was dragged over these cells.
    ///
    /// Geometry rather than a set of indices: the widget changes its own
    /// scroll offset and nothing else, and a band over 100k rows must not
    /// mean 100k numbers crossing on every mouse move.
    Band {
        rows: std::ops::Range<usize>,
        /// `None` in the list layouts, where a band covers whole rows.
        columns: Option<std::ops::Range<usize>>,
        /// How many cells sit side by side. Sent rather than guessed: only
        /// the widget knows the real width, and the application turning a
        /// rectangle into indices needs the stride to be right.
        across: usize,
        /// Whether the band adds to the selection or replaces it.
        add: bool,
    },
    /// Ctrl-I: select what is not selected.
    Invert,
    /// A press on a row, and the pointer has moved far enough to mean it.
    ///
    /// The row is named so that dragging something not selected drags that
    /// one rather than whatever happened to be selected before.
    DragRow(usize),
}

/// How far the pointer moves before a press on a row becomes a drag.
///
/// A press both selects and may begin a drag, so the two cannot be told
/// apart until the pointer moves. Without a threshold a shaky hand turns
/// every click into a drag.
///
/// Public because the places panel asks the same question about its own
/// rows, and two thresholds that happened to agree would stop agreeing.
pub const DRAG: f32 = 6.0;

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
    /// Where a rubber band started and where it is now, in content
    /// coordinates -- the offset is already added in, so scrolling mid-drag
    /// keeps the band over the same files rather than the same pixels.
    band: Option<(Point, Point)>,
    /// A row that was pressed, and where, while it is still undecided
    /// whether this is a click or the start of a drag.
    from_row: Option<(usize, Point)>,
}

pub struct FileList<'a, Message> {
    buffer: &'a Buffer,
    theme: &'a config::Theme,
    row_height: f32,
    text_size: f32,
    layout: config::Layout,
    /// The glyph font, when there is one. `None` draws no icons at all: a
    /// machine with no Nerd Font would otherwise show a column of boxes.
    icons: Option<iced::Font>,
    /// Whether this list has the keyboard. The application decides, from which
    /// tile is focused: iced's own focus machinery is not needed, and
    /// `text_input` never captures the vertical arrows anyway, so a filter box
    /// and this list can both be live without fighting.
    focused: bool,
    /// Which key does what. Read from the config, with the defaults under it.
    keys: &'a crate::keys::Bindings,
    on_action: Box<dyn Fn(Action) -> Message + 'a>,
}

impl<'a, Message> FileList<'a, Message> {
    /// Takes the whole config rather than a field per setting.
    ///
    /// Five of them were being passed one at a time, and each new setting
    /// added another parameter to every call. The widget reads the theme,
    /// the list settings, the bindings and the font size; nothing stops it
    /// reading the handler table, but it has no reason to.
    pub fn new(
        buffer: &'a Buffer,
        config: &'a config::Config,
        icons: Option<iced::Font>,
        focused: bool,
        on_action: impl Fn(Action) -> Message + 'a,
    ) -> Self {
        let list = &config.list;

        Self {
            buffer,
            theme: &config.theme,
            keys: &config.keys,
            row_height: list.row_height.max(1.0),
            text_size: config.window.font_size,
            // The buffer's, not the config's. The config only says what a
            // buffer starts as; after that each tile keeps its own view.
            layout: buffer.view.layout,
            icons: list.icons.then_some(icons).flatten(),
            focused,
            on_action: Box::new(on_action),
        }
    }

    /// How many entries sit side by side, and how tall one line of them is.
    ///
    /// A list is a grid one cell wide. Saying it that way once means the
    /// visible-range arithmetic, the hit test and the reveal are written once
    /// rather than three times.
    fn grid(&self, width: f32) -> (usize, f32) {
        match self.layout {
            config::Layout::Icons => {
                // A tile narrower than one cell gives a negative width here.
                // The cast would saturate to zero anyway, but clamping says
                // so rather than leaving it to be looked up.
                let across = ((width - PADDING * 2.0 - BAR) / CELL.width)
                    .floor()
                    .max(1.0);
                (across as usize, CELL.height)
            }
            _ => (1, self.row_height),
        }
    }

    /// How many lines of cells there are.
    fn lines(&self, width: f32) -> usize {
        let (across, _) = self.grid(width);
        self.buffer.rows().div_ceil(across)
    }

    /// Total height of everything, drawn or not.
    fn content_height(&self) -> f32 {
        // Width is not known here, and the callers that matter pass it in.
        self.buffer.rows() as f32 * self.row_height
    }

    /// Total height for a viewport of this width.
    fn content_height_at(&self, width: f32) -> f32 {
        let (_, line) = self.grid(width);
        self.lines(width) as f32 * line
    }

    /// The furthest the list can be scrolled without showing empty space.
    fn max_offset(&self, height: f32) -> f32 {
        (self.content_height() - height).max(0.0)
    }

    /// The same, for a viewport of a known width.
    fn max_offset_at(&self, bounds: Rectangle) -> f32 {
        (self.content_height_at(bounds.width) - bounds.height).max(0.0)
    }

    /// Which rows fall inside a viewport of this height at this offset.
    ///
    /// Arithmetic rather than a search, which is what a uniform row height
    /// buys: the cost of a frame does not grow with the directory.
    fn visible(&self, offset: f32, bounds: Rectangle) -> std::ops::Range<usize> {
        let rows = self.buffer.rows();
        if rows == 0 {
            return 0..0;
        }

        let (across, line) = self.grid(bounds.width);
        let first_line = (offset / line).floor().max(0.0) as usize;
        let last_line = ((offset + bounds.height) / line).ceil() as usize;

        let first = (first_line * across).min(rows);
        let last = (last_line * across).min(rows);
        first..last
    }

    /// Where one entry is drawn.
    fn cell(&self, index: usize, bounds: Rectangle, offset: f32) -> Rectangle {
        let (across, line) = self.grid(bounds.width);
        let column = index % across;
        let row = index / across;

        match self.layout {
            config::Layout::Icons => Rectangle {
                x: bounds.x + PADDING + column as f32 * CELL.width,
                y: bounds.y + row as f32 * line - offset,
                width: CELL.width,
                height: CELL.height,
            },
            _ => Rectangle {
                x: bounds.x,
                y: bounds.y + row as f32 * line - offset,
                width: bounds.width,
                height: line,
            },
        }
    }

    /// Which entry is under a point, if any.
    fn row_at(&self, point: Point, bounds: Rectangle, offset: f32) -> Option<usize> {
        let (across, line) = self.grid(bounds.width);

        let row = ((point.y - bounds.y + offset) / line).floor();
        if row < 0.0 {
            return None;
        }

        let column = if across == 1 {
            0
        } else {
            let across_from = ((point.x - bounds.x - PADDING) / CELL.width).floor();
            if across_from < 0.0 || across_from >= across as f32 {
                return None;
            }
            across_from as usize
        };

        let index = row as usize * across + column;
        (index < self.buffer.rows()).then_some(index)
    }

    /// Which cells a rubber band covers, as ranges rather than as indices.
    ///
    /// Ranges because a band dragged over a large directory must not put a
    /// hundred thousand numbers on the message queue every time the pointer
    /// moves a pixel. The application turns these into a selection, which
    /// costs what the selection costs and no more.
    fn band_over(&self, from: Point, to: Point, bounds: Rectangle) -> Option<Action> {
        let rows = self.buffer.rows();
        if rows == 0 {
            return None;
        }

        let (across, line) = self.grid(bounds.width);
        let (top, low) = (from.y.min(to.y), from.y.max(to.y));

        let first_line = ((top - bounds.y) / line).floor().max(0.0) as usize;
        let last_line = (((low - bounds.y) / line).ceil().max(0.0) as usize).min(rows);

        if across == 1 {
            let first = first_line.min(rows);
            let last = last_line.min(rows);
            return Some(Action::Band {
                rows: first..last,
                columns: None,
                across: 1,
                add: false,
            });
        }

        let (left, right) = (from.x.min(to.x), from.x.max(to.x));
        let column_of = |x: f32| ((x - bounds.x - PADDING) / CELL.width).floor();
        let first_column = column_of(left).max(0.0) as usize;
        let last_column = (column_of(right).max(0.0) as usize + 1).min(across);

        Some(Action::Band {
            rows: first_line..last_line,
            columns: Some(first_column..last_column),
            across,
            add: false,
        })
    }

    /// Move the offset so a row is fully on screen, if it is not already.
    ///
    /// Only as far as it has to: scrolling a row into view should not also
    /// recentre the list under someone who was reading it.
    fn reveal(&self, offset: f32, row: usize, bounds: Rectangle) -> f32 {
        let (across, line) = self.grid(bounds.width);
        let top = (row / across) as f32 * line;
        let bottom = top + line;

        let offset = if top < offset {
            top
        } else if bottom > offset + bounds.height {
            bottom - bounds.height
        } else {
            offset
        };

        offset.clamp(0.0, self.max_offset_at(bounds))
    }
}

// `Font = iced::Font` rather than the associated type left open. The list is
// only ever drawn by the application's own renderer, and saying so is what
// lets a glyph be drawn in a different face from the name beside it.
impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for FileList<'_, Message>
where
    Renderer: text::Renderer<Font = iced::Font>,
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
                    state.offset = self.reveal(state.offset, self.buffer.cursor, bounds);
                }
                state.offset = state.offset.clamp(0.0, self.max_offset_at(bounds));
            }

            Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                if !cursor.is_over(bounds) {
                    return;
                }

                let lines = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };

                let moved = (state.offset + lines).clamp(0.0, self.max_offset_at(bounds));
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
                    // Empty space. A right click here is about the directory
                    // rather than about a file, so it gets its own menu.
                    if *button == mouse::Button::Right {
                        shell.publish((self.on_action)(Action::Menu {
                            row: None,
                            at: point,
                        }));
                        shell.capture_event();
                        return;
                    }

                    // A drag from here is a rubber band, which is the one
                    // selection gesture a mouse-first file manager cannot be
                    // without.
                    if *button == mouse::Button::Left {
                        let at = Point::new(point.x, point.y + state.offset);
                        state.band = Some((at, at));
                        // An empty band clears the selection at once, so a
                        // click on empty space deselects even if the pointer
                        // never moves. Ctrl held means "keep what I have".
                        if !state.modifiers.command() {
                            shell.publish((self.on_action)(Action::Band {
                                rows: 0..0,
                                columns: None,
                                across: 1,
                                add: false,
                            }));
                        }
                        shell.capture_event();
                    }
                    return;
                };

                let action = match button {
                    mouse::Button::Right => Some(Action::Menu {
                        row: Some(row),
                        at: point,
                    }),

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

                        // Where a drag would have started, if the pointer
                        // moves far enough. Selecting on press and dragging
                        // from the same press is what every file manager
                        // does, so the two cannot be told apart until the
                        // pointer has moved.
                        state.from_row = Some((row, point));

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

            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // A press on a row, and now the pointer has moved far
                // enough: this is a drag, not a click. The threshold is what
                // stops a shaky hand turning every click into one.
                if let Some((row, started)) = state.from_row
                    && let Some(now) = cursor.position()
                    && started.distance(now) > DRAG
                {
                    state.from_row = None;
                    shell.publish((self.on_action)(Action::DragRow(row)));
                    return;
                }

                let Some((from, _)) = state.band else {
                    return;
                };
                let Some(point) = cursor.position() else {
                    return;
                };

                let now = Point::new(point.x, point.y + state.offset);
                state.band = Some((from, now));

                if let Some(action) = self.band_over(from, now, bounds) {
                    shell.publish((self.on_action)(action));
                }
                shell.request_redraw();
            }

            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.from_row = None;

                if state.band.take().is_some() {
                    shell.request_redraw();
                }

                // Nothing is published. A drag is ended by the window, which
                // hears every release wherever it lands. This used to say so
                // itself, and every list in the window said it at once --
                // including the ones the release was nowhere near. Each of
                // those messages focused its own tile, so with two tiles the
                // keyboard ended up wherever the last list happened to
                // report from, and a click on a tile flashed it and gave it
                // straight back.
            }

            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.modifiers = *modifiers;
            }

            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                modifiers,
                ..
            }) => {
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

                // Tab is not in the bindings table. A table that can
                // unbind it can leave a window whose tiles cannot be reached,
                // and `text_input` does not capture it either.
                if matches!(key, keyboard::Key::Named(key::Named::Tab)) {
                    let action = if modifiers.shift() {
                        Action::PreviousTile
                    } else {
                        Action::NextTile
                    };
                    shell.publish((self.on_action)(action));
                    shell.capture_event();
                    return;
                }

                // Everything else comes from the table, so a person can
                // change it and an agent cannot reach past the registry.
                let action = self
                    .keys
                    .action(key, modified_key, *modifiers)
                    .map(|bound| {
                        use crate::keys::Bound;
                        match bound {
                            Bound::Open => Action::Activate(cursor_row),
                            Bound::Leave => Action::Leave,
                            Bound::Back => Action::Back,
                            Bound::Forward => Action::Forward,
                            Bound::SelectAll => Action::SelectAll,
                            Bound::Invert => Action::Invert,
                            Bound::Filter => Action::Filter,
                            Bound::ShowHidden => Action::ShowHidden,
                            Bound::Relist => Action::Relist,
                            Bound::EditPath => Action::TypePath,
                            Bound::CopyPath => Action::CopyPath,
                            Bound::Bookmark => Action::Bookmark,
                            Bound::Buffers => Action::Buffers,
                            Bound::NextLayout => Action::Layout(None),
                            Bound::ListLayout => Action::Layout(Some(config::Layout::List)),
                            Bound::DetailLayout => Action::Layout(Some(config::Layout::Detail)),
                            Bound::IconLayout => Action::Layout(Some(config::Layout::Icons)),
                            Bound::SplitRight => Action::SplitRight,
                            Bound::SplitDown => Action::SplitDown,
                            Bound::CloseTile => Action::CloseTile,
                            Bound::NextTile => Action::NextTile,
                            Bound::PreviousTile => Action::PreviousTile,
                            Bound::Escape => Action::Escape,
                            Bound::Keys => Action::Keys,
                            Bound::Delete => Action::Delete,
                            Bound::Sidebar => Action::Sidebar,
                        }
                    });

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
        let offset = state.offset.clamp(0.0, self.max_offset_at(bounds));

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
            for row in self.visible(offset, bounds) {
                let Some(entry) = self.buffer.at(row) else {
                    continue;
                };

                let rectangle = self.cell(row, bounds, offset);
                match self.layout {
                    config::Layout::Icons => self.draw_tile(renderer, entry, row, rectangle),
                    _ => self.draw_row(renderer, entry, row, rectangle),
                }
            }

            // The band goes inside this layer, not after it. A primitive
            // issued once `with_layer` has returned belongs to the *parent*
            // layer, and the parent is composited underneath: drawn outside,
            // the band appeared only in the strip below the last row, hidden
            // behind the rows everywhere else. It took a solid red quad to
            // see that it was painting at all.
            if let Some((from, to)) = state.band {
                let rectangle = Rectangle {
                    x: from.x.min(to.x),
                    y: from.y.min(to.y) - offset,
                    width: (to.x - from.x).abs(),
                    height: (to.y - from.y).abs(),
                };

                renderer.fill_quad(
                    renderer::Quad {
                        bounds: rectangle,
                        border: iced::Border {
                            color: self.theme.accent.color(),
                            width: 1.0,
                            ..iced::Border::default()
                        },
                        ..renderer::Quad::default()
                    },
                    iced::Color {
                        a: 0.25,
                        ..self.theme.accent.color()
                    },
                );
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
    fn draw_row<Renderer: text::Renderer<Font = iced::Font>>(
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

        // The glyph, in its own face. The name keeps the window's font, so a
        // readable face and a face that has the icons can be two things.
        let indent = match self.icons {
            Some(font) => {
                self.draw_glyph(
                    renderer,
                    crate::icon::of(entry),
                    Rectangle {
                        x: bounds.x + PADDING,
                        width: ICON,
                        ..bounds
                    },
                    colour,
                    font,
                );
                ICON
            }
            None => 0.0,
        };

        let extra = if self.layout == config::Layout::Detail {
            WHEN_COLUMN + MODE_COLUMN
        } else {
            0.0
        };

        self.draw_text(
            renderer,
            name,
            Rectangle {
                x: bounds.x + PADDING + indent,
                width: (bounds.width - SIZE_COLUMN - extra - BAR - PADDING * 2.0 - indent).max(0.0),
                ..bounds
            },
            colour,
            text::Alignment::Left,
        );

        let quiet = if under_cursor && self.focused {
            self.theme.background.color()
        } else {
            self.theme.dim.color()
        };

        if self.layout == config::Layout::Detail {
            self.draw_text(
                renderer,
                when(entry),
                Rectangle {
                    x: bounds.x + bounds.width
                        - SIZE_COLUMN
                        - WHEN_COLUMN
                        - MODE_COLUMN
                        - BAR
                        - PADDING,
                    width: WHEN_COLUMN,
                    ..bounds
                },
                quiet,
                text::Alignment::Left,
            );

            self.draw_text(
                renderer,
                mode(entry),
                Rectangle {
                    x: bounds.x + bounds.width - SIZE_COLUMN - MODE_COLUMN - BAR - PADDING,
                    width: MODE_COLUMN,
                    ..bounds
                },
                quiet,
                text::Alignment::Left,
            );
        }

        if !entry.kind.is_directory() {
            self.draw_text(
                renderer,
                human_size(entry.size),
                Rectangle {
                    x: bounds.x + bounds.width - SIZE_COLUMN - BAR - PADDING,
                    width: SIZE_COLUMN,
                    ..bounds
                },
                quiet,
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

    /// Draw one entry as a tile: a big glyph with its name beneath.
    fn draw_tile<Renderer: text::Renderer<Font = iced::Font>>(
        &self,
        renderer: &mut Renderer,
        entry: &Entry,
        row: usize,
        bounds: Rectangle,
    ) {
        let selected = self.buffer.is_selected(row);
        let under_cursor = row == self.buffer.cursor;

        if selected || under_cursor {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bounds.shrink(4.0),
                    border: iced::Border {
                        radius: 6.0.into(),
                        ..iced::Border::default()
                    },
                    ..renderer::Quad::default()
                },
                if under_cursor && self.focused {
                    self.theme.accent.color()
                } else {
                    self.theme.muted.color()
                },
            );
        }

        let colour = if under_cursor && self.focused {
            self.theme.background.color()
        } else {
            self.colour_of(entry)
        };

        if let Some(font) = self.icons {
            // Two and a half times the text, which is what makes this layout
            // worth having: at the same size it is a list with more gaps.
            Self::draw_glyph_sized(
                renderer,
                crate::icon::of(entry),
                Rectangle {
                    x: bounds.x,
                    y: bounds.y + 10.0,
                    width: bounds.width,
                    height: CELL.height * 0.45,
                },
                colour,
                font,
                self.text_size * 2.5,
                text::Alignment::Center,
            );
        }

        // Two lines of name at most, and the second one clipped. A tile is a
        // fixed size, so a long name cannot be allowed to grow it.
        self.draw_text_in(
            renderer,
            one_line(&entry.name),
            Rectangle {
                x: bounds.x + 4.0,
                y: bounds.y + CELL.height * 0.52,
                width: bounds.width - 8.0,
                height: CELL.height * 0.4,
            },
            colour,
            text::Alignment::Center,
            text::Wrapping::Glyph,
            alignment::Vertical::Top,
        );
    }

    /// The colour an entry's name is drawn in, before the cursor overrides it.
    const fn colour_of(&self, entry: &Entry) -> Color {
        match entry.kind {
            Kind::Link { broken: true, .. } => self.theme.urgent.color(),
            Kind::Directory
            | Kind::Link {
                directory: true, ..
            } => self.theme.accent.color(),
            _ => self.theme.foreground.color(),
        }
    }

    /// Draw one glyph, in the icon face rather than the window's.
    fn draw_glyph<Renderer: text::Renderer<Font = iced::Font>>(
        &self,
        renderer: &mut Renderer,
        glyph: char,
        bounds: Rectangle,
        colour: Color,
        font: iced::Font,
    ) {
        renderer.fill_text(
            text::Text {
                content: glyph.to_string(),
                bounds: Size::new(bounds.width, bounds.height),
                size: Pixels(self.text_size),
                line_height: text::LineHeight::default(),
                font,
                align_x: text::Alignment::Left,
                align_y: alignment::Vertical::Center,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::None,
            },
            Point::new(bounds.x, bounds.center_y()),
            colour,
            bounds,
        );
    }

    /// A glyph at a size of its own, for the tile layout.
    ///
    /// Not a variation on [`Self::draw_glyph`]: it is anchored at the top
    /// centre rather than the left middle, so nothing but the `fill_text`
    /// call is shared. Takes no `self` -- everything it draws with is an
    /// argument.
    #[allow(clippy::too_many_arguments)]
    fn draw_glyph_sized<Renderer: text::Renderer<Font = iced::Font>>(
        renderer: &mut Renderer,
        glyph: char,
        bounds: Rectangle,
        colour: Color,
        font: iced::Font,
        size: f32,
        align_x: text::Alignment,
    ) {
        renderer.fill_text(
            text::Text {
                content: glyph.to_string(),
                bounds: Size::new(bounds.width, bounds.height),
                size: Pixels(size),
                line_height: text::LineHeight::default(),
                font,
                align_x,
                align_y: alignment::Vertical::Top,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::None,
            },
            Point::new(bounds.center_x(), bounds.y),
            colour,
            bounds,
        );
    }

    /// Text inside a box, wrapped and aligned as asked.
    #[allow(clippy::too_many_arguments)]
    fn draw_text_in<Renderer: text::Renderer>(
        &self,
        renderer: &mut Renderer,
        content: String,
        bounds: Rectangle,
        colour: Color,
        align_x: text::Alignment,
        wrapping: text::Wrapping,
        align_y: alignment::Vertical,
    ) {
        if bounds.width <= 0.0 {
            return;
        }

        let position = match align_x {
            text::Alignment::Center => Point::new(bounds.center_x(), bounds.y),
            text::Alignment::Right => Point::new(bounds.x + bounds.width, bounds.y),
            _ => Point::new(bounds.x, bounds.y),
        };

        renderer.fill_text(
            text::Text {
                content,
                bounds: Size::new(bounds.width, bounds.height),
                size: Pixels(self.text_size),
                line_height: text::LineHeight::default(),
                font: renderer.default_font(),
                align_x,
                align_y,
                shaping: text::Shaping::Advanced,
                wrapping,
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
        let content = self.content_height_at(bounds.width);
        if content <= bounds.height {
            return;
        }

        let visible = (bounds.height / content).clamp(0.0, 1.0);
        // A thumb that shrinks with the directory becomes impossible to grab,
        // so it stops at a size a pointer can find.
        let height = (bounds.height * visible).max(24.0);
        let travel = bounds.height - height;
        let progress = offset / self.max_offset_at(bounds).max(1.0);

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
    Renderer: text::Renderer<Font = iced::Font> + 'a,
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

/// When an entry was last written, short enough for a column.
///
/// A date this year is a day and a month; an older one gains the year. That is
/// what `ls -l` does, and for the same reason: the year is noise on a file
/// somebody saved this morning.
fn when(entry: &Entry) -> String {
    let Some(modified) = entry.modified else {
        return String::new();
    };

    let Ok(stamp) = jiff::Timestamp::try_from(modified) else {
        return String::new();
    };
    let at = stamp.to_zoned(jiff::tz::TimeZone::system());
    let now = jiff::Zoned::now();

    if at.year() == now.year() {
        format!(
            "{:>2} {} {:02}:{:02}",
            at.day(),
            month(at.month()),
            at.hour(),
            at.minute()
        )
    } else {
        format!("{:>2} {}  {}", at.day(), month(at.month()), at.year())
    }
}

fn month(number: i8) -> &'static str {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    NAMES
        .get((number as usize).saturating_sub(1))
        .copied()
        .unwrap_or("")
}

/// The mode as `drwxr-xr-x`.
fn mode(entry: &Entry) -> String {
    let bit = |shift: u32, letter: char| {
        if entry.mode & (1 << shift) != 0 {
            letter
        } else {
            '-'
        }
    };

    let kind = match entry.kind {
        Kind::Directory => 'd',
        Kind::Link { .. } => 'l',
        Kind::Other => '?',
        Kind::File => '-',
    };

    let mut out = String::with_capacity(10);
    out.push(kind);
    for (shift, letter) in [
        (8, 'r'),
        (7, 'w'),
        (6, 'x'),
        (5, 'r'),
        (4, 'w'),
        (3, 'x'),
        (2, 'r'),
        (1, 'w'),
        (0, 'x'),
    ] {
        out.push(bit(shift, letter));
    }
    out
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

    /// The mode column is the one thing in a file manager people compare
    /// against `ls -l`, so it has to read the same way.
    #[test]
    fn the_mode_reads_like_ls() {
        let mut entry = Entry {
            name: String::from("x"),
            path: std::path::PathBuf::from("x"),
            kind: Kind::File,
            size: 0,
            modified: None,
            mode: 0o644,
            target: None,
            hidden: false,
        };
        assert_eq!(mode(&entry), "-rw-r--r--");

        entry.mode = 0o755;
        entry.kind = Kind::Directory;
        assert_eq!(mode(&entry), "drwxr-xr-x");

        entry.mode = 0o000;
        entry.kind = Kind::File;
        assert_eq!(mode(&entry), "----------");

        entry.mode = 0o777;
        entry.kind = Kind::Link {
            directory: false,
            broken: false,
        };
        assert_eq!(mode(&entry), "lrwxrwxrwx");
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
