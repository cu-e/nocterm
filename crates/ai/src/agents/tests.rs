use super::*;
use crate::{AgentServerSettings, AiSettings, env::sanitized_environment};

const ALPHA: AgentProfile = AgentProfile::new(
    "alpha",
    "Alpha",
    LaunchDefaults::new(
        "npx",
        &["-y", "@example/alpha-acp@1.0.0"],
        &["ALPHA_API_KEY"],
    ),
)
.recognized_by(&["alpha-acp"])
.writes_home(&[".alpha", ".config/alpha"])
.with_usage(
    UsageProfile::new(
        UsageMeta::RateLimitRecord {
            key: "_alpha/rateLimit",
        },
        AccountUsage::OAuthAccount,
    )
    .with_empty_hint("Alpha limits load from your sign-in."),
);
const BETA: AgentProfile =
    AgentProfile::new("beta", "Beta", LaunchDefaults::new("beta", &["acp"], &[]));

fn catalog() -> AgentCatalog {
    AgentCatalog::new([ALPHA, BETA]).unwrap()
}

fn custom(command: &str) -> AgentServerSettings {
    AgentServerSettings {
        command: Some(command.into()),
        args: Some(vec!["acp".into()]),
        ..Default::default()
    }
}

#[test]
fn settings_override_disable_and_extend_catalog_agents() {
    let mut settings = AiSettings::default();
    settings.agents.insert(
        "beta".into(),
        AgentServerSettings {
            enabled: false,
            ..Default::default()
        },
    );
    settings
        .agents
        .insert("custom".into(), custom("/opt/agent"));
    settings.agents.insert("bad/id".into(), Default::default());
    settings.agents.insert("empty".into(), Default::default());
    let agents = AvailableAgents::resolve(&catalog(), &settings);
    assert!(agents.get("beta").is_none());
    let custom = agents.get("custom").unwrap();
    assert_eq!(custom.launch().command, "/opt/agent");
    assert_eq!(custom.source(), AgentSource::Custom);
    assert_eq!(custom.profile(), None);
    assert_eq!(
        agents.warnings(),
        [
            AgentConfigWarning::Invalid("bad/id".into()),
            AgentConfigWarning::Command("empty".into()),
        ]
    );
    let alpha = agents.get("alpha").unwrap();
    assert_eq!(alpha.source(), AgentSource::Builtin);
    assert_eq!(alpha.launch().args[1], "@example/alpha-acp@1.0.0");
    assert_eq!(alpha.launch().inherit_env, ["ALPHA_API_KEY"]);
    assert_eq!(alpha.profile(), Some(&ALPHA));
    settings.enabled = false;
    assert!(AvailableAgents::resolve(&catalog(), &settings).is_empty());
}

#[test]
fn overriding_a_catalog_agent_keeps_unset_fields_and_its_profile() {
    let mut settings = AiSettings::default();
    settings.agents.insert(
        "alpha".into(),
        AgentServerSettings {
            name: Some("Mine".into()),
            command: Some("/usr/local/bin/alpha".into()),
            inherit_env: vec!["EXTRA".into()],
            ..Default::default()
        },
    );
    let agents = AvailableAgents::resolve(&catalog(), &settings);
    let alpha = agents.get("alpha").unwrap();
    assert_eq!(alpha.name(), "Mine");
    assert_eq!(alpha.launch().command, "/usr/local/bin/alpha");
    assert_eq!(alpha.launch().args, ["-y", "@example/alpha-acp@1.0.0"]);
    assert_eq!(alpha.launch().inherit_env, ["ALPHA_API_KEY", "EXTRA"]);
    assert_eq!(alpha.source(), AgentSource::Builtin);
    assert_eq!(alpha.profile(), Some(&ALPHA));
}

#[test]
fn custom_commands_wrapping_a_known_adapter_get_its_profile() {
    let mut settings = AiSettings::default();
    let mut wrapped = custom("npx");
    wrapped.args = Some(vec!["@example/alpha-acp@2.0.0".into()]);
    settings.agents.insert("work".into(), wrapped);
    settings.agents.insert("other".into(), custom("/opt/other"));
    let agents = AvailableAgents::resolve(&catalog(), &settings);
    assert_eq!(agents.get("work").unwrap().profile(), Some(&ALPHA));
    assert_eq!(agents.get("work").unwrap().source(), AgentSource::Custom);
    assert_eq!(agents.get("other").unwrap().profile(), None);
}

#[test]
fn credential_environment_overrides_are_rejected_but_explicit_inheritance_remains_possible() {
    let mut settings = AiSettings::default();
    settings.agents.insert(
        "alpha".into(),
        AgentServerSettings {
            env: std::collections::BTreeMap::from([(
                "ALPHA_API_KEY".into(),
                "plaintext-marker".into(),
            )]),
            ..Default::default()
        },
    );
    let agents = AvailableAgents::resolve(&catalog(), &settings);
    assert!(agents.get("alpha").is_none());
    assert_eq!(
        agents.warnings(),
        [AgentConfigWarning::Invalid("alpha".into())]
    );
    let mut launch = ALPHA.default_launch();
    launch
        .env
        .insert("ALPHA_API_KEY".into(), "plaintext-marker".into());
    let env = sanitized_environment(
        &launch,
        [("ALPHA_API_KEY".into(), "explicit-inherited-key".into())],
    );
    assert_eq!(env["ALPHA_API_KEY"], "explicit-inherited-key");
    assert!(!env.values().any(|value| value == "plaintext-marker"));
}

#[test]
fn catalog_rejects_profiles_that_could_not_launch_safely() {
    let launch = LaunchDefaults::new("agent", &[], &[]);
    assert_eq!(
        AgentCatalog::new([ALPHA, ALPHA]).unwrap_err(),
        CatalogError::DuplicateId("alpha")
    );
    assert_eq!(
        AgentCatalog::new([AgentProfile::new("a/b", "A", launch)]).unwrap_err(),
        CatalogError::InvalidId("a/b")
    );
    assert_eq!(
        AgentCatalog::new([AgentProfile::new(
            "a",
            "A",
            LaunchDefaults::new(" ", &[], &[])
        )])
        .unwrap_err(),
        CatalogError::MissingCommand("a")
    );
    assert_eq!(
        AgentCatalog::new([AgentProfile::new(
            "a",
            "A",
            LaunchDefaults::new("agent", &[], &["LD_PRELOAD"])
        )])
        .unwrap_err(),
        CatalogError::InvalidEnv("a")
    );
    let outside: [&'static [&'static str]; 4] = [&[""], &["/etc"], &["../x"], &["a/../../b"]];
    for paths in outside {
        assert_eq!(
            AgentCatalog::new([AgentProfile::new("a", "A", launch).writes_home(paths)])
                .unwrap_err(),
            CatalogError::InvalidHomeState("a"),
            "{paths:?}"
        );
    }
}

#[test]
fn catalog_finds_profiles_by_id_then_by_marker() {
    let catalog = catalog();
    assert!(catalog.is_builtin("beta"));
    assert!(!catalog.is_builtin("custom"));
    let mut launch = BETA.default_launch();
    assert_eq!(catalog.for_launch(&launch), Some(&BETA));
    launch.id = "custom".into();
    assert_eq!(catalog.for_launch(&launch), None);
    launch.command = "/opt/alpha-acp/bin/run".into();
    assert_eq!(catalog.for_launch(&launch), Some(&ALPHA));
    assert_eq!(
        catalog.home_state().collect::<Vec<_>>(),
        [".alpha", ".config/alpha"]
    );
}
