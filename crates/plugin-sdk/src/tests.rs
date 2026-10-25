//! Behaviour of the guest-side API: typed reads, the view builder, effects and dispatch.

use alloc::{string::String, vec::Vec};

use nocterm_plugin_abi::WireSnapshot;
use nocterm_plugin_abi::{AbiError, Answer, Call, MAX_LOG_LINES, decode_reply, encode_call};
use nocterm_plugin_api::{
    CAPABILITIES, ContributionContext, Effect, Family, HostError, Kind, LocalId, NodeKind,
    Observed, PluginIcon, Refresh, Snapshot, Tone, View, Zone,
    capability::{clock::ClockNow, notifications::NoticeTone, transfers::Counts as CountsData},
};

use crate::{
    Cx, Plugin, ViewBuilder,
    api::{Effects, Event},
    dispatch::dispatch,
    effect,
    observe::{Clock, Counts, MARKER_IDS, Observable, ObserveError, Usage},
};

mod support {
    use super::*;

    pub(super) fn local(name: &str) -> LocalId {
        LocalId::parse(name).expect("a valid local name")
    }

    pub(super) fn context() -> ContributionContext {
        ContributionContext {
            kind: Kind::Widget,
            zone: Zone::Right,
        }
    }

    pub(super) fn call_bytes(call: &Call) -> Vec<u8> {
        encode_call(call).expect("the call encodes")
    }

    pub(super) fn render_call(snapshot: &Snapshot) -> Call {
        Call::Render {
            local: local("face"),
            snapshot: WireSnapshot::from(snapshot),
        }
    }
}

use support::*;

#[derive(Default)]
struct Echo {
    renders: u32,
}

impl Plugin for Echo {
    fn render(&mut self, cx: &mut Cx<'_>) -> View {
        self.renders += 1;
        for index in 0..=MAX_LOG_LINES {
            cx.log(alloc::format!("line {index}"));
        }
        let mut view = ViewBuilder::new();
        let row = view.row(None);
        view.text(row, alloc::format!("{} {}", cx.local(), self.renders));
        if let Ok(clock) = cx.observe::<Clock>() {
            view.text(row, alloc::format!("{}", clock.unix_minutes));
        }
        view.build()
    }

    fn on_event(&mut self, cx: &mut Cx<'_>, event: &Event) -> Effects {
        match event {
            Event::Activated { key } => effect::effects([
                effect::keyed(effect::store("last", key.clone()), "save"),
                effect::notify(NoticeTone::Info, String::from(cx.local().as_str())),
            ]),
            _ => Effects::default(),
        }
    }
}

#[test]
fn every_observation_in_the_capability_table_has_a_marker() {
    let mut observed: Vec<&str> = CAPABILITIES
        .iter()
        .filter(|entry| entry.family == Family::Observe)
        .map(|entry| entry.id)
        .collect();
    let mut markers = MARKER_IDS.to_vec();
    observed.sort_unstable();
    markers.sort_unstable();
    assert_eq!(observed, markers);
}

#[test]
fn a_marker_picks_only_its_own_observation() {
    let clock = Observed::Clock(ClockNow {
        unix_minutes: 7,
        utc_offset_seconds: 0,
    });
    assert_eq!(Clock::pick(&clock).map(|c| c.unix_minutes), Some(7));
    assert!(Counts::pick(&clock).is_none());
    let counts = Counts::wrap(CountsData {
        active: 2,
        failed: 1,
    });
    assert_eq!(counts.capability(), Counts::ID);
}

#[test]
fn observing_tells_missing_from_refused_from_present() {
    let mut snapshot = Snapshot::new(context());
    snapshot.observations.insert(
        Clock::ID.into(),
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 3,
            utc_offset_seconds: 60,
        })),
    );
    snapshot
        .observations
        .insert(Usage::ID.into(), Err(HostError::Unavailable));
    snapshot.observations.insert(
        Counts::ID.into(),
        Ok(Observed::Clock(ClockNow {
            unix_minutes: 0,
            utc_offset_seconds: 0,
        })),
    );
    let local = local("face");
    let cx = Cx::new(&local, &snapshot);
    assert_eq!(cx.observe::<Clock>().map(|c| c.utc_offset_seconds), Ok(60));
    assert_eq!(
        cx.observe::<Usage>().err(),
        Some(ObserveError::Host(HostError::Unavailable))
    );
    assert_eq!(cx.observe::<Counts>().err(), Some(ObserveError::Kind));
    assert_eq!(
        cx.observe::<crate::observe::Notices>().err(),
        Some(ObserveError::Missing)
    );
}

#[test]
fn the_builder_has_a_method_for_every_node_kind() {
    let mut view = ViewBuilder::new();
    let column = view.column(None);
    let row = view.row(column);
    view.text(row, "t");
    view.icon(row, PluginIcon::Plug);
    view.meter(row, 40);
    view.badge(row, "3");
    view.button(row, "go", "Go");
    view.toggle(row, "flag", true);
    let list = view.list(column);
    view.divider(list);
    view.spacer(list);
    view.refresh(Refresh::NextMinute);
    let view = view.build();
    let mut names: Vec<&str> = view.nodes.iter().map(|node| node.kind.name()).collect();
    names.sort_unstable();
    names.dedup();
    let mut expected = NodeKind::NAMES.to_vec();
    expected.sort_unstable();
    assert_eq!(names, expected);
    assert_eq!(view.refresh, Some(Refresh::NextMinute));
    assert!(view.validate().is_ok(), "{:?}", view.validate());
}

#[test]
fn builder_parents_tones_and_keys_land_on_the_right_nodes() {
    let mut view = ViewBuilder::new();
    let row = view.row(None);
    let text = view.text(row, "hello");
    view.tone(text, Tone::Caution).key(text, "greeting");
    let nodes = view.build().nodes;
    assert_eq!(nodes[0].parent, None);
    assert_eq!(nodes[1].parent, Some(0));
    assert_eq!(nodes[1].tone, Tone::Caution);
    assert_eq!(nodes[1].key.as_deref(), Some("greeting"));
}

#[test]
fn effect_constructors_name_their_capability_and_keys() {
    let requests = [
        effect::notify(NoticeTone::Caution, "x"),
        effect::open_panel(local("panel"), true),
        effect::badge(local("panel"), Some(2)),
        effect::store("k", "v"),
        effect::forget("k"),
    ];
    let ids: Vec<&str> = requests.iter().map(|r| r.effect.capability()).collect();
    assert_eq!(
        ids,
        [
            "notifications.post",
            "panels.set-open",
            "panels.set-badge",
            "storage.set",
            "storage.remove"
        ]
    );
    let keyed = effect::keyed(effect::forget("k"), "gone");
    assert_eq!(keyed.key.as_deref(), Some("gone"));
    assert!(matches!(keyed.effect, Effect::StorageRemove(_)));
}

#[test]
fn dispatch_renders_a_view_and_caps_the_log() {
    let mut plugin = Echo::default();
    let bytes = call_bytes(&render_call(&Snapshot::new(context())));
    let reply = decode_reply(&dispatch(&mut plugin, &bytes).expect("dispatch")).expect("reply");
    let Answer::View(view) = reply.answer else {
        panic!("a render answers with a view");
    };
    assert_eq!(view.nodes.len(), 2);
    assert_eq!(reply.log.len(), MAX_LOG_LINES);
    assert_eq!(reply.log[0], "line 0");
}

#[test]
fn dispatch_keeps_the_plugin_state_between_calls() {
    let mut plugin = Echo::default();
    let bytes = call_bytes(&render_call(&Snapshot::new(context())));
    for _ in 0..2 {
        dispatch(&mut plugin, &bytes).expect("dispatch");
    }
    assert_eq!(plugin.renders, 2);
}

#[test]
fn dispatch_answers_an_event_with_effects() {
    let mut plugin = Echo::default();
    let call = Call::Event {
        local: local("face"),
        event: Event::Activated { key: "go".into() },
        snapshot: WireSnapshot::from(&Snapshot::new(context())),
    };
    let reply =
        decode_reply(&dispatch(&mut plugin, &call_bytes(&call)).expect("dispatch")).expect("reply");
    let Answer::Effects(Effects(requests)) = reply.answer else {
        panic!("an event answers with effects");
    };
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].key.as_deref(), Some("save"));
}

#[test]
fn dispatch_refuses_bytes_that_are_not_a_call() {
    let mut plugin = Echo::default();
    let error = dispatch(&mut plugin, &[0xff, 0xff, 0xff]).expect_err("garbage");
    assert!(matches!(error, AbiError::Decode(_)), "{error:?}");
}
