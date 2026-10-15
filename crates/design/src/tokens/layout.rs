//! Pure grouped token data; its codec retains the original flat wire format.

mod codec;
mod schema;

/// Initial width and the permitted resize interval, in rem.
#[derive(Debug, Clone, PartialEq)]
pub struct WidthBounds {
    pub width: f32,
    pub min_width: f32,
    pub max_width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub agent_panel: WidthBounds,
    pub sidebar: WidthBounds,
    /// Width past which a session tab ellipsizes its title.
    pub tab_max_width: f32,
    /// Space between the terminal grid and the edge of its pane.
    pub terminal_padding: f32,
    /// Width of the "new tab" picker.
    pub picker_width: f32,
    /// Width of the connection editor dialog.
    pub dialog_width: f32,
    /// Maximum width of the settings form and its footer.
    pub settings_width: f32,
    /// Width of the connection editor including its section navigation.
    pub connection_editor_width: f32,
    /// Maximum height of the connection editor's scrolling form.
    pub connection_editor_height: f32,
    /// Width of the connection editor's section navigation.
    pub connection_nav_width: f32,
    /// Height of the connection description text area.
    pub connection_description_height: f32,
    /// Height of a default button; other button sizes keep their relative hierarchy.
    pub button_height: f32,
    /// Horizontal padding of a default button.
    pub button_padding: f32,
    /// Initial height of the independent local terminal dock, in rem.
    pub local_terminal_height: f32,
}
