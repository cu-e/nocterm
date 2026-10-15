//! Original flat token grammar with bounded, grouped decode state.
use super::super::codec::{NameSeed, next, required};
use super::*;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, MapAccess, SeqAccess, Visitor},
    ser::SerializeStruct,
};
use std::fmt;

#[derive(Default)]
struct NormalFields {
    black: Option<Color>,
    red: Option<Color>,
    green: Option<Color>,
    yellow: Option<Color>,
    blue: Option<Color>,
    magenta: Option<Color>,
    cyan: Option<Color>,
    white: Option<Color>,
}
#[derive(Default)]
struct BrightFields {
    black: Option<Color>,
    red: Option<Color>,
    green: Option<Color>,
    yellow: Option<Color>,
    blue: Option<Color>,
    magenta: Option<Color>,
    cyan: Option<Color>,
    white: Option<Color>,
}
#[derive(Default)]
struct Fields {
    normal: NormalFields,
    bright: BrightFields,
    background: Option<Option<Color>>,
    foreground: Option<Option<Color>>,
    cursor: Option<Option<Color>>,
    selection: Option<Option<Color>>,
}
const NAMES: &[&str] = &[
    "background",
    "foreground",
    "cursor",
    "selection",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

impl Serialize for TerminalColors {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            "TerminalColors",
            20 - usize::from(self.background.is_none())
                - usize::from(self.foreground.is_none())
                - usize::from(self.cursor.is_none())
                - usize::from(self.selection.is_none()),
        )?;
        if let Some(value) = &self.background {
            state.serialize_field("background", value)?;
        } else {
            state.skip_field("background")?;
        }
        if let Some(value) = &self.foreground {
            state.serialize_field("foreground", value)?;
        } else {
            state.skip_field("foreground")?;
        }
        if let Some(value) = &self.cursor {
            state.serialize_field("cursor", value)?;
        } else {
            state.skip_field("cursor")?;
        }
        if let Some(value) = &self.selection {
            state.serialize_field("selection", value)?;
        } else {
            state.skip_field("selection")?;
        }
        state.serialize_field("black", &self.normal.black)?;
        state.serialize_field("red", &self.normal.red)?;
        state.serialize_field("green", &self.normal.green)?;
        state.serialize_field("yellow", &self.normal.yellow)?;
        state.serialize_field("blue", &self.normal.blue)?;
        state.serialize_field("magenta", &self.normal.magenta)?;
        state.serialize_field("cyan", &self.normal.cyan)?;
        state.serialize_field("white", &self.normal.white)?;
        state.serialize_field("bright_black", &self.bright.black)?;
        state.serialize_field("bright_red", &self.bright.red)?;
        state.serialize_field("bright_green", &self.bright.green)?;
        state.serialize_field("bright_yellow", &self.bright.yellow)?;
        state.serialize_field("bright_blue", &self.bright.blue)?;
        state.serialize_field("bright_magenta", &self.bright.magenta)?;
        state.serialize_field("bright_cyan", &self.bright.cyan)?;
        state.serialize_field("bright_white", &self.bright.white)?;
        state.end()
    }
}
impl Fields {
    fn read0<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "black" => {
                if self.normal.black.is_some() {
                    return Err(de::Error::duplicate_field("black"));
                }
                self.normal.black = Some(map.next_value()?);
            }
            "red" => {
                if self.normal.red.is_some() {
                    return Err(de::Error::duplicate_field("red"));
                }
                self.normal.red = Some(map.next_value()?);
            }
            "green" => {
                if self.normal.green.is_some() {
                    return Err(de::Error::duplicate_field("green"));
                }
                self.normal.green = Some(map.next_value()?);
            }
            "yellow" => {
                if self.normal.yellow.is_some() {
                    return Err(de::Error::duplicate_field("yellow"));
                }
                self.normal.yellow = Some(map.next_value()?);
            }
            "blue" => {
                if self.normal.blue.is_some() {
                    return Err(de::Error::duplicate_field("blue"));
                }
                self.normal.blue = Some(map.next_value()?);
            }
            "magenta" => {
                if self.normal.magenta.is_some() {
                    return Err(de::Error::duplicate_field("magenta"));
                }
                self.normal.magenta = Some(map.next_value()?);
            }
            "cyan" => {
                if self.normal.cyan.is_some() {
                    return Err(de::Error::duplicate_field("cyan"));
                }
                self.normal.cyan = Some(map.next_value()?);
            }
            "white" => {
                if self.normal.white.is_some() {
                    return Err(de::Error::duplicate_field("white"));
                }
                self.normal.white = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn read1<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "bright_black" => {
                if self.bright.black.is_some() {
                    return Err(de::Error::duplicate_field("bright_black"));
                }
                self.bright.black = Some(map.next_value()?);
            }
            "bright_red" => {
                if self.bright.red.is_some() {
                    return Err(de::Error::duplicate_field("bright_red"));
                }
                self.bright.red = Some(map.next_value()?);
            }
            "bright_green" => {
                if self.bright.green.is_some() {
                    return Err(de::Error::duplicate_field("bright_green"));
                }
                self.bright.green = Some(map.next_value()?);
            }
            "bright_yellow" => {
                if self.bright.yellow.is_some() {
                    return Err(de::Error::duplicate_field("bright_yellow"));
                }
                self.bright.yellow = Some(map.next_value()?);
            }
            "bright_blue" => {
                if self.bright.blue.is_some() {
                    return Err(de::Error::duplicate_field("bright_blue"));
                }
                self.bright.blue = Some(map.next_value()?);
            }
            "bright_magenta" => {
                if self.bright.magenta.is_some() {
                    return Err(de::Error::duplicate_field("bright_magenta"));
                }
                self.bright.magenta = Some(map.next_value()?);
            }
            "bright_cyan" => {
                if self.bright.cyan.is_some() {
                    return Err(de::Error::duplicate_field("bright_cyan"));
                }
                self.bright.cyan = Some(map.next_value()?);
            }
            "bright_white" => {
                if self.bright.white.is_some() {
                    return Err(de::Error::duplicate_field("bright_white"));
                }
                self.bright.white = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn read2<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "background" => {
                if self.background.is_some() {
                    return Err(de::Error::duplicate_field("background"));
                }
                self.background = Some(map.next_value()?);
            }
            "foreground" => {
                if self.foreground.is_some() {
                    return Err(de::Error::duplicate_field("foreground"));
                }
                self.foreground = Some(map.next_value()?);
            }
            "cursor" => {
                if self.cursor.is_some() {
                    return Err(de::Error::duplicate_field("cursor"));
                }
                self.cursor = Some(map.next_value()?);
            }
            "selection" => {
                if self.selection.is_some() {
                    return Err(de::Error::duplicate_field("selection"));
                }
                self.selection = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn finish<E: de::Error>(self) -> Result<TerminalColors, E> {
        let black = required::<_, E>(self.normal.black, "black")?;
        let red = required::<_, E>(self.normal.red, "red")?;
        let green = required::<_, E>(self.normal.green, "green")?;
        let yellow = required::<_, E>(self.normal.yellow, "yellow")?;
        let blue = required::<_, E>(self.normal.blue, "blue")?;
        let magenta = required::<_, E>(self.normal.magenta, "magenta")?;
        let cyan = required::<_, E>(self.normal.cyan, "cyan")?;
        let white = required::<_, E>(self.normal.white, "white")?;
        let bright_black = required::<_, E>(self.bright.black, "bright_black")?;
        let bright_red = required::<_, E>(self.bright.red, "bright_red")?;
        let bright_green = required::<_, E>(self.bright.green, "bright_green")?;
        let bright_yellow = required::<_, E>(self.bright.yellow, "bright_yellow")?;
        let bright_blue = required::<_, E>(self.bright.blue, "bright_blue")?;
        let bright_magenta = required::<_, E>(self.bright.magenta, "bright_magenta")?;
        let bright_cyan = required::<_, E>(self.bright.cyan, "bright_cyan")?;
        let bright_white = required::<_, E>(self.bright.white, "bright_white")?;
        Ok(TerminalColors {
            normal: AnsiColors {
                black,
                red,
                green,
                yellow,
                blue,
                magenta,
                cyan,
                white,
            },
            bright: AnsiColors {
                black: bright_black,
                red: bright_red,
                green: bright_green,
                yellow: bright_yellow,
                blue: bright_blue,
                magenta: bright_magenta,
                cyan: bright_cyan,
                white: bright_white,
            },
            background: self.background.unwrap_or_default(),
            foreground: self.foreground.unwrap_or_default(),
            cursor: self.cursor.unwrap_or_default(),
            selection: self.selection.unwrap_or_default(),
        })
    }
}
struct TokenVisitor;
impl<'de> Visitor<'de> for TokenVisitor {
    type Value = TerminalColors;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("struct TerminalColors")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = Fields::default();
        while let Some(key) = map.next_key_seed(NameSeed::new(NAMES))? {
            if !(fields.read0(key, &mut map)?
                || fields.read1(key, &mut map)?
                || fields.read2(key, &mut map)?)
            {
                return Err(de::Error::unknown_field(key, NAMES));
            }
        }
        fields.finish()
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut fields = Fields::default();
        let expected = "struct TerminalColors with 20 elements";
        fields.background = Some(sequence.next_element()?.unwrap_or_default());
        fields.foreground = Some(sequence.next_element()?.unwrap_or_default());
        fields.cursor = Some(sequence.next_element()?.unwrap_or_default());
        fields.selection = Some(sequence.next_element()?.unwrap_or_default());
        fields.normal.black = Some(next(&mut sequence, 4, &expected)?);
        fields.normal.red = Some(next(&mut sequence, 5, &expected)?);
        fields.normal.green = Some(next(&mut sequence, 6, &expected)?);
        fields.normal.yellow = Some(next(&mut sequence, 7, &expected)?);
        fields.normal.blue = Some(next(&mut sequence, 8, &expected)?);
        fields.normal.magenta = Some(next(&mut sequence, 9, &expected)?);
        fields.normal.cyan = Some(next(&mut sequence, 10, &expected)?);
        fields.normal.white = Some(next(&mut sequence, 11, &expected)?);
        fields.bright.black = Some(next(&mut sequence, 12, &expected)?);
        fields.bright.red = Some(next(&mut sequence, 13, &expected)?);
        fields.bright.green = Some(next(&mut sequence, 14, &expected)?);
        fields.bright.yellow = Some(next(&mut sequence, 15, &expected)?);
        fields.bright.blue = Some(next(&mut sequence, 16, &expected)?);
        fields.bright.magenta = Some(next(&mut sequence, 17, &expected)?);
        fields.bright.cyan = Some(next(&mut sequence, 18, &expected)?);
        fields.bright.white = Some(next(&mut sequence, 19, &expected)?);
        fields.finish()
    }
}
impl<'de> Deserialize<'de> for TerminalColors {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_struct("TerminalColors", NAMES, TokenVisitor)
    }
}
