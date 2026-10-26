use super::{fixtures::*, *};
use alloc::{format, string::String, vec, vec::Vec};

fn set(
    schema: &SettingsSchema,
    key: &str,
    value: SettingValue,
) -> Result<SettingValue, FieldError> {
    apply(schema, &schema.defaults(), key, FieldEdit::Set(value))
}

fn toggled(
    schema: &SettingsSchema,
    current: &SettingValues,
    option: &str,
    on: bool,
) -> Result<SettingValue, FieldError> {
    let edit = FieldEdit::Toggle {
        option: option.into(),
        on,
    };
    apply(schema, current, "tags", edit)
}

fn chosen(list: &[&str]) -> SettingValue {
    SettingValue::Choices(list.iter().map(ToString::to_string).collect())
}

#[test]
fn each_kind_accepts_a_valid_value() {
    let schema = sample();
    let cases = [
        ("on", SettingValue::Bool(false)),
        ("mode", SettingValue::Choice("slow".into())),
        ("tags", chosen(&["b", "a"])),
        ("count", SettingValue::Number(10)),
        ("city", SettingValue::Text("Bergen".into())),
    ];
    for (key, value) in cases {
        assert_eq!(set(&schema, key, value.clone()), Ok(value), "{key}");
    }
}

#[test]
fn an_edit_of_an_unknown_key_or_the_wrong_kind_is_refused() {
    let schema = sample();
    assert_eq!(
        set(&schema, "missing", SettingValue::Bool(true)),
        Err(FieldError::UnknownKey("missing".into()))
    );
    assert_eq!(
        set(&schema, "on", SettingValue::Number(1)),
        Err(FieldError::WrongKind { key: "on".into() })
    );
    let toggle = FieldEdit::Toggle {
        option: "x".into(),
        on: true,
    };
    assert_eq!(
        apply(&schema, &schema.defaults(), "mode", toggle),
        Err(FieldError::WrongKind { key: "mode".into() })
    );
}

#[test]
fn an_option_outside_the_list_is_refused() {
    let schema = sample();
    let unknown = |key: &str| FieldError::UnknownOption {
        key: key.into(),
        option: "zzz".into(),
    };
    assert_eq!(
        set(&schema, "mode", SettingValue::Choice("zzz".into())),
        Err(unknown("mode"))
    );
    assert_eq!(
        set(&schema, "tags", chosen(&["a", "zzz"])),
        Err(unknown("tags"))
    );
    assert_eq!(
        toggled(&schema, &schema.defaults(), "zzz", true),
        Err(unknown("tags"))
    );
}

#[test]
fn a_number_outside_its_range_is_refused() {
    let schema = sample();
    for number in [1, 11] {
        assert_eq!(
            set(&schema, "count", SettingValue::Number(number)),
            Err(FieldError::OutOfRange {
                key: "count".into(),
                min: 2,
                max: 10
            })
        );
    }
}

#[test]
fn a_set_of_options_drops_repeats_and_respects_the_maximum() {
    let schema = sample();
    assert_eq!(
        set(&schema, "tags", chosen(&["b", "b", "a"])),
        Ok(chosen(&["b", "a"]))
    );
    assert_eq!(
        set(&schema, "tags", chosen(&["a", "b", "c"])),
        Err(FieldError::TooMany {
            key: "tags".into(),
            max: 2
        })
    );
}

#[test]
fn toggling_applies_to_the_current_value_so_two_edits_both_land() {
    let schema = sample();
    let mut current = schema.defaults();
    for option in ["b", "c"] {
        let next = toggled(&schema, &current, option, true);
        if option == "b" {
            current.insert("tags", next.unwrap());
        } else {
            assert_eq!(
                next,
                Err(FieldError::TooMany {
                    key: "tags".into(),
                    max: 2
                })
            );
        }
    }
    assert_eq!(current.get("tags"), Some(&chosen(&["a", "b"])));
    let off = toggled(&schema, &current, "a", false).unwrap();
    assert_eq!(off, chosen(&["b"]));
}

#[test]
fn toggling_an_option_to_the_state_it_is_in_changes_nothing() {
    let schema = sample();
    let mut current = schema.defaults();
    current.insert("tags", chosen(&["b", "a"]));
    assert_eq!(
        toggled(&schema, &current, "b", true),
        Ok(chosen(&["b", "a"]))
    );
    assert_eq!(
        toggled(&schema, &current, "c", false),
        Ok(chosen(&["b", "a"]))
    );
}

#[test]
fn text_is_sanitized_before_its_length_is_checked() {
    let schema = sample();
    assert_eq!(
        set(
            &schema,
            "city",
            SettingValue::Text("Os\nlo\u{202E}\u{200B}".into())
        ),
        Ok(SettingValue::Text("Oslo".into()))
    );
    // Twelve characters, nine of them after sanitizing, against a limit of eight.
    let too_long = SettingValue::Text("Bergen\u{200B}\u{200B}\u{200B}abc".into());
    assert_eq!(
        set(&schema, "city", too_long),
        Err(FieldError::TooLong {
            key: "city".into(),
            max: 8
        })
    );
    // Controls inside the text do not count against the limit.
    let padded = SettingValue::Text("Bergen\n\n\n\n\n\n\n".into());
    assert_eq!(
        set(&schema, "city", padded),
        Ok(SettingValue::Text("Bergen".into()))
    );
}

#[test]
fn the_error_text_is_meant_to_be_shown_under_the_field() {
    let range = FieldError::OutOfRange {
        key: "count".into(),
        min: 2,
        max: 10,
    };
    assert_eq!(range.to_string(), "Enter a whole number from 2 to 10.");
}

#[test]
fn sanitizing_strips_controls_bidi_and_zero_width_characters() {
    let raw = "a\u{0}\u{7}b\nc\td\u{85}e\u{9F}f\u{2028}g\u{2029}h\u{FEFF}i\u{200B}j\u{200C}k\u{200D}l\
               \u{2060}m\u{202A}n\u{202E}o\u{2066}p\u{2069}q\u{061C}r\u{200E}s";
    assert_eq!(sanitize_text(raw), "abcdefghijklmnopqrs");
}

#[test]
fn sanitizing_strips_every_default_ignorable_code_point() {
    let ignorable = [
        '\u{00AD}',
        '\u{034F}',
        '\u{115F}',
        '\u{1160}',
        '\u{17B4}',
        '\u{17B5}',
        '\u{180B}',
        '\u{180C}',
        '\u{180D}',
        '\u{180E}',
        '\u{180F}',
        '\u{2065}',
        '\u{206A}',
        '\u{206F}',
        '\u{3164}',
        '\u{FE00}',
        '\u{FE0F}',
        '\u{FFA0}',
        '\u{FFF0}',
        '\u{FFF8}',
        '\u{FFF9}',
        '\u{FFFA}',
        '\u{FFFB}',
        '\u{1BCA0}',
        '\u{1BCA3}',
        '\u{1D173}',
        '\u{1D17A}',
        '\u{E0000}',
        '\u{E0001}',
        '\u{E0020}',
        '\u{E007F}',
        '\u{E0100}',
        '\u{E01EF}',
        '\u{E0FFF}',
    ];
    for character in ignorable {
        let raw = format!("a{character}b");
        assert_eq!(sanitize_text(&raw), "ab", "U+{:04X}", character as u32);
    }
}

#[test]
fn hidden_tag_characters_and_variation_selectors_do_not_survive() {
    let tags: String = ('\u{E0020}'..='\u{E007E}').collect();
    assert_eq!(sanitize_text(&format!("City{tags}")), "City");
    assert_eq!(
        sanitize_text(&format!("Oslo{}", "\u{FE0F}".repeat(200))),
        "Oslo"
    );
}

#[test]
fn sanitizing_keeps_ordinary_text_and_spaces() {
    assert_eq!(
        sanitize_text("  São Paulo, 東京 😀 "),
        "  São Paulo, 東京 😀 "
    );
    assert_eq!(sanitize_text("<b>x</b> https://a"), "<b>x</b> https://a");
    assert_eq!(sanitize_text(""), "");
}

#[test]
fn sanitizing_keeps_at_most_two_combining_marks_per_base() {
    assert_eq!(
        sanitize_text("a\u{301}\u{302}\u{303}\u{304}b"),
        "a\u{301}\u{302}b"
    );
    // A new base resets the count.
    assert_eq!(
        sanitize_text("a\u{301}\u{302}b\u{301}\u{302}"),
        "a\u{301}\u{302}b\u{301}\u{302}"
    );
}

#[test]
fn every_nonspacing_and_enclosing_mark_counts_towards_the_cap() {
    // Enclosing, Cyrillic, Hebrew, Arabic, Devanagari and supplementary-plane nonspacing marks.
    for mark in [
        '\u{0489}',
        '\u{0483}',
        '\u{05B0}',
        '\u{064B}',
        '\u{0951}',
        '\u{1E8D0}',
    ] {
        let raw = format!("a{}b", mark.to_string().repeat(40));
        let kept = format!("a{mark}{mark}b");
        assert_eq!(sanitize_text(&raw), kept, "U+{:04X}", mark as u32);
    }
}

#[test]
fn spacing_marks_and_plain_letters_do_not_count_as_stacking() {
    assert_eq!(
        sanitize_text("\u{0915}\u{093E}\u{093E}\u{093E}"),
        "\u{0915}\u{093E}\u{093E}\u{093E}"
    );
    assert_eq!(sanitize_text(&"\u{0489}".repeat(3)), "\u{0489}\u{0489}");
}

fn numbers() -> SettingsSchema {
    schema_of(vec![field("level", number(1, 10, 3, 5))])
}

fn step(current: &SettingValues, up: bool) -> Result<SettingValue, FieldError> {
    apply(&numbers(), current, "level", FieldEdit::Step { up })
}

#[test]
fn a_step_starts_from_the_stored_value_or_the_default() {
    let mut stored = SettingValues::default();
    assert_eq!(step(&stored, true), Ok(SettingValue::Number(8)));
    assert_eq!(step(&stored, false), Ok(SettingValue::Number(2)));
    stored.insert("level", SettingValue::Number(2));
    assert_eq!(step(&stored, true), Ok(SettingValue::Number(5)));
}

#[test]
fn a_step_stops_at_the_bounds() {
    let mut stored = SettingValues::default();
    stored.insert("level", SettingValue::Number(9));
    assert_eq!(step(&stored, true), Ok(SettingValue::Number(10)));
    stored.insert("level", SettingValue::Number(10));
    assert_eq!(step(&stored, true), Ok(SettingValue::Number(10)));
    stored.insert("level", SettingValue::Number(2));
    assert_eq!(step(&stored, false), Ok(SettingValue::Number(1)));
}

#[test]
fn a_step_of_a_field_that_is_not_a_number_is_refused() {
    let schema = sample();
    assert_eq!(
        apply(
            &schema,
            &schema.defaults(),
            "on",
            FieldEdit::Step { up: true }
        ),
        Err(FieldError::WrongKind { key: "on".into() })
    );
}

#[test]
fn loaded_text_is_sanitized_and_cut_to_its_field_length() {
    let schema = schema_of(vec![
        field("city", text("", 8)),
        field("level", number(1, 10, 1, 5)),
    ]);
    let mut loaded = SettingValues::default();
    loaded.insert(
        "city",
        SettingValue::Text(format!("Os\u{202E}lo{}", "x".repeat(300))),
    );
    loaded.insert("level", SettingValue::Number(7));
    loaded.insert("other", SettingValue::Text("kept\u{202E}".into()));
    let clean = bounded(&schema, loaded);
    assert_eq!(
        clean.get("city"),
        Some(&SettingValue::Text("Oslo".to_string() + "xxxx"))
    );
    assert_eq!(clean.get("level"), Some(&SettingValue::Number(7)));
    assert_eq!(
        clean.get("other"),
        Some(&SettingValue::Text("kept\u{202E}".into()))
    );
}

fn letters() -> Vec<ChoiceOption> {
    options(&["a", "b", "c", "d"]).into_owned()
}

fn toggle(list: &[&str], option: &str, on: bool, ordered: bool) -> Vec<String> {
    let list: Vec<String> = list.iter().map(ToString::to_string).collect();
    toggle_choice(&letters(), &list, option, on, ordered)
}

#[test]
fn turning_an_option_on_keeps_a_sorted_list_sorted() {
    assert_eq!(toggle(&["a", "c"], "b", true, true), ["a", "b", "c"]);
    assert_eq!(toggle(&["b", "c"], "a", true, true), ["a", "b", "c"]);
    assert_eq!(toggle(&["a", "b"], "d", true, true), ["a", "b", "d"]);
}

#[test]
fn turning_an_option_on_appends_to_a_list_in_hand_ordered_sequence() {
    assert_eq!(toggle(&["c", "a"], "b", true, true), ["c", "a", "b"]);
}

#[test]
fn an_unordered_field_always_appends() {
    assert_eq!(toggle(&["a", "c"], "b", true, false), ["a", "c", "b"]);
}

#[test]
fn turning_an_option_off_removes_it_and_keeps_the_rest_in_order() {
    assert_eq!(toggle(&["c", "a", "b"], "a", false, true), ["c", "b"]);
    assert_eq!(toggle(&["a"], "z", false, true), ["a"]);
}
