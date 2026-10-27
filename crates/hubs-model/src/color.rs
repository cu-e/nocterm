//! The colour a server is marked with in menus.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A named colour. The views map each name to the current theme, so a server keeps its colour
/// across themes; the names, not the shades, are what the settings keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HubColor {
    Red,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
}

impl HubColor {
    /// Every colour, in the order a chooser lists them.
    pub const ALL: [Self; 6] = [
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Magenta,
    ];

    /// The name the settings and the views use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Cyan => "cyan",
            Self::Blue => "blue",
            Self::Magenta => "magenta",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_the_one_the_settings_use() {
        for color in HubColor::ALL {
            let value = toml::Value::try_from(color).unwrap();
            assert_eq!(value, toml::Value::String(color.name().to_owned()));
        }
    }
}
