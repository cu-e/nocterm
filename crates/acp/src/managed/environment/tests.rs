//! Original filtering and redaction-secret ordering remain the native oracle.
use super::*;
use crate::managed::tests::fixture;
use nocterm_ai::{connection::ManagedStartupRequest, registry::AgentLaunch};
#[test]
fn native_raw_snapshot_filters_invalid_bytes_keeps_last_parent_and_explicit_override() {
    let fixture = fixture();
    let mut launch = AgentLaunch {
        id: "test".into(),
        name: "test".into(),
        command: "test".into(),
        args: vec![],
        env: Default::default(),
        inherit_env: vec!["CUSTOM_TOKEN".into(), "CUSTOM".into()],
    };
    for (key, value) in [
        ("PATH", "override"),
        ("NOCTERM_SECRET", "hidden"),
        ("CUSTOM_TOKEN", "blocked override"),
        ("NORMAL", "value"),
        ("INVALID NAME", "bad"),
        ("ZERO", "zero\0value"),
    ] {
        launch.env.insert(key.into(), value.into());
    }
    let raw=b"PATH=first\0CUSTOM_TOKEN=inherited secret\0CUSTOM=first\0NOCTERM_X=hidden\0CUSTOM=last\0PATH=last\0NORMAL=parent\0MALFORMED\0=hidden=drive\0LANG=\xff\0";
    let resources = nocterm_ai::AgentResourceSettings::default();
    let facts = fixture
        .budget
        .context()
        .prepare_startup(ManagedStartupRequest {
            launch: &launch,
            working_directory: std::path::Path::new("/tmp"),
            create_private: false,
            isolation: None,
            terminal_auth: false,
            resources: &resources,
        })
        .unwrap();
    let actual = Environment::from_snapshot(raw, facts.get().launch(), &fixture.budget).unwrap();
    let parent = raw.split(|byte| *byte == 0).filter_map(|entry| {
        let equal = entry.get(1..)?.iter().position(|byte| *byte == b'=')? + 1;
        Some((
            String::from_utf8(entry[..equal].to_vec()).ok()?,
            String::from_utf8(entry[equal + 1..].to_vec()).ok()?,
        ))
    });
    let expected = env::sanitized_environment(&launch, parent);
    assert_eq!(
        actual.iter().collect::<Vec<_>>(),
        expected
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>()
    );
    let secrets = actual
        .secrets(facts.get().launch(), &fixture.budget)
        .unwrap();
    let mut original: Vec<_> = expected
        .iter()
        .filter(|(name, _)| {
            ["KEY", "TOKEN", "PASSWORD", "SECRET", "PROXY"]
                .iter()
                .any(|term| name.to_uppercase().contains(term))
        })
        .map(|(_, value)| value.clone())
        .chain(launch.env.values().cloned())
        .filter(|value| value.len() >= 4)
        .collect();
    original.sort_by_key(|value| std::cmp::Reverse(value.len()));
    original.dedup();
    assert_eq!(
        secrets
            .as_slice()
            .iter()
            .map(Source::text)
            .collect::<Vec<_>>(),
        original.iter().map(String::as_str).collect::<Vec<_>>()
    );
}
#[test]
fn secret_sort_preserves_original_tie_order_and_only_adjacent_duplicates() {
    let fixture = fixture();
    let mut launch = AgentLaunch {
        id: "test".into(),
        name: "test".into(),
        command: "test".into(),
        args: vec![],
        env: Default::default(),
        inherit_env: vec![],
    };
    for (key, value) in [
        ("A", "same"),
        ("B", "diff"),
        ("C", "same"),
        ("D", "longer value"),
    ] {
        launch.env.insert(key.into(), value.into());
    }
    let resources = nocterm_ai::AgentResourceSettings::default();
    let facts = fixture
        .budget
        .context()
        .prepare_startup(ManagedStartupRequest {
            launch: &launch,
            working_directory: std::path::Path::new("/tmp"),
            create_private: false,
            isolation: None,
            terminal_auth: false,
            resources: &resources,
        })
        .unwrap();
    let actual = Environment::from_snapshot(b"", facts.get().launch(), &fixture.budget).unwrap();
    let secrets = actual
        .secrets(facts.get().launch(), &fixture.budget)
        .unwrap();
    assert_eq!(
        secrets
            .as_slice()
            .iter()
            .map(Source::text)
            .collect::<Vec<_>>(),
        ["longer value", "same", "diff", "same"]
    );
}
