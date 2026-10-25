//! Canonical values, encoded. xtask writes them to `abi-golden.json`; a change to the bytes of
//! an existing sample is a change of the ABI and needs a new `ABI_VERSION`.

use alloc::{string::String, vec, vec::Vec};

use nocterm_plugin_api::{
    ContributionContext, Effect, EffectOutcome, EffectRequest, Effects, Event, HostError, Kind,
    LocalId, Node, NodeKind, PluginIcon, Refresh, Tone, View, Zone,
    capability::{
        clock::ClockNow,
        monitor::Usage,
        notifications::{NoticeTone, OwnNotice, OwnNotices, Post},
        panels::{SetBadge, SetOpen},
        storage::{Remove, Set},
        transfers::Counts,
        workspace::{WorkspaceKind, WorkspaceState},
    },
};

use crate::{
    Answer, Call, Reply, WireObserved, WireSetting, WireSnapshot, encode_call, encode_reply,
};

/// A named encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub name: &'static str,
    pub bytes: Vec<u8>,
}

fn local(name: &str) -> LocalId {
    LocalId::parse(name).expect("a sample local name is valid")
}

fn context() -> ContributionContext {
    ContributionContext {
        kind: Kind::Widget,
        zone: Zone::Right,
    }
}

/// A snapshot that carries every observation and every setting kind.
fn full_snapshot() -> WireSnapshot {
    WireSnapshot {
        observations: vec![
            (
                String::from("clock.now"),
                Ok(WireObserved::Clock(ClockNow {
                    unix_minutes: 29_000_000,
                    utc_offset_seconds: 3600,
                })),
            ),
            (
                String::from("workspace.kind"),
                Ok(WireObserved::Workspace(WorkspaceState {
                    kind: WorkspaceKind::Remote,
                    connected: true,
                })),
            ),
            (
                String::from("monitor.usage"),
                Ok(WireObserved::Usage(Usage {
                    cpu_percent: 12.5,
                    memory_percent: 40.0,
                })),
            ),
            (
                String::from("transfers.counts"),
                Ok(WireObserved::Counts(Counts {
                    active: 2,
                    failed: 1,
                })),
            ),
            (
                String::from("notifications.own"),
                Ok(WireObserved::Notices(OwnNotices(vec![OwnNotice {
                    tone: NoticeTone::Caution,
                    text: String::from("Done"),
                }]))),
            ),
            (String::from("later.cap"), Err(HostError::Unavailable)),
        ],
        settings: vec![
            (String::from("on"), WireSetting::Bool(true)),
            (
                String::from("mode"),
                WireSetting::Choice(String::from("fast")),
            ),
            (
                String::from("sets"),
                WireSetting::Choices(vec![String::from("a"), String::from("b")]),
            ),
            (String::from("count"), WireSetting::Number(-7)),
            (
                String::from("city"),
                WireSetting::Text(String::from("Oslo")),
            ),
        ],
        storage: vec![(String::from("start"), String::from("1700000000"))],
        context: context(),
        outcomes: vec![
            EffectOutcome {
                key: Some(String::from("open")),
                result: Ok(()),
            },
            EffectOutcome {
                key: None,
                result: Err(HostError::RateLimited {
                    retry_after_ms: 250,
                }),
            },
            EffectOutcome {
                key: Some(String::from("post")),
                result: Err(HostError::NotGranted),
            },
            EffectOutcome {
                key: None,
                result: Err(HostError::Denied),
            },
        ],
    }
}

fn empty_snapshot() -> WireSnapshot {
    WireSnapshot {
        observations: Vec::new(),
        settings: Vec::new(),
        storage: Vec::new(),
        context: ContributionContext {
            kind: Kind::Panel,
            zone: Zone::Left,
        },
        outcomes: Vec::new(),
    }
}

fn node(parent: Option<u16>, key: Option<&str>, kind: NodeKind, tone: Tone) -> Node {
    Node {
        parent,
        key: key.map(String::from),
        kind,
        tone,
    }
}

/// A view with every node kind, tone and refresh.
fn full_view(refresh: Refresh) -> View {
    View {
        nodes: vec![
            node(None, None, NodeKind::Column, Tone::Default),
            node(Some(0), None, NodeKind::Row, Tone::Muted),
            node(
                Some(1),
                None,
                NodeKind::Icon(PluginIcon::Puzzle),
                Tone::Success,
            ),
            node(
                Some(1),
                None,
                NodeKind::Text(String::from("12:30")),
                Tone::Caution,
            ),
            node(Some(1), None, NodeKind::Meter(40), Tone::Default),
            node(
                Some(1),
                None,
                NodeKind::Badge(String::from("3")),
                Tone::Default,
            ),
            node(
                Some(1),
                Some("go"),
                NodeKind::Button(String::from("Go")),
                Tone::Default,
            ),
            node(Some(1), Some("flag"), NodeKind::Toggle(true), Tone::Default),
            node(Some(0), None, NodeKind::List, Tone::Default),
            node(Some(8), None, NodeKind::Divider, Tone::Default),
            node(Some(8), None, NodeKind::Spacer, Tone::Default),
        ],
        refresh: Some(refresh),
    }
}

fn full_effects() -> Effects {
    let request = |key: Option<&str>, effect| EffectRequest {
        key: key.map(String::from),
        effect,
    };
    Effects(vec![
        request(
            Some("note"),
            Effect::NotificationsPost(Post {
                tone: NoticeTone::Info,
                text: String::from("Hello"),
                notice_key: Some(String::from("hello")),
            }),
        ),
        request(
            None,
            Effect::PanelsSetOpen(SetOpen {
                local: local("panel"),
                open: true,
            }),
        ),
        request(
            None,
            Effect::PanelsSetBadge(SetBadge {
                local: local("panel"),
                badge: Some(3),
            }),
        ),
        request(
            Some("save"),
            Effect::StorageSet(Set {
                key: String::from("start"),
                value: String::from("1700000000"),
            }),
        ),
        request(
            None,
            Effect::StorageRemove(Remove {
                key: String::from("start"),
            }),
        ),
    ])
}

/// A view with one icon node for every icon, so a reordered icon changes a sample.
fn icons_view() -> View {
    let mut nodes = vec![node(None, None, NodeKind::Row, Tone::Default)];
    nodes.extend(
        PluginIcon::ALL
            .into_iter()
            .map(|icon| node(Some(0), None, NodeKind::Icon(icon), Tone::Default)),
    );
    View {
        nodes,
        refresh: None,
    }
}

/// A snapshot with the workspace observation of every kind, under one key each.
fn workspace_kinds_snapshot() -> WireSnapshot {
    let observations = WorkspaceKind::ALL
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            (
                alloc::format!("workspace.kind.{index}"),
                Ok(WireObserved::Workspace(WorkspaceState {
                    kind,
                    connected: index % 2 == 0,
                })),
            )
        })
        .collect();
    WireSnapshot {
        observations,
        ..empty_snapshot()
    }
}

/// The names of the samples that pin one context each, in the order of [`Zone::ALL`].
const ZONE_SAMPLES: [&str; Zone::ALL.len()] = [
    "call-render-zone-left",
    "call-render-zone-center",
    "call-render-zone-right",
];

fn reply(answer: Answer, log: &[&str]) -> Reply {
    Reply {
        answer,
        log: log.iter().map(|line| String::from(*line)).collect(),
    }
}

fn sample(name: &'static str, bytes: Result<Vec<u8>, crate::AbiError>) -> Sample {
    Sample {
        name,
        bytes: bytes.expect("a canonical sample fits the ABI bounds"),
    }
}

/// The reply samples and the workspace-kinds render sample, in golden-file order.
fn reply_samples() -> Vec<Sample> {
    vec![
        sample(
            "reply-view-next-second",
            encode_reply(&reply(
                Answer::View(full_view(Refresh::NextSecond)),
                &["rendered"],
            )),
        ),
        sample(
            "reply-view-next-minute",
            encode_reply(&reply(Answer::View(full_view(Refresh::NextMinute)), &[])),
        ),
        sample(
            "reply-view-after-ms",
            encode_reply(&reply(Answer::View(full_view(Refresh::AfterMs(500))), &[])),
        ),
        sample(
            "reply-effects",
            encode_reply(&reply(Answer::Effects(full_effects()), &["a", "b"])),
        ),
        sample(
            "reply-effects-none",
            encode_reply(&reply(Answer::Effects(Effects::default()), &[])),
        ),
        sample(
            "reply-view-icons",
            encode_reply(&reply(Answer::View(icons_view()), &[])),
        ),
        sample(
            "call-render-workspace-kinds",
            encode_call(&Call::Render {
                local: local("bar"),
                snapshot: workspace_kinds_snapshot(),
            }),
        ),
    ]
}

/// Every canonical sample, in a fixed order. New samples are appended.
pub fn samples() -> Vec<Sample> {
    let events = [
        (
            "call-event-activated",
            Event::Activated {
                key: String::from("go"),
            },
        ),
        (
            "call-event-toggled",
            Event::Toggled {
                key: String::from("flag"),
                on: true,
            },
        ),
        (
            "call-event-visibility",
            Event::VisibilityChanged { visible: false },
        ),
        ("call-event-settings", Event::SettingsChanged),
    ];
    let mut samples = vec![
        sample(
            "call-render-empty",
            encode_call(&Call::Render {
                local: local("bar"),
                snapshot: empty_snapshot(),
            }),
        ),
        sample(
            "call-render-full",
            encode_call(&Call::Render {
                local: local("bar"),
                snapshot: full_snapshot(),
            }),
        ),
    ];
    samples.extend(events.into_iter().map(|(name, event)| {
        sample(
            name,
            encode_call(&Call::Event {
                local: local("panel"),
                event,
                snapshot: full_snapshot(),
            }),
        )
    }));
    samples.extend(reply_samples());
    samples.extend(ZONE_SAMPLES.into_iter().zip(Zone::ALL).map(|(name, zone)| {
        sample(
            name,
            encode_call(&Call::Render {
                local: local("bar"),
                snapshot: WireSnapshot {
                    context: ContributionContext {
                        kind: Kind::Widget,
                        zone,
                    },
                    ..empty_snapshot()
                },
            }),
        )
    }));
    samples
}
