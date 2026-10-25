use alloc::{format, string::ToString};

use nocterm_plugin_model::{
    AutoPlace, ContributionKind, FieldEdit, FieldKind, Icon, SettingValue, Zone as ModelZone, apply,
};
use proptest::prelude::*;

use super::support::{EXAMPLE, dto, example, host, manifest};
use crate::{
    API_VERSION, ApiVersion, ContributionError, DeclareError, HostContext, LocalId, Manifest,
    ManifestDto, ManifestError, ManifestHeader, OptionalCapability, Runtime, SettingsError,
};

fn refused(change: impl FnOnce(&mut ManifestDto)) -> ManifestError {
    let mut dto = example();
    change(&mut dto);
    Manifest::validate(&dto, &host()).unwrap_err()
}

#[test]
fn a_valid_package_becomes_a_clamped_user_plugin() {
    let manifest = manifest();
    let plugin = manifest.plugin();
    assert_eq!(plugin.id().as_str(), "user-acme-clock");
    assert_eq!(plugin.name(), "Acme Clock");
    assert_eq!(manifest.publisher(), "acme");
    let face = &plugin.contributions()[0];
    assert_eq!(face.id().as_str(), "user-acme-clock.face");
    assert_eq!(face.kind(), ContributionKind::Widget);
    assert_eq!(face.icon(), Icon::Puzzle);
    let placement = face.placement();
    assert_eq!(placement.auto, AutoPlace::Tray);
    assert_eq!(placement.zone, ModelZone::Right);
    assert!(placement.group.is_none() && !face.pinned() && !face.expandable());
    assert_eq!(manifest.required()[0].id, "clock.now");
    assert_eq!(manifest.optional()[0].id(), "monitor.usage");
    assert_eq!(manifest.reason("clock.now"), Some("To show the time"));
    assert!(manifest.view("face").is_some() && manifest.view("other").is_none());
}

#[test]
fn a_package_for_a_newer_api_is_told_so_before_its_keys_are_read() {
    let newer = "api = \"9.0\"\nbrand-new-key = true\n";
    assert!(toml::from_str::<ManifestDto>(newer).is_err());
    let header: ManifestHeader = toml::from_str(newer).unwrap();
    let error = header.check(API_VERSION).unwrap_err();
    assert_eq!(
        error,
        ManifestError::RequiresNewer {
            api: ApiVersion::new(9, 0)
        }
    );
    assert_eq!(error.to_string(), "Requires a newer Nocterm (API 9.0)");
    let from_dto = refused(|dto| dto.api = "0.2".into());
    assert!(matches!(from_dto, ManifestError::RequiresNewer { .. }));
    assert!(matches!(
        refused(|dto| dto.api = "x".into()),
        ManifestError::Version(_)
    ));
}

#[test]
fn unknown_keys_are_refused_within_the_declared_api() {
    let extra = EXAMPLE.replace(
        "name = \"Acme Clock\"",
        "name = \"Acme Clock\"\nwebsite = \"x\"",
    );
    assert!(toml::from_str::<ManifestDto>(&extra).is_err());
    let kind = EXAMPLE.replace("kind = \"widget\"", "kind = \"gadget\"");
    assert!(toml::from_str::<ManifestDto>(&kind).is_err());
    let runtime = EXAMPLE.replace("api = \"0.1\"", "api = \"0.1\"\nruntime = \"lua\"");
    assert!(toml::from_str::<ManifestDto>(&runtime).is_err());
}

#[test]
fn ids_are_built_by_the_host_from_plain_parts() {
    for (publisher, id) in [
        ("Acme", "clock"),
        ("acme", "clo-ck"),
        ("", "clock"),
        ("acme", ""),
    ] {
        let error = refused(|dto| {
            dto.publisher = publisher.into();
            dto.id = id.into();
        });
        assert!(matches!(error, ManifestError::Id(_)), "{publisher}/{id}");
    }
    let long = refused(|dto| dto.publisher = "a".repeat(30));
    assert!(matches!(long, ManifestError::Id(_)));
}

#[test]
fn names_are_plain_and_never_imitate_the_app() {
    for name in [
        "Nocterm",
        "NOCTERM ",
        "files",
        " Status Bar",
        "Évil",
        "<b>x</b>",
        "",
        "x\u{202E}y",
        &"n".repeat(49),
    ] {
        let error = refused(|dto| dto.name = name.into());
        assert!(matches!(error, ManifestError::Name(_)), "{name:?}");
    }
}

#[test]
fn a_title_is_plain_ascii_and_never_imitates_the_app() {
    for title in [
        "Files",
        " NOCTERM ",
        "Contai\u{200B}ners",
        "Con\u{2060}tainers",
        "\u{0421}ontainers",
        "Fil\u{0435}s",
        "Évil",
        "<b>x</b>",
        "x\u{202E}y",
        &"t".repeat(49),
    ] {
        let error = refused(|dto| dto.contributions[0].title = title.into());
        assert!(
            matches!(
                error,
                ManifestError::Contribution {
                    source: ContributionError::Title,
                    ..
                }
            ),
            "{title:?}"
        );
    }
    let mut dto = example();
    dto.contributions[0].title = "Acme-Clock v1.2_beta".into();
    assert!(Manifest::validate(&dto, &host()).is_ok());
}

#[test]
fn a_declarative_package_cannot_ask_for_an_effect() {
    for id in ["notifications.post", "storage.set", "panels.set-open"] {
        let required = refused(|dto| dto.capabilities.push(id.into()));
        assert_eq!(
            required,
            ManifestError::Declare(DeclareError::EffectInDeclarative(id.into())),
            "{id}"
        );
        let optional = refused(|dto| dto.optional_capabilities.push(id.into()));
        assert_eq!(
            optional,
            ManifestError::Declare(DeclareError::EffectInDeclarative(id.into())),
            "{id}"
        );
    }
    let mut dto = example();
    dto.optional_capabilities.push("notifications.own".into());
    assert!(Manifest::validate(&dto, &host()).is_ok());
}

#[test]
fn a_host_driving_a_stand_in_runtime_may_list_effects() {
    let mut dto = example();
    dto.capabilities.push("notifications.post".into());
    let stand_in = host().with_stand_in_runtime();
    assert!(Manifest::validate(&dto, &stand_in).is_ok());
}

const CODE: &str = r#"
api = "0.1"
runtime = "wasm"
publisher = "acme"
id = "timer"
name = "Acme Timer"
capabilities = ["clock.now", "panels.set-open", "storage.set"]

[[contribution]]
local = "bar"
kind = "widget"
title = "Timer"

[[contribution]]
local = "panel"
kind = "panel"
title = "Timer Panel"
"#;

fn code_dto() -> ManifestDto {
    dto(CODE)
}

#[test]
fn a_code_package_may_ask_for_effects_and_has_no_templates() {
    let manifest = Manifest::validate(&code_dto(), &host()).unwrap();
    assert_eq!(manifest.runtime(), Runtime::Wasm);
    assert_eq!(manifest.required().len(), 3);
    assert!(manifest.views().next().is_none());
    assert!(manifest.view("bar").is_none());
    assert!(manifest.has_contribution(&LocalId::parse("bar").unwrap()));
    assert!(manifest.has_contribution(&LocalId::parse("panel").unwrap()));
    assert!(!manifest.has_contribution(&LocalId::parse("other").unwrap()));
}

#[test]
fn a_declarative_package_has_a_view_per_contribution() {
    let manifest = manifest();
    assert!(manifest.has_contribution(&LocalId::parse("face").unwrap()));
    assert!(!manifest.has_contribution(&LocalId::parse("bar").unwrap()));
    let mut dto = example();
    dto.contributions[0].view = None;
    let error = Manifest::validate(&dto, &host()).unwrap_err();
    assert!(matches!(
        error,
        ManifestError::Contribution {
            source: ContributionError::NoView,
            ..
        }
    ));
}

#[test]
fn a_code_package_cannot_give_a_contribution_a_template() {
    let mut dto = code_dto();
    dto.contributions[0].view = example().contributions[0].view.clone();
    let error = Manifest::validate(&dto, &host()).unwrap_err();
    assert!(matches!(
        error,
        ManifestError::Contribution {
            source: ContributionError::ViewInCode,
            ..
        }
    ));
}

#[test]
fn a_code_package_declares_each_contribution_once() {
    let mut dto = code_dto();
    dto.contributions[1].local = "bar".into();
    let error = Manifest::validate(&dto, &host()).unwrap_err();
    assert!(matches!(
        error,
        ManifestError::Contribution {
            source: ContributionError::Duplicate,
            ..
        }
    ));
}

#[test]
fn the_runtime_names_are_the_authors_words() {
    assert_eq!(
        dto(&CODE.replace("wasm", "declarative")).runtime,
        Runtime::Declarative
    );
    assert_eq!(code_dto().runtime, Runtime::Wasm);
    let unknown = CODE.replace("\"wasm\"", "\"lua\"");
    assert!(toml::from_str::<ManifestDto>(&unknown).is_err());
}

#[test]
fn capabilities_resolve_against_the_table() {
    let unknown = refused(|dto| dto.capabilities.push("sys.exec".into()));
    assert_eq!(
        unknown,
        ManifestError::Declare(DeclareError::UnknownRequired("sys.exec".into()))
    );
    let twice = refused(|dto| dto.optional_capabilities.push("clock.now".into()));
    assert_eq!(
        twice,
        ManifestError::Declare(DeclareError::Duplicate("clock.now".into()))
    );
    let stray = refused(|dto| {
        dto.reasons.insert("workspace.kind".into(), "why".into());
    });
    assert_eq!(
        stray,
        ManifestError::Declare(DeclareError::ReasonFor("workspace.kind".into()))
    );
    assert!(matches!(
        refused(|dto| dto.capabilities.push("Bad Id".into())),
        ManifestError::Declare(DeclareError::Syntax(_))
    ));
    assert_eq!(
        refused(|dto| dto.capabilities = (0..17).map(|i| alloc::format!("a.b{i}")).collect()),
        ManifestError::Declare(DeclareError::TooMany)
    );
}

#[test]
fn an_unknown_optional_capability_is_kept_as_unavailable() {
    let mut dto = example();
    dto.optional_capabilities.push("hologram.show".into());
    let manifest = Manifest::validate(&dto, &host()).unwrap();
    assert!(
        matches!(&manifest.optional()[1], OptionalCapability::Unknown(id) if id == "hologram.show")
    );
}

#[test]
fn reasons_are_sanitized_and_bounded() {
    let mut dto = example();
    dto.reasons.insert(
        "clock.now".into(),
        alloc::format!("a\u{202E}b{}", "z".repeat(300)),
    );
    let manifest = Manifest::validate(&dto, &host()).unwrap();
    let reason = manifest.reason("clock.now").unwrap();
    assert!(reason.starts_with("ab") && reason.chars().count() == 160);
}

#[test]
fn templates_may_only_name_what_the_package_declares() {
    let undeclared = refused(|dto| {
        dto.contributions[0].view.as_mut().unwrap().nodes[2].text =
            Some("{workspace.kind.connected}".into());
    });
    assert!(matches!(
        undeclared,
        ManifestError::Contribution {
            source: ContributionError::View(_),
            ..
        }
    ));
    let mut dto = example();
    dto.contributions[0].view.as_mut().unwrap().nodes[2].text =
        Some("{monitor.usage.cpu_percent|round(0)}".into());
    assert!(
        Manifest::validate(&dto, &host()).is_ok(),
        "an optional capability is declared"
    );
    let missing = refused(|dto| {
        dto.contributions[0].view.as_mut().unwrap().nodes[2].text =
            Some("{settings.nothing}".into());
    });
    assert!(matches!(missing, ManifestError::Contribution { .. }));
}

#[test]
fn contributions_are_counted_named_and_unique() {
    assert_eq!(
        refused(|dto| dto.contributions.clear()),
        ManifestError::Contributions
    );
    assert_eq!(
        refused(|dto| dto.contributions = (0..9).map(|_| dto_first()).collect()),
        ManifestError::Contributions
    );
    let twice = refused(|dto| dto.contributions.push(dto_first()));
    assert!(matches!(
        twice,
        ManifestError::Contribution {
            source: ContributionError::Duplicate,
            ..
        }
    ));
    let bad = refused(|dto| dto.contributions[0].local = "Face".into());
    assert!(matches!(
        bad,
        ManifestError::Contribution {
            source: ContributionError::Local(_),
            ..
        }
    ));
    let no_kind = refused(|dto| dto.contributions[0].kind = None);
    assert!(matches!(
        no_kind,
        ManifestError::Contribution {
            source: ContributionError::NoKind,
            ..
        }
    ));
    let empty = refused(|dto| dto.contributions[0].title = "  ".into());
    assert!(matches!(
        empty,
        ManifestError::Contribution {
            source: ContributionError::Title,
            ..
        }
    ));
}

fn dto_first() -> crate::ContributionDto {
    example().contributions.remove(0)
}

#[test]
fn a_user_contribution_cannot_ask_for_a_host_icon() {
    let manifest = manifest();
    assert_eq!(manifest.plugin().contributions()[0].icon(), Icon::Puzzle);
    for host_icon in [
        "shield-check",
        "activity",
        "clock",
        "container",
        "layers",
        "scroll-text",
    ] {
        let text = EXAMPLE.replace(
            "icon = \"puzzle\"\n\n[contribution.view]",
            &format!("icon = \"{host_icon}\"\n\n[contribution.view]"),
        );
        assert!(toml::from_str::<ManifestDto>(&text).is_err(), "{host_icon}");
    }
}

const SETTINGS: &str = r#"
api = "0.1"
publisher = "acme"
id = "all"
name = "All kinds"
[[contribution]]
local = "a"
kind = "panel"
title = "A"
[contribution.view]
nodes = [{ kind = "divider" }]

[settings]
version = 3

[[settings.group]]
title = "Plugin\u202E"

[[settings.group.field]]
key = "on"
label = "On"
kind = "toggle"
default = true

[[settings.group.field]]
key = "mode"
label = "Mode"
help = "How"
kind = "choice"
default = "fast"
options = [{ key = "fast", label = "Fast" }, { key = "slow", label = "Slow", help = "h" }]

[[settings.group.field]]
key = "tags"
label = "Tags"
kind = "multi-choice"
ordered = true
max_chosen = 2
default = ["fast"]
options = [{ key = "fast", label = "Fast" }, { key = "slow", label = "Slow" }]

[[settings.group.field]]
key = "size"
label = "Size"
kind = "number"
min = 1
max = 9
step = 2
default = 3
unit = "px"
renamed_from = "dimension"

[[settings.group]]
title = "Panel"
contribution = "a"

[[settings.group.field]]
key = "name"
label = "Name"
kind = "text"
max_len = 20
default = "x"
placeholder = "type"
"#;

#[test]
fn the_settings_table_maps_every_kind_onto_the_shared_schema() {
    let manifest = Manifest::validate(&dto(SETTINGS), &host()).unwrap();
    let schema = manifest.plugin().settings();
    assert_eq!(schema.version, 3);
    assert_eq!(schema.groups[0].title, "Plugin");
    assert!(schema.groups[0].contribution.is_none());
    assert_eq!(
        schema.groups[1].contribution.as_ref().unwrap().as_str(),
        "user-acme-all.a"
    );
    assert!(matches!(
        schema.field("on").unwrap().kind,
        FieldKind::Toggle { default: true }
    ));
    assert!(matches!(
        schema.field("mode").unwrap().kind,
        FieldKind::Choice { .. }
    ));
    assert!(matches!(
        schema.field("tags").unwrap().kind,
        FieldKind::MultiChoice {
            ordered: true,
            max: 2,
            ..
        }
    ));
    assert!(matches!(
        schema.field("size").unwrap().kind,
        FieldKind::Number {
            min: 1,
            max: 9,
            step: 2,
            default: 3,
            ..
        }
    ));
    assert_eq!(
        schema.field("size").unwrap().renamed_from.as_deref(),
        Some("dimension")
    );
    assert!(matches!(
        schema.field("name").unwrap().kind,
        FieldKind::Text { max_len: 20, .. }
    ));
    assert_eq!(schema.field("mode").unwrap().help.as_deref(), Some("How"));
}

#[test]
fn settings_mistakes_are_manifest_errors() {
    let with = |from: &str, to: &str| {
        let mut dto = dto(&SETTINGS.replace(from, to));
        dto.contributions[0].title = "A".into();
        Manifest::validate(&dto, &host()).unwrap_err()
    };
    assert!(matches!(
        with(
            "kind = \"toggle\"\ndefault = true",
            "kind = \"toggle\"\ndefault = true\nmin = 1"
        ),
        ManifestError::Settings(SettingsError::Stray { field: "min", .. })
    ));
    assert!(matches!(
        with("default = \"fast\"\noptions", "default = \"gone\"\noptions"),
        ManifestError::Plugin(_)
    ));
    assert!(matches!(
        with("max_chosen = 2\n", ""),
        ManifestError::Settings(SettingsError::Missing {
            field: "max_chosen",
            ..
        })
    ));
    assert!(matches!(
        with("default = true", "default = 5"),
        ManifestError::Settings(SettingsError::Default { .. })
    ));
    assert!(matches!(
        with("contribution = \"a\"", "contribution = \"Z!\""),
        ManifestError::Settings(SettingsError::Group { .. })
    ));
    assert!(matches!(
        with("contribution = \"a\"", "contribution = \"zz\""),
        ManifestError::Plugin(_)
    ));
    assert!(matches!(
        with("key = \"size\"", "key = \"on\""),
        ManifestError::Plugin(_)
    ));
    assert!(matches!(
        with("max_len = 20", "max_len = 999"),
        ManifestError::Plugin(_)
    ));
}

proptest! {
    #[test]
    fn validation_never_panics_on_arbitrary_text(name in ".{0,80}", title in ".{0,80}", template in ".{0,120}") {
        let mut dto = example();
        dto.name = name;
        dto.contributions[0].title = title;
        dto.contributions[0].view.as_mut().unwrap().nodes[2].text = Some(template);
        let _ = Manifest::validate(&dto, &HostContext::default());
    }
}

fn arbitrary_edit() -> impl Strategy<Value = (&'static str, FieldEdit)> {
    let key = prop_oneof![
        Just("on"),
        Just("mode"),
        Just("tags"),
        Just("size"),
        Just("name"),
        Just("nope"),
    ];
    let option = prop_oneof![Just("fast"), Just("slow"), Just("other")].prop_map(str::to_string);
    let value = prop_oneof![
        any::<bool>().prop_map(SettingValue::Bool),
        option.clone().prop_map(SettingValue::Choice),
        proptest::collection::vec(option.clone(), 0..4).prop_map(SettingValue::Choices),
        prop_oneof![any::<i32>(), -2i32..12].prop_map(SettingValue::Number),
        ".{0,40}".prop_map(SettingValue::Text),
    ];
    let edit = prop_oneof![
        value.prop_map(FieldEdit::Set),
        (option, any::<bool>()).prop_map(|(option, on)| FieldEdit::Toggle { option, on }),
        any::<bool>().prop_map(|up| FieldEdit::Step { up }),
    ];
    (key, edit)
}

proptest! {
    #[test]
    fn edits_of_a_packages_settings_keep_every_value_inside_its_field(
        edits in proptest::collection::vec(arbitrary_edit(), 0..24),
    ) {
        let manifest = Manifest::validate(&dto(SETTINGS), &host()).unwrap();
        let schema = manifest.plugin().settings();
        let mut values = schema.defaults();
        for (key, edit) in edits {
            if let Ok(value) = apply(schema, &values, key, edit) {
                values.insert(key, value);
            }
            for field in schema.fields() {
                let value = field.current(&values);
                match (&field.kind, value) {
                    (FieldKind::Toggle { .. }, SettingValue::Bool(_)) => {}
                    (FieldKind::Choice { options, .. }, SettingValue::Choice(chosen)) => {
                        prop_assert!(options.iter().any(|option| option.key == chosen));
                    }
                    (FieldKind::MultiChoice { options, max, .. }, SettingValue::Choices(chosen)) => {
                        prop_assert!(chosen.len() <= usize::from(*max));
                        for one in &chosen {
                            prop_assert!(options.iter().any(|option| option.key == *one));
                        }
                    }
                    (FieldKind::Number { min, max, .. }, SettingValue::Number(number)) => {
                        prop_assert!((*min..=*max).contains(&number));
                    }
                    (FieldKind::Text { max_len, .. }, SettingValue::Text(text)) => {
                        prop_assert!(text.chars().count() <= usize::from(*max_len));
                    }
                    (kind, value) => prop_assert!(false, "{kind:?} holds {value:?}"),
                }
            }
        }
    }
}
