//! Stable public schema identity for the established settings format.
use super::*;
impl JsonSchema for TerminalSettings {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TerminalSettings".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_ui::sections::TerminalSettings".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let default = Self::default();
        let mut schema = nocterm_settings::schema::SectionSchema::new(generator);
        schema.field(
            "charset",
            &default.charset,
            "Text encoding used when a session has no override.",
        );
        schema.optional(
            "font_family",
            &default.font_family,
            "Font family. Unset: the design tokens' terminal font.",
            default.font_family.is_none(),
        );
        schema.optional(
            "font_size",
            &default.font_size,
            "Font size in pixels. Unset: the design tokens' terminal size.",
            default.font_size.is_none(),
        );
        schema.range("font_size", FONT_SIZE_RANGE.start(), FONT_SIZE_RANGE.end());
        schema.optional(
            "line_height",
            &default.line_height,
            "Line height as a multiple of the font size. Unset: the design tokens' value.",
            default.line_height.is_none(),
        );
        schema.range(
            "line_height",
            LINE_HEIGHT_RANGE.start(),
            LINE_HEIGHT_RANGE.end(),
        );
        schema.field(
            "cursor_shape",
            &default.cursor_shape,
            "Shape of the cursor.",
        );
        schema.field(
            "cursor_blink",
            &default.cursor_blink,
            "Blink the cursor while the terminal has focus.",
        );
        schema.field(
            "scrollback_lines",
            &default.scrollback_lines,
            "Lines of history kept above the visible screen.",
        );
        schema.range(
            "scrollback_lines",
            SCROLLBACK_RANGE.start(),
            SCROLLBACK_RANGE.end(),
        );
        schema.field(
            "copy_on_select",
            &default.copy_on_select,
            "Copy text to the clipboard as soon as it is selected.",
        );
        schema.field("clipboard_write", &default.clipboard_write, "Allow programs to write the clipboard only while their terminal screen has focus.\nDenied by default; clipboard reads are always denied.");
        schema.field(
            "term",
            &default.term,
            "Terminal type announced to the remote host (its `TERM` variable).",
        );
        schema.field("show_timestamps", &default.show_timestamps, "Show the timestamp of the first output on each logical line.\nShow each logical line's first output time in UTC (this machine's clock).");
        schema.field(
            "show_line_numbers",
            &default.show_line_numbers,
            "Show sequential logical line numbers outside the terminal grid.",
        );
        schema.field(
            "semantic_highlighting",
            &default.semantic_highlighting,
            "Highlight dates, addresses and important messages in otherwise unstyled output.",
        );
        schema.finish("How terminals look and behave.")
    }
}

impl JsonSchema for CursorShape {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CursorShape".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_ui::sections::CursorShape".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"string","enum":["block", "bar", "underline"],"description":"Shape of the terminal cursor."})
    }
}

impl JsonSchema for ClipboardWritePolicy {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ClipboardWritePolicy".into()
    }
    fn schema_id() -> std::borrow::Cow<'static, str> {
        "nocterm_ui::sections::ClipboardWritePolicy".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"type":"string","enum":["deny", "focused_terminal"],"description":"Permission for terminal output (OSC 52) to change the system clipboard."})
    }
}
