//! Tolerant parsing of the original Zed theme family format.
use crate::{FILE_LIMIT, ThemeError};
use nocterm_design::Color;
use serde::{Deserialize, Serialize};
use serde_json_lenient::Value;
use std::collections::BTreeMap;
mod palette;
#[cfg(test)]
mod tests;

/// Which colour scheme a variant belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}
impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Self::Dark
    }
}
/// Only the first player's cursor and selection are used.
#[derive(Debug, Clone, Default)]
pub struct Player {
    pub cursor: Option<Color>,
    pub selection: Option<Color>,
}
/// A named variant in a theme family.
#[derive(Debug, Clone)]
pub struct ZedTheme {
    pub name: String,
    pub appearance: Appearance,
    pub colors: BTreeMap<String, Color>,
    pub players: Vec<Player>,
}
/// Theme file metadata and its valid variants.
#[derive(Debug, Clone)]
pub struct ThemeFamily {
    pub name: String,
    pub author: String,
    pub themes: Vec<ZedTheme>,
}

/// Accept Zed's short and full hex notation; all other values are unset.
pub fn parse_color(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 | 4 => {
            let mut channels = [255; 4];
            for (i, byte) in hex.bytes().enumerate() {
                channels[i] = (byte as char).to_digit(16)? as u8 * 17;
            }
            Some(Color {
                r: channels[0],
                g: channels[1],
                b: channels[2],
                a: channels[3],
            })
        }
        6 | 8 => text.parse().ok(),
        _ => None,
    }
}
fn color(value: &Value) -> Option<Color> {
    parse_color(value.as_str()?)
}

/// Parse comments, trailing commas and BOMs, ignoring unsupported style fields.
pub fn parse_family(bytes: &[u8]) -> Result<ThemeFamily, ThemeError> {
    if bytes.len() > FILE_LIMIT {
        return Err(ThemeError::Invalid("theme file exceeds 8 MiB".into()));
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: Value = serde_json_lenient::from_slice(bytes)
        .map_err(|e| ThemeError::Invalid(format!("invalid Zed theme: {e}")))?;
    let mut family = ThemeFamily {
        name: value["name"]
            .as_str()
            .unwrap_or("Unnamed family")
            .to_owned(),
        author: value["author"].as_str().unwrap_or_default().to_owned(),
        themes: Vec::new(),
    };
    for theme in value["themes"].as_array().into_iter().flatten() {
        let Some(name) = theme["name"].as_str().filter(|n| !n.trim().is_empty()) else {
            continue;
        };
        let appearance = match theme["appearance"].as_str() {
            Some("light") => Appearance::Light,
            Some("dark") => Appearance::Dark,
            _ => continue,
        };
        let Some(style) = theme["style"].as_object() else {
            continue;
        };
        let mut colors: BTreeMap<_, _> = style
            .iter()
            .filter_map(|(k, v)| color(v).map(|c| (k.clone(), c)))
            .collect();
        for (alias, canonical) in [
            ("scrollbar_thumb.background", "scrollbar.thumb.background"),
            (
                "element.selection.background",
                "element.selection_background",
            ),
        ] {
            if let Some(c) = colors.get(alias).copied() {
                colors.entry(canonical.into()).or_insert(c);
            }
        }
        let players = style
            .get("players")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(1)
            .map(|v| Player {
                cursor: color(&v["cursor"]),
                selection: color(&v["selection"]),
            })
            .collect();
        family.themes.push(ZedTheme {
            name: name.trim().into(),
            appearance,
            colors,
            players,
        });
    }
    if family.themes.is_empty() {
        return Err(ThemeError::Invalid(
            "theme family has no valid themes".into(),
        ));
    }
    Ok(family)
}
