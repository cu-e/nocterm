use super::*;

const DIM_OPACITY: f32 = 0.66;

/// Draws the characters of one row.
///
/// A row is cut into segments after each wide character, because a forced
/// glyph width can only describe single-width cells.
pub(super) fn paint_text(
    row: u16,
    frame: &Frame,
    block_cursor_col: Option<u16>,
    layout: &Layout,
    highlights: &crate::highlighting::Highlights,
    font_size: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    let geometry = &layout.geometry;
    let mut segment = Segment::default();
    let start = usize::from(row) * usize::from(frame.size.cols);
    let mut combining = frame.combining
        [frame.combining.partition_point(|(index, _)| *index < start)..]
        .iter()
        .peekable();

    for (col, cell) in (0u16..).zip(frame.row(row)) {
        if cell.spacer {
            continue;
        }
        if segment.text.is_empty() {
            segment.start = col;
        }
        let foreground = layout.colors.text_foreground(
            cell,
            highlights.role_at(start + usize::from(col), cell),
            block_cursor_col == Some(col),
        );
        let marks = combining.next_if(|(index, _)| *index == start + usize::from(col));
        segment.push_cell(
            cell,
            marks.map(|(_, text)| text.as_str()).unwrap_or(""),
            foreground,
            &layout.font,
        );

        if cell.wide {
            segment.paint(row, geometry, font_size, window, cx);
        }
    }
    segment.paint(row, geometry, font_size, window, cx);
}

#[derive(Default)]
struct Segment {
    start: u16,
    text: String,
    runs: Vec<TextRun>,
    /// Holds anything that leaves a mark; blank segments are not shaped.
    visible: bool,
}

impl Segment {
    fn push_cell(&mut self, cell: &GridCell, marks: &str, foreground: Hsla, font: &Font) {
        let hidden = cell.style.hidden || cell.ch.is_control();
        let ch = if hidden { ' ' } else { cell.ch };
        self.push(ch, text_run(ch, cell.style, foreground, font));
        if !hidden {
            for mark in marks.chars() {
                self.push(mark, text_run(mark, cell.style, foreground, font));
            }
        }
    }

    fn push(&mut self, ch: char, run: TextRun) {
        self.visible |= ch != ' ' || run.underline.is_some() || run.strikethrough.is_some();
        self.text.push(ch);
        match self.runs.last_mut() {
            Some(last)
                if last.font == run.font
                    && last.color == run.color
                    && last.underline == run.underline
                    && last.strikethrough == run.strikethrough =>
            {
                last.len += run.len;
            }
            _ => self.runs.push(run),
        }
    }

    fn paint(
        &mut self,
        row: u16,
        geometry: &Geometry,
        font_size: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.visible {
            let text = SharedString::from(std::mem::take(&mut self.text));
            let shaped = window.text_system().shape_line(
                text,
                font_size,
                &self.runs,
                Some(geometry.cell_width),
            );
            let origin = geometry
                .cell_bounds(
                    CellPoint {
                        row,
                        col: self.start,
                    },
                    1,
                )
                .origin;
            if let Err(error) = shaped.paint(
                origin,
                geometry.line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                tracing::debug!(%error, "could not paint a terminal row");
            }
        }
        self.text.clear();
        self.runs.clear();
        self.visible = false;
    }
}

fn text_run(ch: char, style: Style, color: Hsla, base: &Font) -> TextRun {
    let decoration = |on: bool| on.then_some((px(1.0), Some(color)));
    TextRun {
        len: ch.len_utf8(),
        font: Font {
            weight: if style.bold {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            },
            style: if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            ..base.clone()
        },
        color,
        background_color: None,
        underline: decoration(style.underline).map(|(thickness, color)| UnderlineStyle {
            thickness,
            color,
            wavy: false,
        }),
        strikethrough: decoration(style.strikeout)
            .map(|(thickness, color)| StrikethroughStyle { thickness, color }),
    }
}

/// The style's colours, ready to paint.
pub(super) struct Colors {
    pub(super) foreground: Hsla,
    pub(super) background: Hsla,
    pub(super) cursor: Hsla,
    pub(super) selection: Hsla,
    ansi: [Hsla; 16],
    semantic: [Hsla; crate::highlighting::Role::COUNT],
}

impl Colors {
    pub(super) fn new(style: &TerminalStyle) -> Self {
        Self {
            foreground: hsla(style.foreground),
            background: hsla(style.background),
            cursor: hsla(style.cursor),
            selection: hsla(style.selection),
            ansi: style.ansi.map(hsla),
            semantic: semantic_colors(style),
        }
    }

    fn resolve(&self, color: Color) -> Hsla {
        match color {
            Color::Foreground => self.foreground,
            Color::Background => self.background,
            Color::Ansi(index) => self.ansi[usize::from(index) % 16],
            Color::Rgb(Rgb { r, g, b }) => Rgba {
                r: f32::from(r) / 255.0,
                g: f32::from(g) / 255.0,
                b: f32::from(b) / 255.0,
                a: 1.0,
            }
            .into(),
        }
    }

    fn text_foreground(
        &self,
        cell: &GridCell,
        role: Option<crate::highlighting::Role>,
        block: bool,
    ) -> Hsla {
        let (foreground, background) = self.cell(cell);
        if block {
            background
        } else {
            role.map_or(foreground, |role| self.semantic[role.index()])
        }
    }

    /// A cell's text and background colours, after its style is applied.
    pub(super) fn cell(&self, cell: &GridCell) -> (Hsla, Hsla) {
        let (mut foreground, mut background) = (self.resolve(cell.fg), self.resolve(cell.bg));
        if cell.style.inverse {
            std::mem::swap(&mut foreground, &mut background);
        }
        if cell.style.dim {
            foreground = foreground.opacity(DIM_OPACITY);
        }
        (foreground, background)
    }
}

#[cfg(test)]
mod combining_tests {
    use super::*;
    #[test]
    fn combining_marks_keep_the_base_style_and_utf8_run_lengths() {
        let base = GridCell {
            ch: 'e',
            style: Style {
                bold: true,
                underline: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut segment = Segment::default();
        segment.push_cell(
            &base,
            "\u{301}\u{323}",
            gpui_kit::black(),
            &font("monospace"),
        );
        segment.push_cell(
            &GridCell { ch: 'X', ..base },
            "",
            gpui_kit::black(),
            &font("monospace"),
        );
        assert_eq!(segment.text, "e\u{301}\u{323}X");
        assert_eq!(segment.runs.len(), 1);
        assert_eq!(segment.runs[0].len, segment.text.len());
        assert_eq!(segment.runs[0].font.weight, FontWeight::BOLD);
        assert!(segment.runs[0].underline.is_some());
    }
    #[test]
    fn hidden_cells_do_not_leak_combining_marks() {
        let mut segment = Segment::default();
        let hidden = GridCell {
            ch: 'e',
            style: Style {
                hidden: true,
                ..Default::default()
            },
            ..Default::default()
        };
        segment.push_cell(&hidden, "\u{301}", gpui_kit::black(), &font("monospace"));
        assert_eq!(segment.text, " ");
        assert!(!segment.visible);
    }
}

/// Use the current ANSI family only when it meets normal text contrast.
fn semantic_colors(style: &TerminalStyle) -> [Hsla; crate::highlighting::Role::COUNT] {
    use crate::highlighting::Role;
    let luminance = |color: nocterm_ui::Color| {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    };
    let background = luminance(style.background);
    let contrast = |color| {
        let foreground = luminance(color);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    };
    [
        Role::Error,
        Role::Warning,
        Role::Success,
        Role::Address,
        Role::Timestamp,
        Role::Identifier,
        Role::Version,
        Role::Info,
    ]
    .map(|role| {
        [style.ansi[role.ansi()], style.ansi[role.ansi() + 8]]
            .into_iter()
            .find(|color| color.a == 255 && contrast(*color) >= 4.5)
            .map(hsla)
            .unwrap_or_else(|| hsla(style.foreground))
    })
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    use crate::highlighting::Role;

    fn style(background: nocterm_ui::Color) -> TerminalStyle {
        use nocterm_ui::Color;
        TerminalStyle {
            font_family: "monospace".into(),
            font_size: px(14.0),
            line_height: 1.2,
            padding: gpui_kit::rems(0.0),
            show_timestamps: false,
            show_line_numbers: false,
            semantic_highlighting: true,
            gutter_foreground: Color::rgb(128, 128, 128),
            foreground: Color::rgb(128, 128, 128),
            background,
            cursor: Color::rgb(255, 255, 255),
            selection: Color::rgb(50, 50, 50),
            ansi: [Color::rgb(128, 128, 128); 16],
        }
    }

    #[test]
    fn theme_roles_choose_readable_family_and_fall_back_to_foreground() {
        use nocterm_ui::Color;
        let normal = Color::rgb(128, 0, 0);
        let bright = Color::rgb(255, 120, 120);
        let mut dark = style(Color::rgb(0, 0, 0));
        dark.ansi[1] = normal;
        dark.ansi[9] = bright;
        assert_eq!(semantic_colors(&dark)[Role::Error.index()], hsla(bright));
        let mut light = dark.clone();
        light.background = Color::rgb(255, 255, 255);
        assert_eq!(semantic_colors(&light)[Role::Error.index()], hsla(normal));
        dark.ansi[9] = normal;
        assert_eq!(
            semantic_colors(&dark)[Role::Error.index()],
            hsla(dark.foreground)
        );
        dark.ansi[9] = Color { a: 128, ..bright };
        assert_eq!(
            semantic_colors(&dark)[Role::Error.index()],
            hsla(dark.foreground)
        );
    }

    #[test]
    fn block_cursor_keeps_its_original_background_as_text_foreground() {
        let style = style(nocterm_ui::Color::rgb(0, 0, 0));
        let colors = Colors::new(&style);
        let cell = GridCell {
            ch: 'F',
            ..Default::default()
        };
        assert_eq!(
            colors.text_foreground(&cell, Some(Role::Error), true),
            colors.background
        );
        assert_eq!(
            colors.text_foreground(&cell, None, false),
            colors.foreground
        );
    }
}
