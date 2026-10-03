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

/// The switches a running program has flipped that change how input is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Arrow keys use the application (`SS3`) encoding.
    pub app_cursor: bool,
    /// Pastes are wrapped so the program can tell them from typing.
    pub bracketed_paste: bool,
    /// The alternate screen is showing (a full-screen program is running).
    pub alt_screen: bool,
    /// The wheel moves the cursor instead of the scrollback on the alternate screen.
    pub alternate_scroll: bool,
    /// The program wants to be told when the terminal gains or loses focus.
    pub focus_events: bool,
    /// Button presses and releases are reported.
    pub mouse_clicks: bool,
    /// Pointer movement with a button held is reported.
    pub mouse_drag: bool,
    /// All pointer movement is reported.
    pub mouse_motion: bool,
    /// Mouse reports use the `SGR` encoding.
    pub mouse_sgr: bool,
    /// Mouse reports use the UTF-8 extension of the legacy encoding.
    pub mouse_utf8: bool,
}

impl Modes {
    /// Whether the program takes any mouse input at all.
    pub fn reports_mouse(&self) -> bool {
        self.mouse_clicks || self.mouse_drag || self.mouse_motion
    }
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
        self.generation = self.generation.wrapping_add(1);
        self.term.set_options(options.to_config());
    }

    /// Tells the emulator which colours the host draws with.
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    pub fn modes(&self) -> Modes {
        let mode = *self.term.mode();
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
            mouse_clicks: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            mouse_utf8: mode.contains(TermMode::UTF8_MOUSE),
        }
    }

    /// Moves the viewport through the scrollback.
    pub fn scroll(&mut self, scroll: Scroll) {
        self.term.scroll_display(match scroll {
            Scroll::Lines(lines) => GridScroll::Delta(lines),
            Scroll::PageUp => GridScroll::PageUp,
            Scroll::PageDown => GridScroll::PageDown,
            Scroll::Top => GridScroll::Top,
            Scroll::Bottom => GridScroll::Bottom,
        });
    }

    /// Lines the viewport is scrolled back by; 0 shows the live screen.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
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

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
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
        crate::SearchScan::new(self, query, direction, anchor)
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

    /// Writes the current viewport into `frame`.
    pub fn snapshot(&self, frame: &mut Frame) {
        let content = self.term.renderable_content();
        let cols = usize::from(self.size.cols);
        let rows = usize::from(self.size.rows);
        let display_offset = content.display_offset as i32;

        frame.size = self.size;
        frame.display_offset = content.display_offset;
        frame.history_size = self.term.grid().history_size();
        frame.alt_screen = self.term.mode().contains(TermMode::ALT_SCREEN);
        frame.lines.clear();
        frame.lines.resize(rows, None);
        if !frame.alt_screen {
            let grid = self.term.grid();
            for (row, metadata) in frame.lines.iter_mut().enumerate() {
                let line = alacritty_terminal::index::Line(row as i32 - display_offset);
                if let Some(mark) = grid[line][..].iter().find_map(|cell| cell.logical_line()) {
                    let continuation = line > grid.topmost_line()
                        && grid[line - 1i32][alacritty_terminal::index::Column(cols - 1)]
                            .flags
                            .contains(Flags::WRAPLINE);
                    *metadata = Some(LineMetadata {
                        number: mark.id,
                        timestamp_ms: mark.timestamp_ms,
                        continuation,
                    });
                }
            }
        }
        frame.combining.clear();
        frame.cells.clear();
        frame.cells.resize(cols * rows, Cell::default());

        for indexed in content.display_iter {
            let row = indexed.point.line.0 + display_offset;
            let col = indexed.point.column.0;
            if row < 0 || row as usize >= rows || col >= cols {
                continue;
            }
            let ix = row as usize * cols + col;
            let cell = indexed.cell;
            let flags = cell.flags;

            frame.cells[ix] = Cell {
                ch: cell.c,
                fg: self.resolve(cell.fg),
                bg: self.resolve(cell.bg),
                style: Style {
                    bold: flags.contains(Flags::BOLD),
                    italic: flags.contains(Flags::ITALIC),
                    dim: flags.contains(Flags::DIM),
                    underline: flags.intersects(Flags::ALL_UNDERLINES),
                    strikeout: flags.contains(Flags::STRIKEOUT),
                    inverse: flags.contains(Flags::INVERSE),
                    hidden: flags.contains(Flags::HIDDEN),
                },
                wide: flags.contains(Flags::WIDE_CHAR),
                spacer: flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER),
                selected: content
                    .selection
                    .is_some_and(|selection| selection.contains(indexed.point)),
                search_hit: self.search_match.is_some_and(|(generation, range)| {
                    if generation != self.generation {
                        return false;
                    }
                    let point = (indexed.point.line.0, col);
                    let end_cell = &self.term.grid()
                        [Point::new(Line(range.end.line), Column(usize::from(range.end.column)))];
                    let end_col = usize::from(range.end.column)
                        + usize::from(end_cell.flags.contains(Flags::WIDE_CHAR));
                    point >= (range.start.line, usize::from(range.start.column))
                        && point <= (range.end.line, end_col)
                }),
            };
            if let Some(combining) = cell.zerowidth().filter(|chars| !chars.is_empty()) {
                frame.combining.push((ix, combining.iter().collect()));
            }
        }

        let cursor_row = content.cursor.point.line.0 + display_offset;
        let cursor_col = content.cursor.point.column.0;
        let shape = match content.cursor.shape {
            AnsiCursorShape::Hidden => None,
            AnsiCursorShape::Block => Some(CursorShape::Block),
            AnsiCursorShape::Beam => Some(CursorShape::Bar),
            AnsiCursorShape::Underline => Some(CursorShape::Underline),
            AnsiCursorShape::HollowBlock => Some(CursorShape::Hollow),
        };
        frame.cursor = shape
            .filter(|_| cursor_row >= 0 && (cursor_row as usize) < rows && cursor_col < cols)
            .map(|shape| Cursor {
                row: cursor_row as u16,
                col: cursor_col as u16,
                shape,
                blinking: self.term.cursor_style().blinking,
                wide: frame.cells[cursor_row as usize * cols + cursor_col].wide,
            });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn emulator(cols: u16, rows: u16) -> Emulator {
        Emulator::new(TermSize::new(cols, rows, 8, 16), EmulatorOptions::default())
    }

    fn frame(emulator: &Emulator) -> Frame {
        let mut frame = Frame::default();
        emulator.snapshot(&mut frame);
        frame
    }

    #[test]
    fn prints_text_and_moves_the_cursor() {
        let mut emulator = emulator(20, 4);
        emulator.advance(b"hello\r\nworld");

        let frame = frame(&emulator);

        assert_eq!(frame.row_text(0), "hello");
        assert_eq!(frame.row_text(1), "world");
        let cursor = frame.cursor.unwrap();
        assert_eq!((cursor.row, cursor.col), (1, 5));
    }

    #[test]
    fn maps_sgr_attributes_to_cells() {
        let mut emulator = emulator(20, 2);
        emulator.advance(b"\x1b[1;31;44mA\x1b[0m\x1b[38;2;1;2;3mB\x1b[38;5;196mC\x1b[7mD");

        let frame = frame(&emulator);
        let row = frame.row(0);

        assert_eq!(row[0].fg, Color::Ansi(1));
        assert_eq!(row[0].bg, Color::Ansi(4));
        assert!(row[0].style.bold);
        assert_eq!(row[1].fg, Color::Rgb(Rgb::new(1, 2, 3)));
        assert_eq!(row[1].bg, Color::Background);
        assert_eq!(row[2].fg, Color::Rgb(Rgb::new(0xff, 0, 0)));
        assert!(row[3].style.inverse);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        let mut emulator = emulator(10, 2);
        emulator.advance("日x".as_bytes());

        let frame = frame(&emulator);
        let row = frame.row(0);

        assert!(row[0].wide && !row[0].spacer);
        assert!(row[1].spacer);
        assert_eq!(row[2].ch, 'x');
        assert_eq!(frame.row_text(0), "日x");
    }

    #[test]
    fn combining_characters_are_reported_beside_their_cell() {
        let mut emulator = emulator(10, 2);
        emulator.advance("e\u{301}".as_bytes());

        let frame = frame(&emulator);

        assert_eq!(frame.combining, [(0, "\u{301}".to_owned())]);
    }

    #[test]
    fn combining_flood_is_bounded_in_grid_snapshot_and_selection() {
        let mut emulator = emulator(2, 1);
        emulator.advance(b"a");
        let marks = "\u{301}".repeat(100_000);
        for chunk in marks.as_bytes().chunks(997) {
            emulator.advance(chunk);
        }
        assert_eq!(
            emulator.term.grid()[Point::new(Line(0), Column(0))]
                .zerowidth()
                .unwrap()
                .len(),
            64
        );
        emulator.advance(b"X");
        let frame = frame(&emulator);
        assert_eq!(frame.combining, [(0, "\u{301}".repeat(64))]);
        assert_eq!(frame.row_text(0), "aX");
        emulator.select_all();
        assert_eq!(
            emulator.selection_text().unwrap(),
            format!("a{}X", "\u{301}".repeat(64))
        );
    }

    #[test]
    fn ordinary_combining_clusters_preserve_wide_and_hidden_attributes() {
        let mut emulator = emulator(10, 1);
        emulator.advance("e\u{301}\u{302}日\u{301}\x1b[8mA\u{301}".as_bytes());
        let frame = frame(&emulator);
        assert_eq!(
            frame.combining,
            [
                (0, "\u{301}\u{302}".into()),
                (1, "\u{301}".into()),
                (3, "\u{301}".into())
            ]
        );
        assert!(frame.cells[1].wide);
        assert!(frame.cells[3].style.hidden);
    }

    #[test]
    fn oversized_osc_metadata_preserves_title_and_ends_hyperlinks() {
        let mut emulator = emulator(10, 1);
        assert_eq!(
            emulator.advance(b"\x1b]0;old\x07"),
            [Effect::Title(Some("old".into()))]
        );
        assert!(
            emulator
                .advance(format!("\x1b]0;{}\x07", "x".repeat(4097)).as_bytes())
                .is_empty()
        );
        emulator.advance(b"\x1b[22;0t\x1b]0;new\x07");
        assert_eq!(
            emulator.advance(b"\x1b[23;0t"),
            [Effect::Title(Some("old".into()))]
        );
        emulator.advance(b"\x1b]8;id=normal;https://example.org\x07A");
        emulator.advance(format!("\x1b]8;;{}\x07B", "x".repeat(4097)).as_bytes());
        emulator.advance(b"\x1b]8;;https://example.org\x07C");
        emulator
            .advance(format!("\x1b]8;id={};https://example.org\x07D", "x".repeat(1025)).as_bytes());
        for column in [0, 2] {
            assert!(
                emulator.term.grid()[Point::new(Line(0), Column(column))]
                    .hyperlink()
                    .is_some()
            );
        }
        for column in [1, 3] {
            assert!(
                emulator.term.grid()[Point::new(Line(0), Column(column))]
                    .hyperlink()
                    .is_none()
            );
        }
        assert_eq!(frame(&emulator).row_text(0), "ABCD");
    }

    #[test]
    fn reports_titles_bells_and_query_replies() {
        let mut emulator = emulator(20, 4);

        let effects = emulator.advance(b"\x1b]0;build\x07\x07\x1b[6n");

        assert_eq!(
            effects,
            [
                Effect::Title(Some("build".to_owned())),
                Effect::Bell,
                Effect::Reply(b"\x1b[1;1R".to_vec()),
            ]
        );
    }

    #[test]
    fn answers_colour_queries_from_the_host_palette() {
        let mut emulator = emulator(20, 4);
        emulator.set_palette(Palette {
            background: Rgb::new(0x10, 0x20, 0x30),
            ..Palette::default()
        });

        let effects = emulator.advance(b"\x1b]11;?\x07");

        assert_eq!(
            effects,
            [Effect::Reply(b"\x1b]11;rgb:1010/2020/3030\x07".to_vec())]
        );
    }

    #[test]
    fn tracks_the_modes_that_change_input_encoding() {
        let mut emulator = emulator(20, 4);
        assert_eq!(
            emulator.modes(),
            Modes {
                alternate_scroll: true,
                ..Modes::default()
            }
        );

        emulator.advance(b"\x1b[?1h\x1b[?2004h\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1004h");

        let modes = emulator.modes();
        assert!(modes.app_cursor && modes.bracketed_paste && modes.alt_screen);
        assert!(modes.mouse_clicks && modes.mouse_sgr && modes.focus_events);
        assert!(modes.reports_mouse());
    }

    #[test]
    fn scrolls_through_history() {
        let mut emulator = emulator(10, 3);
        for line in 0..10 {
            emulator.advance(format!("line{line}\r\n").as_bytes());
        }
        assert_eq!(frame(&emulator).row_text(0), "line8");

        emulator.scroll(Scroll::Lines(2));
        let scrolled = frame(&emulator);
        assert_eq!(scrolled.display_offset, 2);
        assert_eq!(scrolled.row_text(0), "line6");
        assert_eq!(scrolled.cursor, None, "the cursor scrolled out of view");

        emulator.scroll(Scroll::Bottom);
        assert_eq!(emulator.display_offset(), 0);
        assert_eq!(frame(&emulator).history_size, 8);
    }

    #[test]
    fn selects_cells_words_and_lines() {
        let mut emulator = emulator(20, 3);
        emulator.advance(b"alpha beta\r\ngamma");

        emulator.start_selection(
            SelectionKind::Cells,
            CellPoint { row: 0, col: 1 },
            Side::Left,
        );
        emulator.update_selection(CellPoint { row: 0, col: 3 }, Side::Right);
        assert_eq!(emulator.selection_text().as_deref(), Some("lph"));
        let selected: Vec<bool> = frame(&emulator).row(0)[..5]
            .iter()
            .map(|c| c.selected)
            .collect();
        assert_eq!(selected, [false, true, true, true, false]);

        emulator.start_selection(
            SelectionKind::Words,
            CellPoint { row: 0, col: 7 },
            Side::Left,
        );
        assert_eq!(emulator.selection_text().as_deref(), Some("beta"));

        emulator.start_selection(
            SelectionKind::Lines,
            CellPoint { row: 1, col: 2 },
            Side::Left,
        );
        assert_eq!(emulator.selection_text().as_deref(), Some("gamma\n"));

        emulator.clear_selection();
        assert_eq!(emulator.selection_text(), None);
    }

    #[test]
    fn resizing_reflows_and_clamps_to_a_minimum() {
        let mut emulator = emulator(10, 3);
        emulator.advance(b"0123456789abc");

        emulator.resize(TermSize::new(0, 0, 8, 16));
        assert_eq!((emulator.size().cols, emulator.size().rows), (2, 1));

        emulator.resize(TermSize::new(20, 3, 8, 16));
        assert_eq!(frame(&emulator).row_text(0), "0123456789abc");
    }

    #[test]
    fn default_cursor_follows_the_options() {
        let mut emulator = Emulator::new(
            TermSize::default(),
            EmulatorOptions {
                cursor_shape: CursorShape::Bar,
                cursor_blink: false,
                ..EmulatorOptions::default()
            },
        );
        let cursor = frame(&emulator).cursor.unwrap();
        assert_eq!((cursor.shape, cursor.blinking), (CursorShape::Bar, false));

        emulator.advance(b"\x1b[?25l");
        assert_eq!(frame(&emulator).cursor, None);
    }

    #[test]
    fn indexed_colours_follow_the_xterm_table() {
        assert_eq!(indexed_color(16), Rgb::new(0, 0, 0));
        assert_eq!(indexed_color(21), Rgb::new(0, 0, 255));
        assert_eq!(indexed_color(231), Rgb::new(255, 255, 255));
        assert_eq!(indexed_color(232), Rgb::new(8, 8, 8));
        assert_eq!(indexed_color(255), Rgb::new(238, 238, 238));
    }
}

#[cfg(test)]
mod line_metadata_tests {
    use super::*;
    fn emulator(cols: u16, rows: u16) -> Emulator {
        Emulator::new(TermSize::new(cols, rows, 0, 0), EmulatorOptions::default())
    }
    fn frame(emulator: &Emulator) -> Frame {
        let mut frame = Frame::default();
        emulator.snapshot(&mut frame);
        frame
    }

    #[test]
    fn wrapped_rows_keep_first_observation_across_fragmented_output() {
        let mut terminal = emulator(4, 4);
        terminal.advance_at(b"ab", 1000);
        terminal.advance_at(b"cdefgh\r\nx", 9000);
        let frame = frame(&terminal);
        assert_eq!(
            frame.lines[0],
            Some(LineMetadata {
                number: 1,
                timestamp_ms: 1000,
                continuation: false
            })
        );
        assert_eq!(
            frame.lines[1],
            Some(LineMetadata {
                number: 1,
                timestamp_ms: 1000,
                continuation: true
            })
        );
        assert_eq!(
            frame.lines[2],
            Some(LineMetadata {
                number: 2,
                timestamp_ms: 9000,
                continuation: false
            })
        );
    }
    #[test]
    fn reflow_and_blank_output_keep_identity() {
        let mut terminal = emulator(8, 5);
        terminal.advance_at(b"abcdefghijk\r\n\r\nx", 1000);
        let before = frame(&terminal);
        assert_eq!(before.lines[2].unwrap().number, 2);
        assert_eq!(before.lines[3].unwrap().number, 3);
        terminal.resize(TermSize::new(4, 5, 0, 0));
        let narrower = frame(&terminal);
        assert!(
            narrower
                .lines
                .iter()
                .flatten()
                .filter(|line| line.number == 1)
                .all(|line| line.timestamp_ms == 1000)
        );
        assert!(
            narrower.lines.iter().flatten().any(|line| line.number == 2),
            "explicit blank LF survives reflow"
        );
        assert!(narrower.lines.iter().flatten().any(|line| line.number == 3));
        terminal.resize(TermSize::new(16, 5, 0, 0));
        let wider = frame(&terminal);
        assert!(wider.lines.iter().flatten().any(|line| line.number == 1));
        assert!(wider.lines.iter().flatten().any(|line| line.number == 2));
        assert!(wider.lines.iter().flatten().any(|line| line.number == 3));
    }
    #[test]
    fn redraw_and_character_edits_preserve_line_but_screen_clear_is_fresh() {
        let mut terminal = emulator(20, 4);
        terminal.advance_at(b"old", 1000);
        for edits in [
            b"\r\x1b[2Knew".as_slice(),
            b"\r\x1b[3Xnew",
            b"\r\x1b[3Pnew",
            b"\r\x1b[2@new",
        ] {
            terminal.advance_at(edits, 2000);
            assert_eq!(frame(&terminal).lines[0].unwrap().number, 1);
            assert_eq!(frame(&terminal).lines[0].unwrap().timestamp_ms, 1000);
        }
        terminal.advance_at(b"\x1b[2J\x1b[Hfresh", 3000);
        assert_eq!(frame(&terminal).lines[0].unwrap().number, 2);
        assert_eq!(frame(&terminal).lines[0].unwrap().timestamp_ms, 3000);
    }
    #[test]
    fn trimming_history_never_renumbers_remaining_lines() {
        let options = EmulatorOptions {
            scrollback_lines: 2,
            ..EmulatorOptions::default()
        };
        let mut terminal = Emulator::new(TermSize::new(10, 2, 0, 0), options);
        for number in 1..=12 {
            terminal.advance_at(format!("{number}\r\n").as_bytes(), number * 1000);
        }
        assert_eq!(terminal.output_line_number(), 12);
        terminal.scroll(Scroll::Top);
        let top = frame(&terminal);
        assert!(top.lines.iter().flatten().all(|line| line.number >= 9));
        assert_eq!(top.history_size, 2);
    }
    #[test]
    fn alternate_screen_hides_metadata_and_does_not_consume_normal_numbers() {
        let mut terminal = emulator(8, 3);
        terminal.advance_at(b"normal", 1000);
        terminal.advance_at(b"\x1b[?1049hother\r\nrows", 2000);
        let alt = frame(&terminal);
        assert!(alt.alt_screen);
        assert!(alt.lines.iter().all(Option::is_none));
        assert_eq!(terminal.output_line_number(), 1);
        terminal.advance_at(b"\x1b[?1049l\r\nnext", 3000);
        let normal = frame(&terminal);
        assert!(!normal.alt_screen);
        assert_eq!(normal.lines[0].unwrap().number, 1);
        assert_eq!(normal.lines[1].unwrap().number, 2);
        terminal.start_selection(
            SelectionKind::Lines,
            CellPoint { row: 1, col: 0 },
            Side::Left,
        );
        assert!(!terminal.selection_text().unwrap().contains("3000"));
    }
}

#[cfg(test)]
mod lineage_storage_tests {
    use super::*;
    #[test]
    fn ordinary_characters_share_metadata_and_attribute_reset_keeps_it() {
        let mut terminal = Emulator::new(TermSize::new(10, 2, 0, 0), EmulatorOptions::default());
        terminal.advance_at(b"abc", 1000);
        let row = &terminal.term.grid()[alacritty_terminal::index::Line(0)];
        assert!(Arc::ptr_eq(
            row[Column(0)].extra.as_ref().unwrap(),
            row[Column(1)].extra.as_ref().unwrap()
        ));
        let mut cell = row[Column(0)].clone();
        let lineage = cell.logical_line();
        cell.set_underline_color(Some(AnsiColor::Named(NamedColor::Red)));
        cell.set_underline_color(None);
        cell.set_hyperlink(None);
        assert_eq!(cell.logical_line(), lineage);
        assert!(std::mem::size_of::<alacritty_terminal::term::cell::Cell>() <= 24);
    }
}

#[cfg(test)]
mod osc_limit_tests {
    use super::*;
    fn emulator() -> Emulator {
        Emulator::new(TermSize::new(80, 3, 0, 0), EmulatorOptions::default())
    }
    #[test]
    fn oversize_title_and_clipboard_never_dispatch_and_each_terminator_recovers() {
        for prefix in [b"\x1b]0;".as_slice(), b"\x1b]52;c;".as_slice()] {
            for terminator in [0x07, 0x18, 0x1a, 0x1b] {
                let mut terminal = emulator();
                assert!(terminal.advance(prefix).is_empty());
                for _ in 0..256 {
                    assert!(terminal.advance(&[b'A'; 8192]).is_empty());
                }
                let mut recovery = vec![terminator];
                if terminator == 0x1b {
                    recovery.push(b'\\');
                }
                recovery.extend(b"restored\x1b]0;new title\x07");
                let effects = terminal.advance(&recovery);
                assert!(
                    matches!(effects.as_slice(), [Effect::Title(title)] if title.as_deref() == Some("new title"))
                );
                let mut frame = Frame::default();
                terminal.snapshot(&mut frame);
                assert_eq!(frame.row_text(0), "restored");
            }
        }
    }
    #[test]
    fn valid_title_at_the_metadata_limit_is_not_truncated() {
        let mut terminal = emulator();
        terminal.advance(b"\x1b]0;");
        let title = vec![b'x'; 4096];
        assert!(terminal.advance(&title).is_empty());
        let effects = terminal.advance(b"\x07");
        assert!(
            matches!(effects.as_slice(), [Effect::Title(Some(value))] if value.len() == title.len())
        );
    }
    #[test]
    fn each_escape_follower_preserves_upstream_semantics_below_the_limit() {
        for byte in 0..=255 {
            let mut guarded = emulator();
            let mut native = emulator();
            let bytes = [
                b"before\x1b".as_slice(),
                &[byte],
                b"]0;x\x07after\x1b]52;c;b2s=\x1b\\",
            ]
            .concat();
            let mut guarded_effects = Vec::new();
            let mut native_effects = Vec::new();
            for byte in bytes {
                guarded_effects.extend(guarded.advance_at(&[byte], 0));
                native.term.set_output_timestamp_ms(0);
                native.parser.advance(&mut native.term, &[byte]);
                native_effects.extend(native.take_effects());
            }
            assert_eq!(guarded_effects, native_effects, "escape follower {byte:#x}");
            let (mut left, mut right) = (Frame::default(), Frame::default());
            guarded.snapshot(&mut left);
            native.snapshot(&mut right);
            assert_eq!(left, right, "escape follower {byte:#x}");
        }
    }
    #[test]
    fn ordinary_fragmented_osc_and_utf8_preserve_native_effects_and_synchronized_output() {
        let bytes = "\x1b[?2026hλ\x1b]0;title λ\x1b\\\x1b]52;c;b2s=\x07\x1b[?2026l".as_bytes();
        let mut terminal = emulator();
        let effects: Vec<_> = bytes
            .iter()
            .flat_map(|byte| terminal.advance(&[*byte]))
            .collect();
        assert!(effects.iter().any(
            |effect| matches!(effect, Effect::Title(title) if title.as_deref() == Some("title λ"))
        ));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::CopyToClipboard(text) if text == "ok"))
        );
        let mut frame = Frame::default();
        terminal.snapshot(&mut frame);
        assert_eq!(frame.row_text(0), "λ");
        assert!(terminal.sync_deadline().is_none());
    }
}
