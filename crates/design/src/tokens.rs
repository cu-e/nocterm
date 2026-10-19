//! The typed model of a token file.
//!
//! The doc comment of every field doubles as its entry in the generated
//! reference (`docs/reference/design-tokens.md`), so describe the token the
//! way a person restyling the app needs it described.

use std::{borrow::Cow, collections::BTreeMap, fmt};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, Visitor},
};

use crate::Color;

mod codec;
mod layout;
mod terminal;
pub use layout::{Layout, WidthBounds};
pub use terminal::{AnsiColors, TerminalColors};

const BUILTIN: &str = include_str!("../tokens/default.toml");

/// A token file is malformed or holds a value nocterm cannot render with.
#[derive(Debug)]
pub enum TokenError {
    Parse(Box<toml::de::Error>),
    Invalid { token: &'static str, reason: String },
}
impl fmt::Display for TokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(source) => write!(formatter, "invalid design tokens: {source}"),
            Self::Invalid { token, reason } => {
                write!(formatter, "invalid design token `{token}`: {reason}")
            }
        }
    }
}
impl std::error::Error for TokenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(source) => Some(source.as_ref()),
            Self::Invalid { .. } => None,
        }
    }
}
impl From<Box<toml::de::Error>> for TokenError {
    fn from(source: Box<toml::de::Error>) -> Self {
        Self::Parse(source)
    }
}

/// Everything that determines how nocterm looks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DesignTokens {
    /// Display name of this token set.
    pub name: String,
    /// Fonts and text sizes.
    pub typography: Typography,
    /// Corner radii and elevation.
    pub shape: Shape,
    /// Sizes of the interface's fixed regions, in rem.
    pub layout: Layout,
    /// Colours used while the interface is light.
    pub light: Palette,
    /// Colours used while the interface is dark.
    pub dark: Palette,
}

/// Fonts and text sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Typography {
    /// Interface font family. Unset: the platform's interface font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_font: Option<String>,
    /// Base interface font size in pixels; this is one rem. Unset: 16.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_size: Option<f32>,
    /// Terminal font family. Unset: the platform's monospace font.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono_font: Option<String>,
    /// Terminal font size in pixels. Unset: 13.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mono_size: Option<f32>,
    /// Explorer file and folder text size in pixels. Unset: 12.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explorer_size: Option<f32>,
    /// Default button label size in pixels. Other button sizes keep their relative hierarchy.
    pub button_size: f32,
    /// Terminal line height, as a multiple of the terminal font size.
    pub terminal_line_height: f32,
}

impl Typography {
    /// The terminal font size, in pixels, when `mono_size` is unset.
    pub const DEFAULT_MONO_SIZE: f32 = 13.0;

    /// The terminal font size, in pixels.
    pub fn mono_size(&self) -> f32 {
        self.mono_size.unwrap_or(Self::DEFAULT_MONO_SIZE)
    }
}

/// Corner radii and elevation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    /// Corner radius of controls, in pixels. Unset: 6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    /// Corner radius of dialogs, popovers and notifications, in pixels. Unset: 8.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_lg: Option<f32>,
    /// Whether controls cast elevation shadows. Unset: yes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<bool>,
}

/// The colours of one appearance, light or dark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    /// Interface colour overrides, keyed by the component theme's colour names
    /// (`background`, `primary.background`, `sidebar.background`, …). Anything
    /// not listed keeps the standard theme's value.
    #[serde(default)]
    pub ui: UiColors,
    /// The window's backdrop behind floating cards: the gaps between them and
    /// the title and status bars. Unset: a shade darker than `background`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<Color>,
    /// Colours of the terminal grid.
    pub terminal: TerminalColors,
}

/// Interface colour overrides, keyed by the component theme's colour names.
///
/// A token file may nest the keys (`primary.background = "…"` is the table
/// `primary` holding `background`) or quote them (`"link.hover" = "…"`); both
/// spell the same flat name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UiColors(BTreeMap<String, Color>);

impl UiColors {
    /// The overrides, by colour name.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Color)> {
        self.0.iter().map(|(name, color)| (name.as_str(), *color))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, Color)> for UiColors {
    fn from_iter<I: IntoIterator<Item = (String, Color)>>(colors: I) -> Self {
        Self(colors.into_iter().collect())
    }
}

impl Serialize for UiColors {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UiColors {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        enum Node {
            Color(Color),
            Group(BTreeMap<String, Node>),
        }

        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a colour or a table of colours")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Node, E> {
                text.parse().map(Node::Color).map_err(E::custom)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut children = BTreeMap::new();
                while let Some((name, node)) = map.next_entry::<String, Node>()? {
                    children.insert(name, node);
                }
                Ok(Node::Group(children))
            }
        }

        impl<'de> Deserialize<'de> for Node {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                deserializer.deserialize_any(NodeVisitor)
            }
        }

        fn flatten(prefix: &str, node: Node, colors: &mut BTreeMap<String, Color>) {
            match node {
                Node::Color(color) => {
                    colors.insert(prefix.to_owned(), color);
                }
                Node::Group(children) => {
                    for (name, child) in children {
                        let path = if prefix.is_empty() {
                            name
                        } else {
                            format!("{prefix}.{name}")
                        };
                        flatten(&path, child, colors);
                    }
                }
            }
        }

        let root = Node::Group(BTreeMap::<String, Node>::deserialize(deserializer)?);
        let mut colors = BTreeMap::new();
        flatten("", root, &mut colors);
        Ok(Self(colors))
    }
}

impl JsonSchema for UiColors {
    fn schema_name() -> Cow<'static, str> {
        "UiColors".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "additionalProperties": true,
        })
    }
}

impl DesignTokens {
    /// The built-in token file, verbatim.
    pub const BUILTIN_SOURCE: &str = BUILTIN;

    /// The tokens nocterm ships with.
    pub fn builtin() -> Self {
        Self::with_overrides("").expect("the built-in design tokens are valid")
    }

    /// The built-in tokens with `overrides` — a partial token file — layered
    /// on top, key by key.
    pub fn with_overrides(overrides: &str) -> Result<Self, TokenError> {
        let mut source: toml::Table = toml::from_str(BUILTIN).map_err(Box::new)?;
        let overrides: toml::Table = toml::from_str(overrides).map_err(Box::new)?;
        merge(&mut source, overrides);

        let tokens: Self = source.try_into().map_err(Box::new)?;
        tokens.validate()?;
        Ok(tokens)
    }

    /// The colours for the given appearance.
    pub fn palette(&self, dark: bool) -> &Palette {
        if dark { &self.dark } else { &self.light }
    }

    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn validate(&self) -> Result<(), TokenError> {
        if self.name.trim().is_empty() {
            return Err(invalid("name", "must not be empty"));
        }

        let typography = &self.typography;
        within(
            "typography.terminal_line_height",
            typography.terminal_line_height,
            1.0,
            3.0,
        )?;
        if let Some(size) = typography.ui_size {
            within("typography.ui_size", size, 8.0, 40.0)?;
        }
        if let Some(size) = typography.mono_size {
            within("typography.mono_size", size, 6.0, 72.0)?;
        }
        if let Some(size) = typography.explorer_size {
            within("typography.explorer_size", size, 8.0, 24.0)?;
        }
        within("typography.button_size", typography.button_size, 8.0, 24.0)?;
        for (token, family) in [
            ("typography.ui_font", &typography.ui_font),
            ("typography.mono_font", &typography.mono_font),
        ] {
            if family
                .as_deref()
                .is_some_and(|family| family.trim().is_empty())
            {
                return Err(invalid(token, "must not be empty; leave it unset instead"));
            }
        }

        for (token, radius) in [
            ("shape.radius", self.shape.radius),
            ("shape.radius_lg", self.shape.radius_lg),
        ] {
            if let Some(radius) = radius {
                within(token, radius, 0.0, 64.0)?;
            }
        }

        let layout = &self.layout;
        within(
            "layout.sidebar_min_width",
            layout.sidebar.min_width,
            4.0,
            100.0,
        )?;
        within(
            "layout.sidebar_max_width",
            layout.sidebar.max_width,
            layout.sidebar.min_width,
            200.0,
        )?;
        within(
            "layout.sidebar_width",
            layout.sidebar.width,
            layout.sidebar.min_width,
            layout.sidebar.max_width,
        )?;
        within(
            "layout.agent_panel_min_width",
            layout.agent_panel.min_width,
            1.0,
            100.0,
        )?;
        within(
            "layout.agent_panel_max_width",
            layout.agent_panel.max_width,
            layout.agent_panel.min_width,
            100.0,
        )?;
        within(
            "layout.agent_panel_width",
            layout.agent_panel.width,
            layout.agent_panel.min_width,
            layout.agent_panel.max_width,
        )?;
        within("layout.tab_max_width", layout.tab_max_width, 4.0, 100.0)?;
        within("layout.terminal_padding", layout.terminal_padding, 0.0, 8.0)?;
        within("layout.picker_width", layout.picker_width, 10.0, 100.0)?;
        within("layout.dialog_width", layout.dialog_width, 10.0, 100.0)?;
        within("layout.settings_width", layout.settings_width, 10.0, 100.0)?;
        within(
            "layout.connection_editor_width",
            layout.connection_editor_width,
            20.0,
            100.0,
        )?;
        within(
            "layout.connection_editor_height",
            layout.connection_editor_height,
            10.0,
            100.0,
        )?;
        within(
            "layout.connection_nav_width",
            layout.connection_nav_width,
            4.0,
            30.0,
        )?;
        within(
            "layout.connection_description_height",
            layout.connection_description_height,
            2.0,
            30.0,
        )?;
        within("layout.button_height", layout.button_height, 1.0, 4.0)?;
        within("layout.button_padding", layout.button_padding, 0.1, 2.0)?;
        within(
            "layout.local_terminal_height",
            layout.local_terminal_height,
            5.0,
            60.0,
        )?;
        Ok(())
    }
}

impl Default for DesignTokens {
    fn default() -> Self {
        Self::builtin()
    }
}

fn invalid(token: &'static str, reason: impl Into<String>) -> TokenError {
    TokenError::Invalid {
        token,
        reason: reason.into(),
    }
}

fn within(token: &'static str, value: f32, min: f32, max: f32) -> Result<(), TokenError> {
    // Written as a negated range check so that NaN is rejected too.
    if !(min..=max).contains(&value) {
        return Err(invalid(token, format!("{value} is outside {min}..={max}")));
    }
    Ok(())
}

/// Layers `overlay` over `base`: tables merge key by key, anything else replaces.
fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(base)), toml::Value::Table(overlay)) => merge(base, overlay),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod format_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn builtin_tokens_are_valid() {
        let tokens = DesignTokens::builtin();

        assert_eq!(tokens.name, "Nocterm Default");
        assert!(tokens.light.ui.is_empty());
        assert_eq!(tokens.typography.ui_font, None);
    }

    #[test]
    fn builtin_tokens_round_trip_through_toml() {
        let tokens = DesignTokens::builtin();
        let text = toml::to_string(&tokens).unwrap();

        assert_eq!(DesignTokens::with_overrides(&text).unwrap(), tokens);
    }

    #[test]
    fn overrides_merge_key_by_key() {
        let tokens = DesignTokens::with_overrides(
            r##"
            [typography]
            mono_size = 15.0

            [dark.terminal]
            red = "#ff0000"
            "##,
        )
        .unwrap();
        let builtin = DesignTokens::builtin();

        assert_eq!(tokens.typography.mono_size, Some(15.0));
        assert_eq!(tokens.dark.terminal.normal.red, Color::rgb(255, 0, 0));
        // Siblings of an overridden key keep their built-in values.
        assert_eq!(
            tokens.typography.terminal_line_height,
            builtin.typography.terminal_line_height
        );
        assert_eq!(
            tokens.dark.terminal.normal.green,
            builtin.dark.terminal.normal.green
        );
        assert_eq!(tokens.light, builtin.light);
    }

    #[test]
    fn ui_colours_accept_nested_and_quoted_names() {
        let tokens = DesignTokens::with_overrides(
            r##"
            [dark.ui]
            background = "#101010"
            primary.background = "#3366ff"
            tab.active.foreground = "#ffffff"
            "link.hover" = "#99aaff"
            "##,
        )
        .unwrap();

        let names: Vec<&str> = tokens.dark.ui.iter().map(|(name, _)| name).collect();
        assert_eq!(
            names,
            [
                "background",
                "link.hover",
                "primary.background",
                "tab.active.background",
                "tab.active.foreground"
            ]
        );
    }

    #[test]
    fn unknown_tokens_are_rejected() {
        let error = DesignTokens::with_overrides("[layout]\nsidebar_widht = 20.0\n").unwrap_err();

        assert!(error.to_string().contains("sidebar_widht"), "{error}");
        let TokenError::Parse(source) = &error else {
            panic!("a rejected TOML token must retain its parser classification");
        };
        let native = error
            .source()
            .unwrap()
            .downcast_ref::<toml::de::Error>()
            .unwrap();
        assert!(std::ptr::eq(native, source.as_ref()));
        assert!(native.source().is_none(), "the codec exposes a leaf error");
        assert_eq!(
            error.to_string(),
            format!("invalid design tokens: {native}")
        );
    }

    #[test]
    fn malformed_colours_are_rejected() {
        let error = DesignTokens::with_overrides("[dark.ui]\nbackground = \"blue\"\n").unwrap_err();

        assert!(error.to_string().contains("blue"), "{error}");
        let TokenError::Parse(source) = &error else {
            panic!("a rejected TOML token must retain its parser classification");
        };
        let native = error
            .source()
            .unwrap()
            .downcast_ref::<toml::de::Error>()
            .unwrap();
        assert!(std::ptr::eq(native, source.as_ref()));
        assert!(native.source().is_none(), "the codec exposes a leaf error");
        assert_eq!(
            error.to_string(),
            format!("invalid design tokens: {native}")
        );
    }

    #[test]
    fn out_of_range_values_name_the_token() {
        let error = DesignTokens::with_overrides("[layout]\nsidebar_width = 500.0\n").unwrap_err();
        assert!(
            error.to_string().contains("layout.sidebar_width"),
            "{error}"
        );

        let error =
            DesignTokens::with_overrides("[typography]\nterminal_line_height = nan\n").unwrap_err();
        assert!(
            error.to_string().contains("terminal_line_height"),
            "{error}"
        );
    }

    #[test]
    fn boxed_parse_conversion_retains_original_codec_allocation_and_display() {
        let native = Box::new(toml::from_str::<toml::Table>("name = ").unwrap_err());
        let original = native.as_ref() as *const toml::de::Error;
        let description = native.to_string();
        let error = TokenError::from(native);
        assert!(matches!(&error, TokenError::Parse(_)));
        let source = error
            .source()
            .unwrap()
            .downcast_ref::<toml::de::Error>()
            .unwrap();
        assert!(std::ptr::eq(source, original));
        assert!(source.source().is_none());
        assert_eq!(
            error.to_string(),
            format!("invalid design tokens: {description}")
        );
        let invalid =
            DesignTokens::with_overrides("[layout]\nsidebar_width = 500.0\n").unwrap_err();
        assert!(matches!(
            &invalid,
            TokenError::Invalid {
                token: "layout.sidebar_width",
                ..
            }
        ));
        assert!(invalid.source().is_none());
    }

    #[test]
    fn ansi_colours_are_in_index_order() {
        let colors = DesignTokens::builtin().dark.terminal;
        let ansi = colors.ansi();

        assert_eq!(ansi[1], colors.normal.red);
        assert_eq!(ansi[8], colors.bright.black);
        assert_eq!(ansi[15], colors.bright.white);
    }
}
