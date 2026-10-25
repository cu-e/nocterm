//! Behavior tests of the ABI types and codec.

use alloc::{collections::BTreeMap, string::String, vec, vec::Vec};

use nocterm_plugin_api::{
    ContributionContext, EffectOutcome, HostError, Kind, LocalId, NodeKind, Observed, PluginIcon,
    Snapshot, Zone,
    capability::{
        clock::ClockNow,
        monitor::Usage,
        notifications::{NoticeTone, OwnNotice, OwnNotices},
        transfers::Counts,
        workspace::{WorkspaceKind, WorkspaceState},
    },
};
use nocterm_plugin_model::{SettingValue, SettingValues};

use crate::{
    ABI_VERSION, AbiError, Answer, Call, MAX_INPUT_BYTES, MAX_REPLY_BYTES, Reply, WireObserved,
    WireSnapshot, decode_call, decode_reply, encode_call, encode_reply, pack, samples, unpack,
};

fn snapshot() -> Snapshot {
    let mut settings = SettingValues::default();
    settings.insert("on", SettingValue::Bool(true));
    settings.insert("mode", SettingValue::Choice("fast".into()));
    settings.insert("sets", SettingValue::Choices(vec!["a".into(), "b".into()]));
    settings.insert("count", SettingValue::Number(-3));
    settings.insert("city", SettingValue::Text("Oslo".into()));
    let mut observations = BTreeMap::new();
    observations.insert(
        "clock.now".into(),
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 5,
            utc_offset_seconds: -3600,
        })),
    );
    observations.insert(
        "workspace.kind".into(),
        Ok(Observed::Workspace(WorkspaceState {
            kind: WorkspaceKind::Container,
            connected: false,
        })),
    );
    observations.insert(
        "monitor.usage".into(),
        Ok(Observed::Usage(Usage {
            cpu_percent: 1.5,
            memory_percent: 99.0,
        })),
    );
    observations.insert(
        "transfers.counts".into(),
        Ok(Observed::Counts(Counts {
            active: 1,
            failed: 0,
        })),
    );
    observations.insert(
        "notifications.own".into(),
        Ok(Observed::Notices(OwnNotices(vec![OwnNotice {
            tone: NoticeTone::Info,
            text: "x".into(),
        }]))),
    );
    observations.insert("future.cap".into(), Err(HostError::NotGranted));
    Snapshot {
        observations,
        settings,
        storage: BTreeMap::from([("k".into(), "v".into())]),
        context: ContributionContext {
            kind: Kind::Panel,
            zone: Zone::Center,
        },
        outcomes: vec![EffectOutcome {
            key: Some("a".into()),
            result: Err(HostError::Denied),
        }],
    }
}

#[test]
fn a_snapshot_survives_the_wire_with_every_observation_and_setting_kind() {
    let snapshot = snapshot();
    let wire = WireSnapshot::from(&snapshot);
    assert_eq!(wire.observations.len(), 6);
    assert_eq!(wire.settings.len(), 5);
    let bytes = encode_call(&Call::Render {
        local: LocalId::parse("bar").unwrap(),
        snapshot: wire,
    })
    .unwrap();
    let Call::Render { snapshot: back, .. } = decode_call(&bytes).unwrap() else {
        panic!("a render call stays a render call");
    };
    assert_eq!(Snapshot::from(back), snapshot);
}

#[test]
fn every_sample_decodes_and_encodes_to_the_same_bytes() {
    let all = samples();
    assert!(all.len() >= 10);
    for sample in &all {
        let again = match decode_call(&sample.bytes) {
            Ok(call) => encode_call(&call),
            Err(_) => encode_reply(&decode_reply(&sample.bytes).expect(sample.name)),
        };
        assert_eq!(again.as_ref(), Ok(&sample.bytes), "{}", sample.name);
    }
}

#[test]
fn sample_names_are_unique_and_the_bytes_are_stable_between_runs() {
    let first = samples();
    let second = samples();
    assert_eq!(first, second);
    let mut names: Vec<_> = first.iter().map(|s| s.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), first.len());
}

#[test]
fn the_samples_pin_every_icon_workspace_kind_and_zone() {
    let all = samples();
    assert_eq!(
        all.len(),
        13 + Zone::ALL.len(),
        "append a sample, never drop one"
    );
    let bytes = |name: &str| {
        &all.iter()
            .find(|sample| sample.name == name)
            .expect(name)
            .bytes
    };

    let Answer::View(view) = decode_reply(bytes("reply-view-icons")).unwrap().answer else {
        panic!("the icon sample is a view");
    };
    let icons: Vec<_> = view
        .nodes
        .iter()
        .filter_map(|node| match node.kind {
            NodeKind::Icon(icon) => Some(icon),
            _ => None,
        })
        .collect();
    assert_eq!(icons, PluginIcon::ALL);

    let Call::Render { snapshot, .. } = decode_call(bytes("call-render-workspace-kinds")).unwrap()
    else {
        panic!("the workspace sample is a render call");
    };
    let kinds: Vec<_> = snapshot
        .observations
        .into_iter()
        .filter_map(|(_, observed)| match observed {
            Ok(WireObserved::Workspace(state)) => Some(state.kind),
            _ => None,
        })
        .collect();
    assert_eq!(kinds, WorkspaceKind::ALL);

    let zones: Vec<_> = all
        .iter()
        .filter(|sample| sample.name.starts_with("call-render-zone-"))
        .map(|sample| match decode_call(&sample.bytes).unwrap() {
            Call::Render { snapshot, .. } => snapshot.context.zone,
            Call::Event { snapshot, .. } => snapshot.context.zone,
        })
        .collect();
    assert_eq!(zones, Zone::ALL);
}

#[test]
fn the_largest_snapshot_the_host_can_build_fits_the_input_bound_with_room_to_spare() {
    use nocterm_plugin_api::capability::storage::{MAX_KEY, MAX_TOTAL, MAX_VALUE};
    let entries = MAX_TOTAL / MAX_VALUE;
    let wire = WireSnapshot {
        observations: vec![(
            "notifications.own".into(),
            Ok(WireObserved::Notices(OwnNotices(
                (0..3)
                    .map(|_| OwnNotice {
                        tone: NoticeTone::Caution,
                        text: "\u{1f600}".repeat(200),
                    })
                    .collect(),
            ))),
        )],
        // The host caps the setting values of a plugin at 4 KiB.
        settings: vec![(
            "city".into(),
            crate::WireSetting::Text("x".repeat(4 * 1024)),
        )],
        storage: (0..entries)
            .map(|index| (alloc::format!("{index:0>MAX_KEY$}"), "v".repeat(MAX_VALUE)))
            .collect(),
        context: ContributionContext {
            kind: Kind::Panel,
            zone: Zone::Left,
        },
        outcomes: (0..32)
            .map(|_| EffectOutcome {
                key: Some("k".repeat(32)),
                result: Err(HostError::RateLimited {
                    retry_after_ms: u32::MAX,
                }),
            })
            .collect(),
    };
    let bytes = encode_call(&Call::Render {
        local: LocalId::parse("bar").unwrap(),
        snapshot: wire,
    })
    .expect("the worst case fits");
    assert!(bytes.len() * 2 < MAX_INPUT_BYTES, "{} bytes", bytes.len());
}

#[test]
fn a_reply_carries_its_answer_and_log() {
    let reply = Reply {
        answer: Answer::Effects(Default::default()),
        log: vec![String::from("hello")],
    };
    let bytes = encode_reply(&reply).unwrap();
    assert_eq!(decode_reply(&bytes), Ok(reply));
}

#[test]
fn bytes_that_are_not_a_value_or_have_a_tail_are_refused() {
    assert!(matches!(decode_reply(&[]), Err(AbiError::Decode(_))));
    assert!(matches!(
        decode_call(&[0xff, 0xff, 0xff]),
        Err(AbiError::Decode(_))
    ));
    let reply = Reply {
        answer: Answer::Effects(Default::default()),
        log: Vec::new(),
    };
    let mut bytes = encode_reply(&reply).unwrap();
    bytes.push(0);
    assert_eq!(decode_reply(&bytes), Err(AbiError::Trailing));
}

#[test]
fn the_bounds_apply_before_decoding_and_after_encoding() {
    let big = vec![0u8; MAX_INPUT_BYTES + 1];
    assert_eq!(
        decode_call(&big),
        Err(AbiError::TooLarge {
            max: MAX_INPUT_BYTES
        })
    );
    let big = vec![0u8; MAX_REPLY_BYTES + 1];
    assert_eq!(
        decode_reply(&big),
        Err(AbiError::TooLarge {
            max: MAX_REPLY_BYTES
        })
    );
    let call = Call::Render {
        local: LocalId::parse("bar").unwrap(),
        snapshot: WireSnapshot {
            storage: vec![("k".into(), "v".repeat(MAX_INPUT_BYTES))],
            ..WireSnapshot::from(&Snapshot::new(ContributionContext {
                kind: Kind::Widget,
                zone: Zone::Left,
            }))
        },
    };
    assert_eq!(
        encode_call(&call),
        Err(AbiError::TooLarge {
            max: MAX_INPUT_BYTES
        })
    );
    let reply = Reply {
        answer: Answer::Effects(Default::default()),
        log: vec!["x".repeat(MAX_REPLY_BYTES)],
    };
    assert_eq!(
        encode_reply(&reply),
        Err(AbiError::TooLarge {
            max: MAX_REPLY_BYTES
        })
    );
}

#[test]
fn a_decode_error_keeps_the_codec_cause() {
    use core::error::Error as _;
    let error = decode_reply(&[]).unwrap_err();
    assert!(error.source().is_some());
}

#[test]
fn a_pointer_and_a_length_pack_into_one_value_and_back() {
    for (ptr, len) in [(0, 0), (1, 2), (u32::MAX, 0), (0, u32::MAX), (65536, 4096)] {
        assert_eq!(unpack(pack(ptr, len)), (ptr, len));
    }
    assert_eq!(ABI_VERSION, 1);
}
