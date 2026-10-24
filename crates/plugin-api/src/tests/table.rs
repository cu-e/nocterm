use alloc::collections::BTreeSet;
use core::time::Duration;

use crate::{
    API_VERSION, ApiVersion, CAPABILITIES, Capability, CapabilityEntry, Deprecation, Effect,
    EffectTarget, Family, LocalId, Observed, TableError,
    capability::{
        capability,
        clock::Now,
        monitor::UsageNow,
        notifications::{NoticeTone, Own, Post, PostNotice},
        panels::{SetBadge, SetOpen, SetPanelBadge, SetPanelOpen},
        storage::{Remove, Set, StoreRemove, StoreSet},
        transfers::CountsNow,
        workspace::Kind,
    },
    validate_table,
};

fn entry(id: &'static str) -> CapabilityEntry {
    *capability(id).expect("in the table")
}

#[test]
fn the_shipped_table_follows_its_rules() {
    assert_eq!(validate_table(CAPABILITIES), Ok(()));
}

#[test]
fn every_marker_has_a_row_and_every_row_a_marker() {
    let markers: BTreeSet<&str> = [
        Now::ID,
        Kind::ID,
        UsageNow::ID,
        CountsNow::ID,
        Own::ID,
        PostNotice::ID,
        SetPanelOpen::ID,
        SetPanelBadge::ID,
        StoreSet::ID,
        StoreRemove::ID,
    ]
    .into_iter()
    .collect();
    let rows: BTreeSet<&str> = CAPABILITIES.iter().map(|entry| entry.id).collect();
    assert_eq!(markers, rows);
}

#[test]
fn effects_and_observations_name_rows_of_the_right_family() {
    let effects = [
        Effect::NotificationsPost(Post {
            tone: NoticeTone::Info,
            text: "x".into(),
            notice_key: None,
        }),
        Effect::PanelsSetOpen(SetOpen {
            local: LocalId::parse("a").unwrap(),
            open: true,
        }),
        Effect::PanelsSetBadge(SetBadge {
            local: LocalId::parse("a").unwrap(),
            badge: None,
        }),
        Effect::StorageSet(Set {
            key: "k".into(),
            value: "v".into(),
        }),
        Effect::StorageRemove(Remove { key: "k".into() }),
    ];
    for effect in &effects {
        let row = capability(effect.capability()).expect("a row");
        assert_eq!(row.family, Family::Effect);
        assert!(row.target.is_some());
    }
    let observation = Observed::Notices(Default::default());
    assert_eq!(
        capability(observation.capability()).unwrap().family,
        Family::Observe
    );
}

#[test]
fn only_effects_have_targets_and_the_target_decides_what_they_touch() {
    for row in CAPABILITIES {
        assert_eq!(
            row.target.is_some(),
            row.family == Family::Effect,
            "{}",
            row.id
        );
    }
    assert_eq!(entry("storage.set").target, Some(EffectTarget::OwnData));
    assert_eq!(
        entry("panels.set-open").target,
        Some(EffectTarget::OwnContribution)
    );
    assert!(entry("panels.set-open").gesture);
    assert_eq!(entry("notifications.post").live_cap, Some(3));
    assert_eq!(
        entry("notifications.post").rate.map(|(_, per)| per),
        Some(Duration::from_secs(10))
    );
}

#[test]
fn passive_providers_are_session_derived_where_they_read_the_session() {
    for id in [
        "clock.now",
        "workspace.kind",
        "monitor.usage",
        "transfers.counts",
    ] {
        assert!(entry(id).session_derived, "{id}");
    }
    assert!(!entry("notifications.own").session_derived);
}

#[test]
fn a_broken_table_is_refused_for_each_rule() {
    let base = entry("clock.now");
    let with = |change: fn(&mut CapabilityEntry)| {
        let mut row = base;
        change(&mut row);
        validate_table(&[row])
    };
    assert_eq!(
        with(|r| r.id = "Clock.now"),
        Err(TableError::Id("Clock.now"))
    );
    assert_eq!(with(|r| r.id = "clock"), Err(TableError::Id("clock")));
    assert_eq!(
        with(|r| r.id = "clock.a--b"),
        Err(TableError::Id("clock.a--b"))
    );
    assert_eq!(
        with(|r| r.target = Some(EffectTarget::OwnData)),
        Err(TableError::Target("clock.now"))
    );
    assert_eq!(
        with(|r| r.family = Family::Effect),
        Err(TableError::Target("clock.now"))
    );
    assert_eq!(
        with(|r| r.consent = " "),
        Err(TableError::Text("clock.now"))
    );
    assert_eq!(
        with(|r| r.excluded = ""),
        Err(TableError::Text("clock.now"))
    );
    assert_eq!(
        with(|r| r.since = ApiVersion::new(API_VERSION.major, API_VERSION.minor + 1)),
        Err(TableError::Future("clock.now"))
    );
    assert_eq!(
        with(|r| {
            r.deprecated = Some(Deprecation {
                since: API_VERSION,
                successor: "clock.later",
                note: "n",
            });
        }),
        Err(TableError::Successor("clock.now"))
    );
    assert_eq!(
        validate_table(&[base, base]),
        Err(TableError::Duplicate("clock.now"))
    );
}
