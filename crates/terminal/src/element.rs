//! Painting the grid.
//!
//! The grid is drawn in passes over the frame the emulator produced: cell
//! backgrounds, the selection, the cursor, then the text. Text is shaped in
//! runs with every glyph forced to the cell width, so the font's own spacing
//! can never drift a column out of the grid.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::{
    App, BorderStyle, Bounds, CursorStyle, ElementInputHandler, Entity, FocusHandle, Font,
    FontStyle, FontWeight, Hitbox, HitboxBehavior, Hsla, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, Point, Rgba, ScrollWheelEvent, SharedString, StrikethroughStyle,
    TextAlign, TextRun, UnderlineStyle, Window, canvas, component::ActiveTheme as _, fill, font,
    outline, point, prelude::*, px, size,
};
use nocterm_ui::{TerminalStyle, hsla};
use nocterm_vt::{
    Cell as GridCell, CellPoint, Color, CursorShape, Frame, Rgb, Side, Style, TermSize,
};

use crate::{Terminal, TerminalView};

/// Share of a cell's width a bar cursor takes.
const BAR_WIDTH: f32 = 0.12;
/// Share of a cell's height an underline cursor takes.
const UNDERLINE_HEIGHT: f32 = 0.1;
mod text;
use text::{Colors, paint_text};

/// Where the grid was last laid out, so the pointer can be mapped to cells.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// The top left corner of cell `(0, 0)`.
    pub(crate) origin: Point<Pixels>,
    pub(crate) cell_width: Pixels,
    pub(crate) line_height: Pixels,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

impl Geometry {
    /// The cell under `position`, and which half of it; positions outside the
    /// grid map to the nearest cell.
    pub(crate) fn cell_at(&self, position: Point<Pixels>) -> (CellPoint, Side) {
        let x = f32::from(position.x - self.origin.x) / f32::from(self.cell_width);
        let y = f32::from(position.y - self.origin.y) / f32::from(self.line_height);
        let last_col = f32::from(self.cols.saturating_sub(1));
        let last_row = f32::from(self.rows.saturating_sub(1));

        let col = x.floor().clamp(0.0, last_col);
        let row = y.floor().clamp(0.0, last_row);
        let side = if x < 0.0 {
            Side::Left
        } else if x >= f32::from(self.cols) || x - x.floor() >= 0.5 {
            Side::Right
        } else {
            Side::Left
        };
        (
            CellPoint {
                row: row as u16,
                col: col as u16,
            },
            side,
        )
    }

    /// The bounds of `cells` cells starting at `at`.
    pub(crate) fn cell_bounds(&self, at: CellPoint, cells: u16) -> Bounds<Pixels> {
        Bounds::new(
            point(
                self.origin.x + self.cell_width * f32::from(at.col),
                self.origin.y + self.line_height * f32::from(at.row),
            ),
            size(self.cell_width * f32::from(cells), self.line_height),
        )
    }
}

/// One frame of a terminal, as its view decided to draw it.
pub(crate) struct TerminalElement {
    pub(crate) view: Entity<TerminalView>,
    pub(crate) terminal: Entity<Terminal>,
    pub(crate) focus: FocusHandle,
    pub(crate) style: TerminalStyle,
    pub(crate) focused: bool,
    /// The blink phase: whether a blinking cursor is lit.
    pub(crate) cursor_lit: bool,
    /// Text an input method is composing, drawn at the cursor.
    pub(crate) marked_text: Option<SharedString>,
    pub(crate) frame: Rc<RefCell<Frame>>,
    pub(crate) frame_dirty: Rc<Cell<bool>>,
    pub(crate) frame_version: Rc<Cell<Option<(u64, u64)>>>,
    pub(crate) highlights: Rc<RefCell<crate::highlighting::Highlights>>,
    pub(crate) geometry: Rc<Cell<Option<Geometry>>>,
}

struct Layout {
    gutter_origin: Point<Pixels>,
    gutter_cells: usize,
    number_digits: usize,
    font: Font,
    geometry: Geometry,
    hitbox: Hitbox,
    colors: Colors,
}

impl TerminalElement {
    pub(crate) fn render(self) -> impl IntoElement {
        canvas(
            move |bounds, window, cx| self.prepaint(bounds, window, cx),
            |bounds, (element, layout), window, cx| element.paint(bounds, layout, window, cx),
        )
        .size_full()
    }

    /// Fits the grid to the bounds and takes the frame to draw.
    fn prepaint(self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> (Self, Layout) {
        let font = font(self.style.font_family.clone());
        let font_size = self.style.font_size;
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let cell_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * self.style.line_height).round();

        let padding = self.style.padding.to_pixels(window.rem_size());
        let terminal = self.terminal.read(cx);
        let alt_screen = terminal.emulator().modes().alt_screen;
        let number_digits = terminal.emulator().output_line_number().max(1).ilog10() as usize + 1;
        let number_digits = number_digits.max(3);
        let gutter_cells = gutter_cells(
            self.style.show_timestamps,
            self.style.show_line_numbers,
            number_digits,
            alt_screen,
        );
        let gutter = cell_width * gutter_cells as f32;
        let gutter_origin = point(bounds.origin.x + padding, bounds.origin.y + padding);
        let origin = point(gutter_origin.x + gutter, gutter_origin.y);
        let width = f32::from(bounds.size.width - padding * 2.0 - gutter).max(0.0);
        let height = f32::from(bounds.size.height - padding * 2.0).max(0.0);
        let fit = |space: f32, unit: Pixels| {
            (space / f32::from(unit)).floor().min(f32::from(u16::MAX)) as u16
        };
        let size = TermSize::new(
            fit(width, cell_width),
            fit(height, line_height),
            f32::from(cell_width).round() as u16,
            f32::from(line_height) as u16,
        );

        self.terminal
            .update(cx, |terminal, cx| terminal.resize(size, cx));
        self.refresh_frame(cx);

        self.highlights.borrow_mut().update(
            &self.frame.borrow(),
            self.terminal.read(cx).emulator().generation(),
            self.style.semantic_highlighting,
        );

        let geometry = Geometry {
            origin,
            cell_width,
            line_height,
            cols: size.cols,
            rows: size.rows,
        };
        self.geometry.set(Some(geometry));

        let layout = Layout {
            gutter_origin,
            gutter_cells,
            number_digits,
            font,
            geometry,
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            colors: Colors::new(&self.style),
        };
        (self, layout)
    }

    /// Cursor and composition changes can paint the existing emulator snapshot.
    pub(crate) fn refresh_frame(&self, cx: &App) -> bool {
        let terminal = self.terminal.read(cx);
        let emulator = terminal.emulator();
        let version = (emulator.generation(), terminal.display_revision());
        if !self.frame_dirty.get()
            && self.frame_version.get() == Some(version)
            && self.frame.borrow().size == emulator.size()
        {
            return false;
        }
        emulator.snapshot(&mut self.frame.borrow_mut());
        self.frame_version.set(Some(version));
        self.frame_dirty.set(false);
        true
    }

    fn paint(self, bounds: Bounds<Pixels>, layout: Layout, window: &mut Window, cx: &mut App) {
        let colors = &layout.colors;
        window.paint_quad(fill(bounds, colors.background));

        {
            let frame = self.frame.borrow();
            for row in 0..frame.size.rows {
                let cells = frame.row(row);
                paint_runs(window, &layout.geometry, row, cells, |cell| {
                    let (_, background) = colors.cell(cell);
                    (background != colors.background).then_some(background)
                });
                paint_runs(window, &layout.geometry, row, cells, |cell| {
                    cell.search_hit.then_some(cx.theme().warning.opacity(0.35))
                });
                paint_runs(window, &layout.geometry, row, cells, |cell| {
                    cell.selected.then_some(colors.selection)
                });
            }

            self.paint_gutter(&frame, &layout, window, cx);
            let block = self.paint_cursor(&frame, &layout, window);
            for row in 0..frame.size.rows {
                let block_col = block.filter(|at| at.row == row).map(|at| at.col);
                paint_text(
                    row,
                    &frame,
                    block_col,
                    &layout,
                    &self.highlights.borrow(),
                    self.style.font_size,
                    window,
                    cx,
                );
            }
            self.paint_marked_text(&frame, &layout, window, cx);
        }

        self.register_input(bounds, layout.hitbox, window, cx);
    }

    fn paint_gutter(&self, frame: &Frame, layout: &Layout, window: &mut Window, cx: &mut App) {
        if layout.gutter_cells == 0 {
            return;
        }
        let color = hsla(self.style.gutter_foreground);
        for (row, mark) in frame.lines.iter().enumerate() {
            let Some(mark) = mark.filter(|mark| !mark.continuation) else {
                continue;
            };
            let text = gutter_label(
                mark,
                self.style.show_timestamps,
                self.style.show_line_numbers,
                layout.number_digits,
            );
            let run = TextRun {
                len: text.len(),
                font: layout.font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let shaped = window.text_system().shape_line(
                text.into(),
                self.style.font_size,
                &[run],
                Some(layout.geometry.cell_width),
            );
            let origin = point(
                layout.gutter_origin.x,
                layout.gutter_origin.y + layout.geometry.line_height * row as f32,
            );
            if let Err(error) = shaped.paint(
                origin,
                layout.geometry.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                tracing::debug!(%error, "could not paint terminal line metadata");
            }
        }
    }

    /// Paints the cursor; returns the cell of a block cursor, whose
    /// character is then drawn in the background colour.
    fn paint_cursor(
        &self,
        frame: &Frame,
        layout: &Layout,
        window: &mut Window,
    ) -> Option<CellPoint> {
        let cursor = frame.cursor?;
        if self.marked_text.is_some() || (self.focused && cursor.blinking && !self.cursor_lit) {
            return None;
        }
        let geometry = &layout.geometry;
        let at = CellPoint {
            row: cursor.row,
            col: cursor.col,
        };
        let cell = geometry.cell_bounds(at, if cursor.wide { 2 } else { 1 });
        let color = layout.colors.cursor;
        let shape = if self.focused {
            cursor.shape
        } else {
            CursorShape::Hollow
        };

        match shape {
            CursorShape::Block => {
                window.paint_quad(fill(cell, color));
                return Some(at);
            }
            CursorShape::Bar => {
                let width = px((f32::from(geometry.cell_width) * BAR_WIDTH)
                    .round()
                    .max(1.0));
                window.paint_quad(fill(
                    Bounds::new(cell.origin, size(width, cell.size.height)),
                    color,
                ));
            }
            CursorShape::Underline => {
                let height = px((f32::from(geometry.line_height) * UNDERLINE_HEIGHT)
                    .round()
                    .max(1.0));
                let origin = point(cell.origin.x, cell.origin.y + cell.size.height - height);
                window.paint_quad(fill(
                    Bounds::new(origin, size(cell.size.width, height)),
                    color,
                ));
            }
            CursorShape::Hollow => {
                window.paint_quad(outline(cell, color, BorderStyle::Solid));
            }
        }
        None
    }

    /// Draws what an input method is composing over the cursor.
    fn paint_marked_text(&self, frame: &Frame, layout: &Layout, window: &mut Window, cx: &mut App) {
        let (Some(text), Some(cursor)) = (&self.marked_text, frame.cursor) else {
            return;
        };
        let colors = &layout.colors;
        let run = TextRun {
            len: text.len(),
            font: layout.font.clone(),
            color: colors.foreground,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(colors.foreground),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped =
            window
                .text_system()
                .shape_line(text.clone(), self.style.font_size, &[run], None);
        let origin = layout
            .geometry
            .cell_bounds(
                CellPoint {
                    row: cursor.row,
                    col: cursor.col,
                },
                1,
            )
            .origin;
        window.paint_quad(fill(
            Bounds::new(origin, size(shaped.width, layout.geometry.line_height)),
            colors.background,
        ));
        if let Err(error) = shaped.paint(
            origin,
            layout.geometry.line_height,
            TextAlign::Left,
            None,
            window,
            cx,
        ) {
            tracing::debug!(%error, "could not paint composed text");
        }
    }

    /// Takes keyboard text and pointer input for the frame just painted.
    fn register_input(
        self,
        bounds: Bounds<Pixels>,
        hitbox: Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.focus,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        window.set_cursor_style(CursorStyle::IBeam, &hitbox);

        let grid_left = self.geometry.get().map(|geometry| geometry.origin.x);
        let view = self.view.clone();
        let down_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase.bubble()
                && down_hitbox.is_hovered(window)
                && grid_left.is_none_or(|left| event.position.x >= left)
            {
                view.update(cx, |view, cx| view.mouse_down(event, window, cx));
            }
        });

        let view = self.view.clone();
        let move_hitbox = hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase.bubble() {
                let hovered = move_hitbox.is_hovered(window);
                view.update(cx, |view, cx| view.mouse_move(event, hovered, cx));
            }
        });

        let view = self.view.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
            if phase.bubble() {
                view.update(cx, |view, cx| view.mouse_up(event, cx));
            }
        });

        let view = self.view;
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase.bubble() && hitbox.should_handle_scroll(window) {
                view.update(cx, |view, cx| view.scroll_wheel(event, cx));
            }
        });
    }
}

fn gutter_cells(timestamps: bool, numbers: bool, number_digits: usize, alt_screen: bool) -> usize {
    if alt_screen || (!timestamps && !numbers) {
        return 0;
    }
    usize::from(timestamps) * 9 + if numbers { number_digits + 1 } else { 0 } + 1
}

fn gutter_label(
    mark: nocterm_vt::LineMetadata,
    timestamps: bool,
    numbers: bool,
    digits: usize,
) -> String {
    let mut label = String::new();
    if timestamps {
        let seconds = (mark.timestamp_ms / 1000) % 86_400;
        label.push_str(&format!(
            "{:02}:{:02}:{:02} ",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        ));
    }
    if numbers {
        label.push_str(&format!("{:>digits$}", mark.number));
    }
    label
}

/// Fills runs of adjacent cells of one row that `color_of` gives a colour.
fn paint_runs(
    window: &mut Window,
    geometry: &Geometry,
    row: u16,
    cells: &[GridCell],
    color_of: impl Fn(&GridCell) -> Option<Hsla>,
) {
    let mut run: Option<(u16, u16, Hsla)> = None;
    let flush = |run: Option<(u16, u16, Hsla)>, window: &mut Window| {
        if let Some((start, end, color)) = run {
            let bounds = geometry.cell_bounds(CellPoint { row, col: start }, end - start);
            window.paint_quad(fill(bounds, color));
        }
    };

    for (col, cell) in (0u16..).zip(cells) {
        let color = color_of(cell);
        match (&mut run, color) {
            (Some((_, end, current)), Some(color)) if *current == color && *end == col => {
                *end = col + 1
            }
            (_, color) => {
                flush(run.take(), window);
                run = color.map(|color| (col, col + 1, color));
            }
        }
    }
    flush(run, window);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> Geometry {
        Geometry {
            origin: point(px(10.0), px(20.0)),
            cell_width: px(8.0),
            line_height: px(16.0),
            cols: 80,
            rows: 24,
        }
    }

    #[test]
    fn maps_positions_to_cells_and_halves() {
        let geometry = geometry();

        assert_eq!(
            geometry.cell_at(point(px(10.0), px(20.0))),
            (CellPoint { row: 0, col: 0 }, Side::Left)
        );
        assert_eq!(
            geometry.cell_at(point(
                px(10.0 + 8.0 * 3.0 + 5.0),
                px(20.0 + 16.0 * 2.0 + 1.0)
            )),
            (CellPoint { row: 2, col: 3 }, Side::Right)
        );
    }

    #[test]
    fn positions_outside_the_grid_clamp_to_its_edges() {
        let geometry = geometry();

        assert_eq!(
            geometry.cell_at(point(px(0.0), px(0.0))),
            (CellPoint { row: 0, col: 0 }, Side::Left)
        );
        assert_eq!(
            geometry.cell_at(point(px(5000.0), px(5000.0))),
            (CellPoint { row: 23, col: 79 }, Side::Right)
        );
    }

    #[test]
    fn cell_bounds_cover_whole_cells() {
        let bounds = geometry().cell_bounds(CellPoint { row: 1, col: 2 }, 2);

        assert_eq!(bounds.origin, point(px(26.0), px(36.0)));
        assert_eq!(bounds.size, size(px(16.0), px(16.0)));
    }
}

#[cfg(test)]
mod gutter_tests {
    use super::*;
    #[test]
    fn alternate_screen_and_disabled_gutter_preserve_full_pty_width() {
        for (timestamps, numbers) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(gutter_cells(timestamps, numbers, 6, true), 0);
        }
        assert_eq!(gutter_cells(false, false, 6, false), 0);
        assert_eq!(gutter_cells(true, false, 6, false), 10);
        assert_eq!(gutter_cells(false, true, 6, false), 8);
        assert_eq!(gutter_cells(true, true, 6, false), 17);
        let width = px(8.0);
        let gutter = width * gutter_cells(true, true, 6, false) as f32;
        let geometry = Geometry {
            origin: point(px(10.0) + gutter, px(20.0)),
            cell_width: width,
            line_height: px(16.0),
            cols: 80,
            rows: 24,
        };
        assert_eq!(
            geometry.cell_at(geometry.origin),
            (CellPoint { row: 0, col: 0 }, Side::Left)
        );
        assert_eq!(
            geometry
                .cell_bounds(CellPoint { row: 0, col: 1 }, 1)
                .origin
                .x,
            geometry.origin.x + width
        );
    }
    #[test]
    fn utc_time_and_line_number_are_presentation_only() {
        let mark = nocterm_vt::LineMetadata {
            number: 42,
            timestamp_ms: ((25 * 3600 + 2 * 60 + 3) * 1000),
            continuation: false,
        };
        assert_eq!(gutter_label(mark, true, true, 3), "01:02:03  42");
        assert_eq!(gutter_label(mark, false, true, 3), " 42");
    }
}
