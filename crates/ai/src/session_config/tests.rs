use super::*;

fn choice(id: &str, name: &str) -> ConfigChoice {
    ConfigChoice {
        id: ConfigChoiceId::new(id),
        name: name.into(),
        description: None,
        extensions: None,
    }
}
fn option(id: &str, name: &str, category: ConfigCategory, kind: ConfigKind) -> ConfigOption {
    ConfigOption {
        id: ConfigId::new(id),
        name: name.into(),
        description: None,
        category: Some(category),
        kind,
        extensions: None,
    }
}
fn options() -> Vec<ConfigOption> {
    let mut opus = choice("opus", "Opus");
    opus.description = Some("Slower".into());
    vec![
        option(
            "model",
            "Model",
            ConfigCategory::Model,
            ConfigKind::Select {
                current: ConfigChoiceId::new("sonnet"),
                choices: SelectChoices::Groups(vec![
                    ConfigGroup {
                        id: ConfigGroupId::new("fast"),
                        name: "Fast".into(),
                        choices: vec![choice("sonnet", "Sonnet")],
                        extensions: None,
                    },
                    ConfigGroup {
                        id: ConfigGroupId::new("deep"),
                        name: "Deep".into(),
                        choices: vec![opus],
                        extensions: None,
                    },
                ]),
            },
        ),
        option(
            "effort",
            "Effort",
            ConfigCategory::ThoughtLevel,
            ConfigKind::Select {
                current: ConfigChoiceId::new("low"),
                choices: SelectChoices::Flat(vec![choice("low", "Low"), choice("high", "High")]),
            },
        ),
        option(
            "mode",
            "Mode",
            ConfigCategory::Mode,
            ConfigKind::Select {
                current: ConfigChoiceId::new("ask"),
                choices: SelectChoices::Flat(vec![choice("ask", "Ask"), choice("auto", "Auto")]),
            },
        ),
        option(
            "web",
            "Web search",
            ConfigCategory::Other("_tools".into()),
            ConfigKind::Boolean(false),
        ),
    ]
}

fn value(value: &str) -> ConfigValue {
    ConfigValue::Value(value.into())
}

#[test]
fn stored_options_restore_as_the_agent_reported_them() {
    let stored = store(&options());
    assert_eq!(restore(&stored), options());
    let text = toml::to_string(&BTreeMap::from([("agent", stored.clone())])).unwrap();
    let loaded: BTreeMap<String, Vec<StoredOption>> = toml::from_str(&text).unwrap();
    assert_eq!(loaded["agent"], stored);
}

#[test]
fn a_choice_shows_at_once_only_when_the_option_offers_it() {
    let mut options = options();
    assert!(choose(&mut options, "model", &value("opus")));
    assert!(choose(&mut options, "web", &ConfigValue::Flag(true)));
    assert!(!choose(&mut options, "model", &value("gpt")));
    assert!(!choose(&mut options, "missing", &value("opus")));
    assert!(!choose(&mut options, "web", &value("opus")));
    assert_eq!(
        store(&options)
            .iter()
            .map(|option| option.current.clone())
            .collect::<Vec<_>>(),
        [
            value("opus"),
            value("low"),
            value("ask"),
            ConfigValue::Flag(true)
        ]
    );
}

#[test]
fn an_open_session_receives_only_the_choices_it_lacks_in_option_order() {
    let choices = Choices::from([
        ("effort".into(), value("high")),
        ("model".into(), value("opus")),
        ("mode".into(), value("ask")),
        ("web".into(), value("on")),
        ("gone".into(), value("x")),
    ]);
    let requests = requests(&options(), &choices);
    assert_eq!(
        requests,
        [
            (ConfigId::new("model"), value("opus")),
            (ConfigId::new("effort"), value("high")),
        ]
    );
    let mut applied = options();
    for (id, value) in &requests {
        choose(&mut applied, id.as_str(), value);
    }
    assert!(super::requests(&applied, &choices).is_empty());
}

#[test]
fn only_model_and_effort_carry_over_to_new_chats() {
    let remembered = options()
        .iter()
        .filter(|option| remembered(option))
        .map(|option| option.id.to_string())
        .collect::<Vec<_>>();
    assert_eq!(remembered, ["model", "effort"]);
}

#[test]
fn values_round_trip_through_unchanged_preference_grammar() {
    for value in [value("opus"), ConfigValue::Flag(true)] {
        assert_eq!(
            serde_json::from_slice::<ConfigValue>(&serde_json::to_vec(&value).unwrap()).unwrap(),
            value
        );
    }
}

#[test]
fn a_chat_keeps_every_current_value_and_shows_its_choices_over_reported_options() {
    let current = current(&options());
    assert_eq!(current.get("model"), Some(&value("sonnet")));
    assert_eq!(current.get("mode"), Some(&value("ask")));
    assert_eq!(current.get("web"), Some(&ConfigValue::Flag(false)));
    let choices = Choices::from([
        ("model".into(), value("opus")),
        ("mode".into(), value("auto")),
        ("effort".into(), value("unknown")),
    ]);
    let shown = super::current(&with_choices(options(), &choices));
    assert_eq!(shown.get("model"), Some(&value("opus")));
    assert_eq!(shown.get("mode"), Some(&value("auto")));
    assert_eq!(
        shown.get("effort"),
        Some(&value("low")),
        "a value the agent does not offer is not shown"
    );
}
