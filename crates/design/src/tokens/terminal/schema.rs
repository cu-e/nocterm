//! Public flat token schema, independent of internal data grouping.
use super::*;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use std::borrow::Cow;
impl JsonSchema for TerminalColors {
    fn schema_name() -> Cow<'static, str> {
        "TerminalColors".into()
    }
    fn schema_id() -> Cow<'static, str> {
        "nocterm_design::tokens::TerminalColors".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "background": described(generator.subschema_for::<Option<Color>>(), "Terminal background. Unset: the interface background."),
                "foreground": described(generator.subschema_for::<Option<Color>>(), "Default text colour. Unset: the interface foreground."),
                "cursor": described(generator.subschema_for::<Option<Color>>(), "Cursor colour. Unset: the terminal foreground."),
                "selection": described(generator.subschema_for::<Option<Color>>(), "Selection highlight. Unset: the interface selection colour."),
                "black": described(generator.subschema_for::<Color>(), "ANSI colour 0."),
                "red": described(generator.subschema_for::<Color>(), "ANSI colour 1."),
                "green": described(generator.subschema_for::<Color>(), "ANSI colour 2."),
                "yellow": described(generator.subschema_for::<Color>(), "ANSI colour 3."),
                "blue": described(generator.subschema_for::<Color>(), "ANSI colour 4."),
                "magenta": described(generator.subschema_for::<Color>(), "ANSI colour 5."),
                "cyan": described(generator.subschema_for::<Color>(), "ANSI colour 6."),
                "white": described(generator.subschema_for::<Color>(), "ANSI colour 7."),
                "bright_black": described(generator.subschema_for::<Color>(), "ANSI colour 8."),
                "bright_red": described(generator.subschema_for::<Color>(), "ANSI colour 9."),
                "bright_green": described(generator.subschema_for::<Color>(), "ANSI colour 10."),
                "bright_yellow": described(generator.subschema_for::<Color>(), "ANSI colour 11."),
                "bright_blue": described(generator.subschema_for::<Color>(), "ANSI colour 12."),
                "bright_magenta": described(generator.subschema_for::<Color>(), "ANSI colour 13."),
                "bright_cyan": described(generator.subschema_for::<Color>(), "ANSI colour 14."),
                "bright_white": described(generator.subschema_for::<Color>(), "ANSI colour 15."),
            },
            "required": ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "bright_black", "bright_red", "bright_green", "bright_yellow", "bright_blue", "bright_magenta", "bright_cyan", "bright_white"],
            "description": "Colours of the terminal grid.",
        })
    }
}
fn described(mut schema: Schema, description: &str) -> impl serde::Serialize {
    schema.insert("description".into(), description.into());
    // Preserve derived-schema insertion order instead of serializing Schema
    // through its presentation-only keyword reordering wrapper.
    schema.to_value()
}
