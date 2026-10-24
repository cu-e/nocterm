//! Stable theme format behavior across internal token groups.
use super::*;

#[test]
fn grouped_tokens_keep_flat_required_keys_and_optional_omission() {
    let tokens = DesignTokens::builtin();
    let table: toml::Table = toml::to_string(&tokens).unwrap().parse().unwrap();
    let layout = table["layout"].as_table().unwrap();
    assert_eq!(layout.len(), 19);
    assert!(layout.contains_key("agent_panel_min_width"));
    assert!(!layout.contains_key("agent_panel"));
    let terminal = table["dark"]["terminal"].as_table().unwrap();
    assert!(terminal.contains_key("bright_white"));
    assert!(!terminal.contains_key("normal"));
    assert!(!terminal.contains_key("bright"));
    let mut missing = layout.clone();
    missing.remove("sidebar_width");
    assert!(toml::Value::Table(missing).try_into::<Layout>().is_err());
    let mut missing = terminal.clone();
    missing.remove("bright_white");
    assert!(
        toml::Value::Table(missing)
            .try_into::<TerminalColors>()
            .is_err()
    );
    let mut colors = tokens.dark.terminal;
    colors.background = None;
    colors.foreground = None;
    colors.cursor = None;
    colors.selection = None;
    let source = toml::to_string(&colors).unwrap();
    let table: toml::Table = source.parse().unwrap();
    assert_eq!(table.len(), 16);
    assert_eq!(toml::from_str::<TerminalColors>(&source).unwrap(), colors);
    colors.background = Some(Color::rgb(1, 2, 3));
    colors.foreground = Some(Color::rgb(4, 5, 6));
    colors.cursor = Some(Color::rgb(7, 8, 9));
    colors.selection = Some(Color::rgb(10, 11, 12));
    let source = toml::to_string(&colors).unwrap();
    assert_eq!(toml::from_str::<TerminalColors>(&source).unwrap(), colors);
}

#[test]
fn nested_internal_group_names_and_flat_typos_are_rejected() {
    for text in [
        "[layout]\nsidebar_widht = 20.0",
        "[layout.sidebar]\nwidth = 20.0",
        "[layout.agent_panel]\nwidth = 20.0",
        "[dark.terminal.normal]\nred = '#ff0000'",
        "[light.terminal.bright]\nwhite = '#ffffff'",
        "[dark.terminal]\nbright_gray = '#ffffff'",
    ] {
        assert!(DesignTokens::with_overrides(text).is_err(), "{text}");
    }
    assert!(
        DesignTokens::with_overrides("[layout]\nsidebar_width=20.0\nsidebar_width=22.0").is_err()
    );
}

#[test]
fn each_ansi_override_changes_only_its_existing_index() {
    let names = [
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
    let builtin = DesignTokens::builtin();
    for appearance in ["light", "dark"] {
        for (index, name) in names.iter().enumerate() {
            let tokens = DesignTokens::with_overrides(&format!(
                "[{appearance}.terminal]\n{name} = '#123456'"
            ))
            .unwrap();
            let mut expected = builtin.palette(appearance == "dark").terminal.ansi();
            expected[index] = Color::rgb(0x12, 0x34, 0x56);
            assert_eq!(
                tokens.palette(appearance == "dark").terminal.ansi(),
                expected
            );
        }
    }
}

#[test]
fn layout_generic_sequence_decode_keeps_original_field_order() {
    use serde::de::value::{Error, SeqDeserializer};
    let values: Vec<f32> = (1..=19).map(|value| value as f32).collect();
    let layout = Layout::deserialize(SeqDeserializer::<_, Error>::new(values.into_iter())).unwrap();
    assert_eq!(layout.agent_panel.width, 1.0);
    assert_eq!(layout.sidebar.max_width, 6.0);
    assert_eq!(layout.tab_max_width, 7.0);
    assert_eq!(layout.local_terminal_height, 18.0);
    assert_eq!(layout.status_bar_center_max, 19.0);
    let short = SeqDeserializer::<_, Error>::new([1.0_f32].into_iter());
    assert!(Layout::deserialize(short).is_err());
}

#[test]
fn public_token_schema_keeps_flat_properties_and_required_names() {
    let mut generator = SchemaGenerator::default();
    let layout = Layout::json_schema(&mut generator);
    assert_eq!(
        layout.as_value()["properties"].as_object().unwrap().len(),
        19
    );
    assert_eq!(layout.as_value()["required"].as_array().unwrap().len(), 19);
    assert_eq!(layout.as_value()["additionalProperties"], false);
    let terminal = TerminalColors::json_schema(&mut generator);
    assert_eq!(
        terminal.as_value()["properties"].as_object().unwrap().len(),
        20
    );
    assert_eq!(
        terminal.as_value()["required"].as_array().unwrap().len(),
        16
    );
    assert_eq!(terminal.as_value()["additionalProperties"], false);
    assert!(terminal.as_value()["properties"].get("normal").is_none());
    assert!(
        terminal.as_value()["properties"]
            .get("bright_black")
            .is_some()
    );
}

#[test]
fn every_flat_layout_override_retains_all_siblings() {
    let builtin = DesignTokens::builtin();
    let base: toml::Table = toml::to_string(&builtin.layout).unwrap().parse().unwrap();
    for (name, value) in &base {
        let original = value.as_float().unwrap();
        let changed = original + 0.01;
        let tokens =
            DesignTokens::with_overrides(&format!("[layout]\n{name} = {changed}")).unwrap();
        let actual: toml::Table = toml::to_string(&tokens.layout).unwrap().parse().unwrap();
        for (sibling, original) in &base {
            if sibling == name {
                assert_eq!(actual[sibling].as_float().unwrap() as f32, changed as f32);
            } else {
                assert_eq!(&actual[sibling], original);
            }
        }
    }
}

#[test]
fn generic_map_duplicate_fields_are_rejected_before_missing_fields() {
    use serde::de::value::{Error, MapDeserializer};
    let duplicate = [("sidebar_width", 20.0_f32), ("sidebar_width", 21.0_f32)];
    let error =
        Layout::deserialize(MapDeserializer::<_, Error>::new(duplicate.into_iter())).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("duplicate field `sidebar_width`")
    );
}

#[test]
fn truncated_generic_sequences_keep_native_struct_length_diagnostics() {
    use serde::de::value::{Error, SeqDeserializer};
    for length in 0..19 {
        let values = (0..length).map(|value| value as f32);
        let error = Layout::deserialize(SeqDeserializer::<_, Error>::new(values)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid length {length}, expected struct Layout with 19 elements")
        );
    }
    let empty = SeqDeserializer::<_, Error>::new(std::iter::empty::<f32>());
    let error = TerminalColors::deserialize(empty).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid length 4, expected struct TerminalColors with 20 elements"
    );
}

#[test]
fn scalar_struct_errors_keep_the_original_generic_expectation() {
    use serde::de::value::{Error, U64Deserializer};
    let layout = Layout::deserialize(U64Deserializer::<Error>::new(1)).unwrap_err();
    assert_eq!(
        layout.to_string(),
        "invalid type: integer `1`, expected struct Layout"
    );
    let terminal = TerminalColors::deserialize(U64Deserializer::<Error>::new(1)).unwrap_err();
    assert_eq!(
        terminal.to_string(),
        "invalid type: integer `1`, expected struct TerminalColors"
    );
}

#[test]
fn numeric_identifiers_keep_native_out_of_range_diagnostics() {
    use serde::de::value::{Error, MapDeserializer};
    for index in [19_u64, 20, u64::MAX] {
        let error = Layout::deserialize(MapDeserializer::<_, Error>::new(
            [(index, 0_f32)].into_iter(),
        ))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid value: integer `{index}`, expected field index 0 <= i < 19")
        );
    }
    for index in [20_u64, 21, u64::MAX] {
        let error = TerminalColors::deserialize(MapDeserializer::<_, Error>::new(
            [(index, 0_f32)].into_iter(),
        ))
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("invalid value: integer `{index}`, expected field index 0 <= i < 20")
        );
    }
}

#[test]
fn borrowed_byte_identifiers_retain_flat_names_and_lossy_unknown_diagnostics() {
    use serde::de::value::{BorrowedBytesDeserializer, Error, MapDeserializer};
    let builtin = DesignTokens::builtin().layout;
    let table: toml::Table = toml::to_string(&builtin).unwrap().parse().unwrap();
    let values = table.iter().map(|(key, value)| {
        (
            BorrowedBytesDeserializer::<Error>::new(key.as_bytes()),
            value.as_float().unwrap() as f32,
        )
    });
    let decoded = Layout::deserialize(MapDeserializer::<_, Error>::new(values)).unwrap();
    assert_eq!(decoded, builtin);
    let unknown = [(BorrowedBytesDeserializer::<Error>::new(b"\xff"), 0_f32)];
    let error =
        Layout::deserialize(MapDeserializer::<_, Error>::new(unknown.into_iter())).unwrap_err();
    assert!(error.to_string().starts_with("unknown field `�`"));
}
