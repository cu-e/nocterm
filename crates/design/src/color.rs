//! The colour notation used by the token files.

use std::{borrow::Cow, fmt, str::FromStr};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// An sRGB colour with alpha, written `#rrggbb` or `#rrggbbaa`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    /// An opaque colour.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 0xff }
    }

    /// Linear interpolation towards `other`; `amount` 0 is `self`, 1 is `other`.
    pub fn mix(self, other: Self, amount: f32) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let channel = |from: u8, to: u8| {
            (f32::from(from) + (f32::from(to) - f32::from(from)) * amount).round() as u8
        };
        Self {
            r: channel(self.r, other.r),
            g: channel(self.g, other.g),
            b: channel(self.b, other.b),
            a: channel(self.a, other.a),
        }
    }
}

/// A string is not `#rrggbb` or `#rrggbbaa`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not a colour; expected #rrggbb or #rrggbbaa")]
pub struct ParseColorError(String);

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let error = || ParseColorError(text.to_owned());
        let digits = text.strip_prefix('#').ok_or_else(error)?;
        if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error());
        }
        let channel =
            |ix: usize| u8::from_str_radix(&digits[ix * 2..ix * 2 + 2], 16).map_err(|_| error());
        Ok(Self {
            r: channel(0)?,
            g: channel(1)?,
            b: channel(2)?,
            a: if digits.len() == 8 { channel(3)? } else { 0xff },
        })
    }
}

impl fmt::Display for Color {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)?;
        if self.a != 0xff {
            write!(formatter, "{:02x}", self.a)?;
        }
        Ok(())
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(de::Error::custom)
    }
}

impl JsonSchema for Color {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "format": "color",
            "pattern": "^#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?$",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_both_notations() {
        let opaque: Color = "#1A2b3C".parse().unwrap();
        assert_eq!(opaque, Color::rgb(0x1a, 0x2b, 0x3c));
        assert_eq!(opaque.to_string(), "#1a2b3c");

        let translucent: Color = "#1a2b3c80".parse().unwrap();
        assert_eq!(translucent.a, 0x80);
        assert_eq!(translucent.to_string(), "#1a2b3c80");
    }

    #[test]
    fn rejects_malformed_colours() {
        for text in [
            "",
            "1a2b3c",
            "#1a2b3",
            "#1a2b3cg0",
            "#1a2b3c8",
            "red",
            "#€€",
        ] {
            assert!(text.parse::<Color>().is_err(), "{text:?} should not parse");
        }
    }

    #[test]
    fn mixes_towards_the_other_colour() {
        let black = Color::rgb(0, 0, 0);
        let white = Color::rgb(255, 255, 255);

        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5), Color::rgb(128, 128, 128));
    }
}
