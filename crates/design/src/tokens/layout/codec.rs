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
struct AgentPanelFields {
    width: Option<f32>,
    min_width: Option<f32>,
    max_width: Option<f32>,
}
#[derive(Default)]
struct SidebarFields {
    width: Option<f32>,
    min_width: Option<f32>,
    max_width: Option<f32>,
}
#[derive(Default)]
struct Fields {
    agent_panel: AgentPanelFields,
    sidebar: SidebarFields,
    tab_max_width: Option<f32>,
    terminal_padding: Option<f32>,
    picker_width: Option<f32>,
    dialog_width: Option<f32>,
    settings_width: Option<f32>,
    connection_editor_width: Option<f32>,
    connection_editor_height: Option<f32>,
    connection_nav_width: Option<f32>,
    connection_description_height: Option<f32>,
    button_height: Option<f32>,
    button_padding: Option<f32>,
    local_terminal_height: Option<f32>,
    status_bar_center_max: Option<f32>,
}
const NAMES: &[&str] = &[
    "agent_panel_width",
    "agent_panel_min_width",
    "agent_panel_max_width",
    "sidebar_width",
    "sidebar_min_width",
    "sidebar_max_width",
    "tab_max_width",
    "terminal_padding",
    "picker_width",
    "dialog_width",
    "settings_width",
    "connection_editor_width",
    "connection_editor_height",
    "connection_nav_width",
    "connection_description_height",
    "button_height",
    "button_padding",
    "local_terminal_height",
    "status_bar_center_max",
];

impl Serialize for Layout {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Layout", 19)?;
        state.serialize_field("agent_panel_width", &self.agent_panel.width)?;
        state.serialize_field("agent_panel_min_width", &self.agent_panel.min_width)?;
        state.serialize_field("agent_panel_max_width", &self.agent_panel.max_width)?;
        state.serialize_field("sidebar_width", &self.sidebar.width)?;
        state.serialize_field("sidebar_min_width", &self.sidebar.min_width)?;
        state.serialize_field("sidebar_max_width", &self.sidebar.max_width)?;
        state.serialize_field("tab_max_width", &self.tab_max_width)?;
        state.serialize_field("terminal_padding", &self.terminal_padding)?;
        state.serialize_field("picker_width", &self.picker_width)?;
        state.serialize_field("dialog_width", &self.dialog_width)?;
        state.serialize_field("settings_width", &self.settings_width)?;
        state.serialize_field("connection_editor_width", &self.connection_editor_width)?;
        state.serialize_field("connection_editor_height", &self.connection_editor_height)?;
        state.serialize_field("connection_nav_width", &self.connection_nav_width)?;
        state.serialize_field(
            "connection_description_height",
            &self.connection_description_height,
        )?;
        state.serialize_field("button_height", &self.button_height)?;
        state.serialize_field("button_padding", &self.button_padding)?;
        state.serialize_field("local_terminal_height", &self.local_terminal_height)?;
        state.serialize_field("status_bar_center_max", &self.status_bar_center_max)?;
        state.end()
    }
}
impl Fields {
    fn read0<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "agent_panel_width" => {
                if self.agent_panel.width.is_some() {
                    return Err(de::Error::duplicate_field("agent_panel_width"));
                }
                self.agent_panel.width = Some(map.next_value()?);
            }
            "agent_panel_min_width" => {
                if self.agent_panel.min_width.is_some() {
                    return Err(de::Error::duplicate_field("agent_panel_min_width"));
                }
                self.agent_panel.min_width = Some(map.next_value()?);
            }
            "agent_panel_max_width" => {
                if self.agent_panel.max_width.is_some() {
                    return Err(de::Error::duplicate_field("agent_panel_max_width"));
                }
                self.agent_panel.max_width = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn read1<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "sidebar_width" => {
                if self.sidebar.width.is_some() {
                    return Err(de::Error::duplicate_field("sidebar_width"));
                }
                self.sidebar.width = Some(map.next_value()?);
            }
            "sidebar_min_width" => {
                if self.sidebar.min_width.is_some() {
                    return Err(de::Error::duplicate_field("sidebar_min_width"));
                }
                self.sidebar.min_width = Some(map.next_value()?);
            }
            "sidebar_max_width" => {
                if self.sidebar.max_width.is_some() {
                    return Err(de::Error::duplicate_field("sidebar_max_width"));
                }
                self.sidebar.max_width = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn read2<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "tab_max_width" => {
                if self.tab_max_width.is_some() {
                    return Err(de::Error::duplicate_field("tab_max_width"));
                }
                self.tab_max_width = Some(map.next_value()?);
            }
            "terminal_padding" => {
                if self.terminal_padding.is_some() {
                    return Err(de::Error::duplicate_field("terminal_padding"));
                }
                self.terminal_padding = Some(map.next_value()?);
            }
            "picker_width" => {
                if self.picker_width.is_some() {
                    return Err(de::Error::duplicate_field("picker_width"));
                }
                self.picker_width = Some(map.next_value()?);
            }
            "dialog_width" => {
                if self.dialog_width.is_some() {
                    return Err(de::Error::duplicate_field("dialog_width"));
                }
                self.dialog_width = Some(map.next_value()?);
            }
            "settings_width" => {
                if self.settings_width.is_some() {
                    return Err(de::Error::duplicate_field("settings_width"));
                }
                self.settings_width = Some(map.next_value()?);
            }
            "connection_editor_width" => {
                if self.connection_editor_width.is_some() {
                    return Err(de::Error::duplicate_field("connection_editor_width"));
                }
                self.connection_editor_width = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn read3<'de, A: MapAccess<'de>>(&mut self, key: &str, map: &mut A) -> Result<bool, A::Error> {
        match key {
            "connection_editor_height" => {
                if self.connection_editor_height.is_some() {
                    return Err(de::Error::duplicate_field("connection_editor_height"));
                }
                self.connection_editor_height = Some(map.next_value()?);
            }
            "connection_nav_width" => {
                if self.connection_nav_width.is_some() {
                    return Err(de::Error::duplicate_field("connection_nav_width"));
                }
                self.connection_nav_width = Some(map.next_value()?);
            }
            "connection_description_height" => {
                if self.connection_description_height.is_some() {
                    return Err(de::Error::duplicate_field("connection_description_height"));
                }
                self.connection_description_height = Some(map.next_value()?);
            }
            "button_height" => {
                if self.button_height.is_some() {
                    return Err(de::Error::duplicate_field("button_height"));
                }
                self.button_height = Some(map.next_value()?);
            }
            "button_padding" => {
                if self.button_padding.is_some() {
                    return Err(de::Error::duplicate_field("button_padding"));
                }
                self.button_padding = Some(map.next_value()?);
            }
            "local_terminal_height" => {
                if self.local_terminal_height.is_some() {
                    return Err(de::Error::duplicate_field("local_terminal_height"));
                }
                self.local_terminal_height = Some(map.next_value()?);
            }
            "status_bar_center_max" => {
                if self.status_bar_center_max.is_some() {
                    return Err(de::Error::duplicate_field("status_bar_center_max"));
                }
                self.status_bar_center_max = Some(map.next_value()?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn finish<E: de::Error>(self) -> Result<Layout, E> {
        let agent_panel_width = required::<_, E>(self.agent_panel.width, "agent_panel_width")?;
        let agent_panel_min_width =
            required::<_, E>(self.agent_panel.min_width, "agent_panel_min_width")?;
        let agent_panel_max_width =
            required::<_, E>(self.agent_panel.max_width, "agent_panel_max_width")?;
        let sidebar_width = required::<_, E>(self.sidebar.width, "sidebar_width")?;
        let sidebar_min_width = required::<_, E>(self.sidebar.min_width, "sidebar_min_width")?;
        let sidebar_max_width = required::<_, E>(self.sidebar.max_width, "sidebar_max_width")?;
        let tab_max_width = required::<_, E>(self.tab_max_width, "tab_max_width")?;
        let terminal_padding = required::<_, E>(self.terminal_padding, "terminal_padding")?;
        let picker_width = required::<_, E>(self.picker_width, "picker_width")?;
        let dialog_width = required::<_, E>(self.dialog_width, "dialog_width")?;
        let settings_width = required::<_, E>(self.settings_width, "settings_width")?;
        let connection_editor_width =
            required(self.connection_editor_width, "connection_editor_width")?;
        let connection_editor_height =
            required(self.connection_editor_height, "connection_editor_height")?;
        let connection_nav_width =
            required::<_, E>(self.connection_nav_width, "connection_nav_width")?;
        let connection_description_height = required::<_, E>(
            self.connection_description_height,
            "connection_description_height",
        )?;
        let button_height = required::<_, E>(self.button_height, "button_height")?;
        let button_padding = required::<_, E>(self.button_padding, "button_padding")?;
        let local_terminal_height =
            required::<_, E>(self.local_terminal_height, "local_terminal_height")?;
        let status_bar_center_max =
            required::<_, E>(self.status_bar_center_max, "status_bar_center_max")?;
        Ok(Layout {
            agent_panel: WidthBounds {
                width: agent_panel_width,
                min_width: agent_panel_min_width,
                max_width: agent_panel_max_width,
            },
            sidebar: WidthBounds {
                width: sidebar_width,
                min_width: sidebar_min_width,
                max_width: sidebar_max_width,
            },
            tab_max_width,
            terminal_padding,
            picker_width,
            dialog_width,
            settings_width,
            connection_editor_width,
            connection_editor_height,
            connection_nav_width,
            connection_description_height,
            button_height,
            button_padding,
            local_terminal_height,
            status_bar_center_max,
        })
    }
}
struct TokenVisitor;
impl<'de> Visitor<'de> for TokenVisitor {
    type Value = Layout;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("struct Layout")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = Fields::default();
        while let Some(key) = map.next_key_seed(NameSeed::new(NAMES))? {
            if !(fields.read0(key, &mut map)?
                || fields.read1(key, &mut map)?
                || fields.read2(key, &mut map)?
                || fields.read3(key, &mut map)?)
            {
                return Err(de::Error::unknown_field(key, NAMES));
            }
        }
        fields.finish()
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut fields = Fields::default();
        let expected = "struct Layout with 19 elements";
        fields.agent_panel.width = Some(next(&mut sequence, 0, &expected)?);
        fields.agent_panel.min_width = Some(next(&mut sequence, 1, &expected)?);
        fields.agent_panel.max_width = Some(next(&mut sequence, 2, &expected)?);
        fields.sidebar.width = Some(next(&mut sequence, 3, &expected)?);
        fields.sidebar.min_width = Some(next(&mut sequence, 4, &expected)?);
        fields.sidebar.max_width = Some(next(&mut sequence, 5, &expected)?);
        fields.tab_max_width = Some(next(&mut sequence, 6, &expected)?);
        fields.terminal_padding = Some(next(&mut sequence, 7, &expected)?);
        fields.picker_width = Some(next(&mut sequence, 8, &expected)?);
        fields.dialog_width = Some(next(&mut sequence, 9, &expected)?);
        fields.settings_width = Some(next(&mut sequence, 10, &expected)?);
        fields.connection_editor_width = Some(next(&mut sequence, 11, &expected)?);
        fields.connection_editor_height = Some(next(&mut sequence, 12, &expected)?);
        fields.connection_nav_width = Some(next(&mut sequence, 13, &expected)?);
        fields.connection_description_height = Some(next(&mut sequence, 14, &expected)?);
        fields.button_height = Some(next(&mut sequence, 15, &expected)?);
        fields.button_padding = Some(next(&mut sequence, 16, &expected)?);
        fields.local_terminal_height = Some(next(&mut sequence, 17, &expected)?);
        fields.status_bar_center_max = Some(next(&mut sequence, 18, &expected)?);
        fields.finish()
    }
}
impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_struct("Layout", NAMES, TokenVisitor)
    }
}
