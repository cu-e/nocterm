//! The terminal state machine and the snapshot a renderer draws from.

use std::{
    sync::{Arc, Mutex, PoisonError},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll as GridScroll},
    index::{Column, Line, Point, Side as GridSide},
    selection::{Selection, SelectionType},
    term::{Config, TermMode, cell::Flags, color::COUNT as COLOR_SLOTS, viewport_to_point},
    vte::ansi::{
        Color as AnsiColor, CursorShape as AnsiCursorShape, CursorStyle, NamedColor, Processor,
        Rgb as AnsiRgb,
    },
};

mod input;

/// Narrowest grid the emulator will lay out.
const MIN_COLS: u16 = 2;
/// Shortest grid the emulator will lay out.
const MIN_ROWS: u16 = 1;

/// The size of the grid, and of one of its cells for programs that ask in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl TermSize {
    /// A size of at least the smallest grid the emulator supports.
    pub fn new(cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> Self {
        Self {
            cols: cols.max(MIN_COLS),
            rows: rows.max(MIN_ROWS),
            cell_width,
            cell_height,
        }
    }

    /// Total number of cells on screen.
    pub fn cell_count(&self) -> usize {
        usize::from(self.cols) * usize::from(self.rows)
    }
}

impl Default for TermSize {
    fn default() -> Self {
        Self::new(80, 24, 0, 0)
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.cols)
    }
}

/// An opaque colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// The colour of a cell, still relative to the host's palette where it can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default text colour.
    Foreground,
    /// The terminal's default background.
    Background,
    /// One of the sixteen ANSI colours, `0..=15`.
    Ansi(u8),
    /// A colour the program specified exactly.
    Rgb(Rgb),
}

/// The colours the host draws with, so the emulator can answer programs that
/// ask what they are (`OSC 4`, `OSC 10`, `OSC 11`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            foreground: Rgb::new(0xe5, 0xe5, 0xe5),
            background: Rgb::new(0, 0, 0),
            cursor: Rgb::new(0xe5, 0xe5, 0xe5),
            ansi: std::array::from_fn(|ix| indexed_color(ix as u8)),
        }
    }
}

/// How a cell is drawn besides its colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub dim: bool,
    pub underline: bool,
    pub strikeout: bool,
    /// Swap foreground and background.
    pub inverse: bool,
    /// Occupies its cell but draws nothing.
    pub hidden: bool,
}

/// One cell of the visible grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub style: Style,
    /// The character is two cells wide; the next cell is its [`spacer`](Self::spacer).
    pub wide: bool,
    /// The right half of a wide character. Draw nothing.
    pub spacer: bool,
    /// Inside the selection.
    pub selected: bool,
    /// The active search result, independent from selection and clipboard text.
    pub search_hit: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Foreground,
            bg: Color::Background,
            style: Style::default(),
            wide: false,
            spacer: false,
            selected: false,
            search_hit: false,
        }
    }
}

/// Shape of the cursor as the running program requested it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
    /// An outlined block.
    Hollow,
}

/// The cursor, when it is visible in the current viewport.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: u16,
    pub col: u16,
    pub shape: CursorShape,
    pub blinking: bool,
    /// The cursor sits on a two-cell-wide character.
    pub wide: bool,
}

/// A cell of the viewport: `row` 0 is the top visible line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellPoint {
    pub row: u16,
    pub col: u16,
}

/// Which half of a cell a pointer is over; decides whether that cell is
/// included when a selection starts or ends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// How a selection grows from where it was started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionKind {
    /// Exactly the cells dragged over.
    Cells,
    /// Whole words.
    Words,
    /// Whole lines.
    Lines,
}

/// A request to move the viewport through the scrollback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Positive scrolls back into history, negative towards the present.
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// Something the host has to do because of what the program printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Send these bytes back to the program: the answer to a status query.
    Reply(Vec<u8>),
    /// The program named its window; `None` restores the default name.
    Title(Option<String>),
    /// The program rang the bell.
    Bell,
    /// The program asked to put this text on the clipboard (`OSC 52`).
    CopyToClipboard(String),
}

/// Options that can change while a terminal is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmulatorOptions {
    /// Lines of history kept above the screen.
    pub scrollback_lines: usize,
    /// Cursor shape until the program asks for another.
    pub cursor_shape: CursorShape,
    /// Whether that default cursor blinks.
    pub cursor_blink: bool,
}

impl Default for EmulatorOptions {
    fn default() -> Self {
        Self {
            scrollback_lines: 10_000,
            cursor_shape: CursorShape::Block,
            cursor_blink: true,
        }
    }
}

impl EmulatorOptions {
    fn to_config(self) -> Config {
        Config {
            scrolling_history: self.scrollback_lines,
            kitty_keyboard: true,
            default_cursor_style: CursorStyle {
                shape: match self.cursor_shape {
                    CursorShape::Block => AnsiCursorShape::Block,
                    CursorShape::Bar => AnsiCursorShape::Beam,
                    CursorShape::Underline => AnsiCursorShape::Underline,
                    CursorShape::Hollow => AnsiCursorShape::HollowBlock,
                },
                blinking: self.cursor_blink,
            },
            ..Config::default()
        }
    }
}

/// The output line behind one visible physical row. Soft wraps share identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineMetadata {
    pub number: u64,
    /// Time of first output observation on this machine, in Unix milliseconds.
    pub timestamp_ms: u64,
    pub continuation: bool,
}

/// Everything a renderer needs to draw the terminal once.
///
/// Reuse one `Frame` across draws: [`Emulator::snapshot`] overwrites it in
/// place and keeps its allocations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub size: TermSize,
    /// Row-major: the cell at `(row, col)` is `cells[row * cols + col]`.
    pub cells: Vec<Cell>,
    /// Presentation-only metadata; never included in terminal text or selection.
    pub lines: Vec<Option<LineMetadata>>,
    pub alt_screen: bool,
    /// Whether a visible physical row soft-wraps into the next row, on either screen.
    pub wraps: Vec<bool>,
    /// The first visible row continues a soft-wrapped row above the viewport.
    pub starts_with_continuation: bool,
    /// Combining characters stacked on a cell, as `(cell index, characters)`
    /// in ascending index order.
    pub combining: Vec<(usize, String)>,
    pub cursor: Option<Cursor>,
    /// Lines the viewport is scrolled back by; 0 shows the live screen.
    pub display_offset: usize,
    /// Lines of history above the screen.
    pub history_size: usize,
}

impl Frame {
    /// The cells of one row.
    pub fn row(&self, row: u16) -> &[Cell] {
        let cols = usize::from(self.size.cols);
        let start = usize::from(row) * cols;
        &self.cells[start..start + cols]
    }

    /// The row's text, without trailing blanks. Meant for tests and logs.
    pub fn row_text(&self, row: u16) -> String {
        let text: String = self
            .row(row)
            .iter()
            .filter(|cell| !cell.spacer)
            .map(|cell| cell.ch)
            .collect();
        text.trim_end().to_owned()
    }
}

#[derive(Clone)]
struct Listener(Arc<Mutex<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// A terminal: feed it the bytes a program prints and it keeps the screen.
///
/// The emulator is pure state. It owns no thread, no socket and no window, so
/// the same type backs a visible tab, a test, or a headless recorder.
pub struct Emulator {
    term: Term<Listener>,
    parser: Processor,
    osc_guard: crate::osc_guard::OscGuard,
    events: Arc<Mutex<Vec<Event>>>,
    size: TermSize,
    palette: Palette,
    options: EmulatorOptions,
    generation: u64,
    search_match: Option<(u64, crate::SearchMatch)>,
}

impl Emulator {
    pub fn new(size: TermSize, options: EmulatorOptions) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        Self {
            term: Term::new(options.to_config(), &size, Listener(events.clone())),
            parser: Processor::new(),
            osc_guard: crate::osc_guard::OscGuard::default(),
            events,
            size,
            palette: Palette::default(),
            options,
            generation: 0,
            search_match: None,
        }
    }

    /// Feeds bytes printed by the program and returns what the host must do
    /// about them, in order.
    pub fn advance(&mut self, bytes: &[u8]) -> Vec<Effect> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.advance_at(bytes, now.min(u128::from(u64::MAX)) as u64)
    }

    /// Feed output with an explicit host-observation clock. Useful for replay and tests.
    pub fn advance_at(&mut self, bytes: &[u8], timestamp_ms: u64) -> Vec<Effect> {
        if !bytes.is_empty() {
            self.generation = self.generation.wrapping_add(1);
        }
        self.term.set_output_timestamp_ms(timestamp_ms);
        self.osc_guard
            .advance(bytes, |bytes| self.parser.advance(&mut self.term, bytes));
        self.take_effects()
    }

    /// When a synchronized update the program began must be force-finished.
    ///
    /// A program may ask for output to be held back until it has drawn a full
    /// frame. If it never says it is done, the host calls [`Self::finish_sync`]
    /// once this deadline has passed.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    /// Ends a synchronized update and applies everything it held back.
    pub fn finish_sync(&mut self) -> Vec<Effect> {
        self.generation = self.generation.wrapping_add(1);
        self.parser.stop_sync(&mut self.term);
        self.take_effects()
    }

    /// Highest allocated logical output line, independent of scrollback trimming.
    pub fn output_line_number(&self) -> u64 {
        self.term.output_line_number()
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    pub fn resize(&mut self, size: TermSize) {
        if size != self.size {
            self.generation = self.generation.wrapping_add(1);
            self.size = size;
            self.term.resize(size);
        }
    }

    pub fn set_options(&mut self, options: EmulatorOptions) {
        if self.options == options {
            return;
        }
        self.options = options;
        self.generation = self.generation.wrapping_add(1);
        self.term.set_options(options.to_config());
    }

    /// Tells the emulator which colours the host draws with.
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Begins a selection at a cell of the viewport, replacing any other.
    pub fn start_selection(&mut self, kind: SelectionKind, at: CellPoint, side: Side) {
        let kind = match kind {
            SelectionKind::Cells => SelectionType::Simple,
            SelectionKind::Words => SelectionType::Semantic,
            SelectionKind::Lines => SelectionType::Lines,
        };
        let point = self.grid_point(at);
        self.term.selection = Some(Selection::new(kind, point, grid_side(side)));
    }

    /// Extends the selection to a cell of the viewport.
    pub fn update_selection(&mut self, at: CellPoint, side: Side) {
        let point = self.grid_point(at);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, grid_side(side));
        }
    }

    /// Selects retained history and the screen without touching search state.
    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(grid.topmost_line(), Column(0)),
            GridSide::Left,
        );
        selection.update(
            Point::new(grid.bottommost_line(), Column(grid.columns() - 1)),
            GridSide::Right,
        );
        self.term.selection = Some(selection);
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn search_grid(
        &self,
    ) -> &alacritty_terminal::grid::Grid<alacritty_terminal::term::cell::Cell> {
        self.term.grid()
    }

    pub fn search(
        &self,
        query: &str,
        direction: crate::SearchDirection,
        anchor: Option<crate::SearchPoint>,
    ) -> Result<crate::SearchScan, String> {
        self.search_with_options(query, crate::SearchOptions::default(), direction, anchor)
    }

    pub fn search_with_options(
        &self,
        query: &str,
        options: crate::SearchOptions,
        direction: crate::SearchDirection,
        anchor: Option<crate::SearchPoint>,
    ) -> Result<crate::SearchScan, String> {
        crate::SearchScan::new(self, query, options, direction, anchor)
    }

    pub fn selection_start(&self) -> Option<crate::SearchPoint> {
        let range = self.term.selection.as_ref()?.to_range(&self.term)?;
        Some(crate::SearchPoint {
            line: range.start.line.0,
            column: range.start.column.0 as u16,
            part: 0,
        })
    }

    pub fn clear_search(&mut self) {
        self.search_match = None;
    }

    /// Publishes a current-generation match and brings its start into view.
    pub fn show_search_match(&mut self, result: crate::SearchMatch) {
        let grid = self.term.grid();
        if result.start > result.end
            || result.start.line < grid.topmost_line().0
            || result.end.line > grid.bottommost_line().0
            || usize::from(result.start.column) >= grid.columns()
            || usize::from(result.end.column) >= grid.columns()
        {
            return;
        }
        self.search_match = Some((self.generation, result));
        let row = result.start.line + self.term.grid().display_offset() as i32;
        if row < 0 || row >= i32::from(self.size.rows) {
            let offset = (-result.start.line).max(0) as usize;
            let delta = offset as i32 - self.term.grid().display_offset() as i32;
            self.term.scroll_display(GridScroll::Delta(delta));
        }
    }

    /// The selected text, if anything is selected.
    pub fn selection_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    fn grid_point(&self, at: CellPoint) -> Point {
        let row = at.row.min(self.size.rows - 1);
        let col = at.col.min(self.size.cols - 1);
        viewport_to_point(
            self.term.grid().display_offset(),
            Point::new(usize::from(row), Column(usize::from(col))),
        )
    }

    /// Maps a cell colour to the palette slot or the exact colour to draw.
    fn resolve(&self, color: AnsiColor) -> Color {
        match color {
            AnsiColor::Spec(rgb) => Color::Rgb(from_ansi_rgb(rgb)),
            AnsiColor::Indexed(index) => self.resolve_indexed(usize::from(index)),
            AnsiColor::Named(named) => match named {
                NamedColor::Background => self.resolve_indexed(NamedColor::Background as usize),
                NamedColor::Foreground
                | NamedColor::BrightForeground
                | NamedColor::DimForeground
                | NamedColor::Cursor => self.resolve_indexed(NamedColor::Foreground as usize),
                NamedColor::DimBlack => Color::Ansi(0),
                NamedColor::DimRed => Color::Ansi(1),
                NamedColor::DimGreen => Color::Ansi(2),
                NamedColor::DimYellow => Color::Ansi(3),
                NamedColor::DimBlue => Color::Ansi(4),
                NamedColor::DimMagenta => Color::Ansi(5),
                NamedColor::DimCyan => Color::Ansi(6),
                NamedColor::DimWhite => Color::Ansi(7),
                ansi => self.resolve_indexed(ansi as usize),
            },
        }
    }

    fn resolve_indexed(&self, index: usize) -> Color {
        // A program may have redefined the slot (`OSC 4`, `OSC 10`, `OSC 11`).
        if let Some(rgb) = self.term.colors()[index] {
            return Color::Rgb(from_ansi_rgb(rgb));
        }
        match index {
            0..=15 => Color::Ansi(index as u8),
            16..=255 => Color::Rgb(indexed_color(index as u8)),
            index if index == NamedColor::Background as usize => Color::Background,
            _ => Color::Foreground,
        }
    }

    /// The colour a program is told a slot has when it asks.
    fn reported_color(&self, index: usize) -> Option<Rgb> {
        if index >= COLOR_SLOTS {
            return None;
        }
        if let Some(rgb) = self.term.colors()[index] {
            return Some(from_ansi_rgb(rgb));
        }
        match index {
            0..=15 => Some(self.palette.ansi[index]),
            16..=255 => Some(indexed_color(index as u8)),
            index if index == NamedColor::Foreground as usize => Some(self.palette.foreground),
            index if index == NamedColor::Background as usize => Some(self.palette.background),
            index if index == NamedColor::Cursor as usize => Some(self.palette.cursor),
            _ => None,
        }
    }

    fn take_effects(&mut self) -> Vec<Effect> {
        let events =
            std::mem::take(&mut *self.events.lock().unwrap_or_else(PoisonError::into_inner));
        events
            .into_iter()
            .filter_map(|event| match event {
                Event::PtyWrite(text) => Some(Effect::Reply(text.into_bytes())),
                Event::Title(title) => Some(Effect::Title(Some(title))),
                Event::ResetTitle => Some(Effect::Title(None)),
                Event::Bell => Some(Effect::Bell),
                Event::ClipboardStore(_, text) => Some(Effect::CopyToClipboard(text)),
                Event::ColorRequest(index, format) => self.reported_color(index).map(|rgb| {
                    Effect::Reply(
                        format(AnsiRgb {
                            r: rgb.r,
                            g: rgb.g,
                            b: rgb.b,
                        })
                        .into_bytes(),
                    )
                }),
                Event::TextAreaSizeRequest(format) => Some(Effect::Reply(
                    format(WindowSize {
                        num_lines: self.size.rows,
                        num_cols: self.size.cols,
                        cell_width: self.size.cell_width,
                        cell_height: self.size.cell_height,
                    })
                    .into_bytes(),
                )),
                // Letting a remote program read the local clipboard is a
                // deliberate non-feature; the rest needs no host action.
                Event::ClipboardLoad(..)
                | Event::MouseCursorDirty
                | Event::CursorBlinkingChange
                | Event::Wakeup
                | Event::Exit
                | Event::ChildExit(_) => None,
            })
            .collect()
    }
}

fn grid_side(side: Side) -> GridSide {
    match side {
        Side::Left => GridSide::Left,
        Side::Right => GridSide::Right,
    }
}

fn from_ansi_rgb(rgb: AnsiRgb) -> Rgb {
    Rgb::new(rgb.r, rgb.g, rgb.b)
}

/// The fixed xterm 256-colour table.
fn indexed_color(index: u8) -> Rgb {
    const ANSI: [Rgb; 16] = [
        Rgb::new(0x00, 0x00, 0x00),
        Rgb::new(0xcd, 0x00, 0x00),
        Rgb::new(0x00, 0xcd, 0x00),
        Rgb::new(0xcd, 0xcd, 0x00),
        Rgb::new(0x00, 0x00, 0xee),
        Rgb::new(0xcd, 0x00, 0xcd),
        Rgb::new(0x00, 0xcd, 0xcd),
        Rgb::new(0xe5, 0xe5, 0xe5),
        Rgb::new(0x7f, 0x7f, 0x7f),
        Rgb::new(0xff, 0x00, 0x00),
        Rgb::new(0x00, 0xff, 0x00),
        Rgb::new(0xff, 0xff, 0x00),
        Rgb::new(0x5c, 0x5c, 0xff),
        Rgb::new(0xff, 0x00, 0xff),
        Rgb::new(0x00, 0xff, 0xff),
        Rgb::new(0xff, 0xff, 0xff),
    ];

    match index {
        0..=15 => ANSI[usize::from(index)],
        16..=231 => {
            let index = index - 16;
            let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
            Rgb::new(level(index / 36), level(index / 6 % 6), level(index % 6))
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            Rgb::new(level, level, level)
        }
    }
}

mod snapshot;
#[cfg(test)]
mod tests;

mod modes;
pub use modes::Modes;

#[cfg(test)]
mod keyboard_tests;
