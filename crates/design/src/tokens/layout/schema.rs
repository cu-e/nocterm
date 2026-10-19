//! Public flat token schema, independent of internal data grouping.
use super::*;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use std::borrow::Cow;
impl JsonSchema for Layout {
    fn schema_name() -> Cow<'static, str> {
        "Layout".into()
    }
    fn schema_id() -> Cow<'static, str> {
        "nocterm_design::tokens::Layout".into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "agent_panel_width": described(generator.subschema_for::<f32>(), "Initial width of the AI panel, in rem."),
                "agent_panel_min_width": described(generator.subschema_for::<f32>(), "Minimum AI panel width, in rem."),
                "agent_panel_max_width": described(generator.subschema_for::<f32>(), "Maximum AI panel width, in rem."),
                "sidebar_width": described(generator.subschema_for::<f32>(), "Initial width of the left sidebar."),
                "sidebar_min_width": described(generator.subschema_for::<f32>(), "Narrowest the sidebar can be dragged."),
                "sidebar_max_width": described(generator.subschema_for::<f32>(), "Widest the sidebar can be dragged."),
                "tab_max_width": described(generator.subschema_for::<f32>(), "Width past which a session tab ellipsizes its title."),
                "terminal_padding": described(generator.subschema_for::<f32>(), "Space between the terminal grid and the edge of its pane."),
                "picker_width": described(generator.subschema_for::<f32>(), "Width of the \"new tab\" picker."),
                "dialog_width": described(generator.subschema_for::<f32>(), "Width of the connection editor dialog."),
                "settings_width": described(generator.subschema_for::<f32>(), "Maximum width of the settings form and its footer."),
                "connection_editor_width": described(generator.subschema_for::<f32>(), "Width of the connection editor including its section navigation."),
                "connection_editor_height": described(generator.subschema_for::<f32>(), "Maximum height of the connection editor's scrolling form."),
                "connection_nav_width": described(generator.subschema_for::<f32>(), "Width of the connection editor's section navigation."),
                "connection_description_height": described(generator.subschema_for::<f32>(), "Height of the connection description text area."),
                "button_height": described(generator.subschema_for::<f32>(), "Height of a default button; other button sizes keep their relative hierarchy."),
                "button_padding": described(generator.subschema_for::<f32>(), "Horizontal padding of a default button."),
                "local_terminal_height": described(generator.subschema_for::<f32>(), "Initial height of the independent local terminal dock, in rem."),
            },
            "required": ["agent_panel_width", "agent_panel_min_width", "agent_panel_max_width", "sidebar_width", "sidebar_min_width", "sidebar_max_width", "tab_max_width", "terminal_padding", "picker_width", "dialog_width", "settings_width", "connection_editor_width", "connection_editor_height", "connection_nav_width", "connection_description_height", "button_height", "button_padding", "local_terminal_height"],
            "description": "Sizes of the interface's fixed regions, in rem.",
        })
    }
}
fn described(mut schema: Schema, description: &str) -> impl serde::Serialize {
    schema.insert("description".into(), description.into());
    // Preserve derived-schema insertion order instead of serializing Schema
    // through its presentation-only keyword reordering wrapper.
    schema.to_value()
}
