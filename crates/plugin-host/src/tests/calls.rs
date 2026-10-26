use std::time::Duration;

use gpui::TestAppContext;
use nocterm_plugin_api::{
    EffectsError, HostError, MAX_EFFECTS, MAX_NODES, ModuleError, Node, NodeKind, Observed,
    ProtocolError, RuntimeError, View, ViewError, capability::clock::ClockNow,
};
use nocterm_plugin_model::SettingValue;

use super::rig::{Rig, activated, effects, post_effect};
use crate::{
    CallError, Input, MAX_FAULTS, MAX_RENDERS_PER_SECOND, PluginDataDoc, PluginStatus, StoredValue,
    test_support::{declarative_package, plugin_id},
};

fn latest(rig: &Rig) -> nocterm_plugin_api::Snapshot {
    rig.script.snapshots().pop().expect("the guest was called")
}

#[gpui::test]
fn the_guest_sees_granted_observations_and_unavailable_where_there_is_no_provider(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.render("face", cx).unwrap();

    let snapshot = latest(&rig);
    assert_eq!(
        snapshot.observation("clock.now"),
        Some(&Ok(Observed::Clock(ClockNow {
            unix_minutes: 600,
            utc_offset_seconds: 0
        })))
    );
    assert_eq!(
        snapshot.observation("workspace.kind"),
        Some(&Err(HostError::Unavailable)),
        "no provider in this window"
    );
    assert_eq!(
        snapshot.observation("notifications.post"),
        None,
        "an effect is not an observation"
    );
}

#[gpui::test]
fn an_optional_capability_is_unavailable_until_the_user_accepted_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.render("face", cx).unwrap();
    assert_eq!(
        latest(&rig).observation("monitor.usage"),
        Some(&Err(HostError::Unavailable))
    );
}

#[gpui::test]
fn the_snapshot_carries_resolved_settings_storage_and_the_contribution_context(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.data.put(
        &id,
        PluginDataDoc {
            values: [
                ("city".to_owned(), StoredValue::Text("Oslo".into())),
                ("gone".to_owned(), StoredValue::Bool(true)),
            ]
            .into(),
            storage: [("k".to_owned(), "v".to_owned())].into(),
        },
    );
    rig.restore(cx);
    rig.render("face", cx).unwrap();

    let snapshot = latest(&rig);
    assert_eq!(
        snapshot.settings.get("city"),
        Some(&SettingValue::Text("Oslo".into()))
    );
    assert_eq!(
        snapshot.settings.get("gone"),
        None,
        "unknown keys are dropped"
    );
    assert_eq!(snapshot.storage.get("k").map(String::as_str), Some("v"));
    assert_eq!(snapshot.context, rig.call("face").context);
}

#[gpui::test]
fn observations_are_sampled_at_one_hertz_and_four_hertz_when_focused(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.render("face", cx).unwrap();
    rig.render("face", cx).unwrap();
    assert_eq!(rig.clock.reads(), 1, "the second render reuses the sample");

    cx.executor().advance_clock(Duration::from_millis(900));
    rig.render("face", cx).unwrap();
    assert_eq!(rig.clock.reads(), 1);
    cx.executor().advance_clock(Duration::from_millis(100));
    rig.render("face", cx).unwrap();
    assert_eq!(rig.clock.reads(), 2, "a second has passed");

    let mut with_focus = rig.call("face");
    with_focus.in_focus = true;
    cx.executor().advance_clock(Duration::from_millis(250));
    rig.host
        .update(cx, |host, cx| host.render(&with_focus, &rig.providers, cx))
        .unwrap();
    assert_eq!(
        rig.clock.reads(),
        3,
        "a panel with focus samples every 250 ms"
    );
}

#[gpui::test]
fn a_new_window_has_its_own_samples(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.render("face", cx).unwrap();
    let mut other = rig.call("face");
    other.window = super::rig::windows(cx)[1];
    rig.host
        .update(cx, |host, cx| host.render(&other, &rig.providers, cx))
        .unwrap();
    assert_eq!(rig.clock.reads(), 2);
}

#[gpui::test]
fn renders_are_capped_across_plugins_and_come_back_each_second(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    for _ in 0..MAX_RENDERS_PER_SECOND {
        rig.render("face", cx).unwrap();
    }
    let refused = rig.render("board", cx).unwrap_err();
    let CallError::Throttled { retry_after_ms } = refused else {
        panic!("expected a throttle, got {refused:?}");
    };
    assert!((1..=1000).contains(&retry_after_ms));

    cx.executor().advance_clock(Duration::from_secs(1));
    assert!(rig.render("board", cx).is_ok());
}

#[gpui::test]
fn a_view_is_sanitized_and_an_invalid_view_is_refused(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script.render_with(Ok(View {
        nodes: vec![Node::new(NodeKind::Text("a\u{202e}b\u{7}c".into()))],
        ..View::default()
    }));
    let view = rig.render("face", cx).unwrap();
    assert_eq!(view.nodes[0].kind, NodeKind::Text("abc".into()));

    let mut orphan = Node::new(NodeKind::Text("x".into()));
    orphan.parent = Some(5);
    rig.script.render_with(Ok(View {
        nodes: vec![orphan],
        ..View::default()
    }));
    assert!(matches!(rig.render("face", cx), Err(CallError::View(_))));
}

#[gpui::test]
fn a_contribution_the_package_does_not_have_is_refused(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    assert_eq!(
        rig.render("nothing", cx),
        Err(CallError::NoContribution("nothing".into()))
    );
}

#[gpui::test]
fn three_traps_or_missed_deadlines_take_the_plugin_out_of_service(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    for fault in [RuntimeError::Trap, RuntimeError::Deadline] {
        rig.script.render_with(Err(fault.clone()));
        assert_eq!(rig.render("face", cx), Err(CallError::Runtime(fault)));
        cx.executor().advance_clock(Duration::from_millis(10));
    }
    assert_eq!(MAX_FAULTS, 3);
    let status = |cx: &mut TestAppContext| {
        rig.host
            .read_with(cx, |host, _| host.plugins()[0].status.clone())
    };
    assert_eq!(status(cx), PluginStatus::Running);

    rig.script.render_with(Err(RuntimeError::Trap));
    assert!(rig.render("face", cx).is_err());
    assert_eq!(status(cx), PluginStatus::Faulted);
    assert_eq!(
        rig.render("face", cx),
        Err(CallError::NotRunning(id.clone()))
    );
    assert!(
        rig.records
            .doc()
            .audit
            .iter()
            .any(|entry| entry.action == crate::AuditAction::Faulted)
    );

    rig.script.render_with(Ok(View::default()));
    rig.host
        .update(cx, |host, cx| host.set_enabled(&id, true, cx))
        .unwrap();
    assert_eq!(
        status(cx),
        PluginStatus::Running,
        "a switch clears the faults"
    );
}

#[gpui::test]
fn faults_of_the_code_runtime_count_like_traps(cx: &mut TestAppContext) {
    for fault in [
        RuntimeError::Memory,
        RuntimeError::Malformed(ProtocolError::WrongAnswer),
        RuntimeError::Module(ModuleError::Exports),
    ] {
        let rig = Rig::new(cx);
        rig.installed(cx);
        rig.script.render_with(Err(fault.clone()));
        for _ in 0..MAX_FAULTS {
            assert_eq!(
                rig.render("face", cx),
                Err(CallError::Runtime(fault.clone()))
            );
            cx.executor().advance_clock(Duration::from_millis(100));
        }
        let status = rig
            .host
            .read_with(cx, |host, _| host.plugins()[0].status.clone());
        assert_eq!(status, PluginStatus::Faulted, "{fault}");
    }
}

#[gpui::test]
fn views_and_effects_a_code_plugin_returns_and_the_host_refuses_count_as_faults(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    let status = |cx: &mut TestAppContext| {
        rig.host
            .read_with(cx, |host, _| host.plugins()[0].status.clone())
    };
    let oversized = View {
        nodes: (0..=MAX_NODES)
            .map(|_| Node::new(NodeKind::Spacer))
            .collect(),
        ..View::default()
    };
    rig.script.render_with(Ok(oversized));
    for _ in 0..(MAX_FAULTS - 1) {
        assert_eq!(
            rig.render("face", cx),
            Err(CallError::View(ViewError::TooManyNodes))
        );
        cx.executor().advance_clock(Duration::from_millis(100));
    }
    assert_eq!(status(cx), PluginStatus::Running);

    let too_many = effects(
        (0..=MAX_EFFECTS)
            .map(|_| (None, post_effect("x")))
            .collect(),
    );
    rig.script.answer_event(Ok(too_many));
    assert_eq!(
        rig.event("face", Input::Pointer, activated(), cx),
        Err(CallError::Effects(EffectsError::TooMany))
    );
    assert_eq!(status(cx), PluginStatus::Faulted, "the third fault");
    assert!(
        rig.post.requests().is_empty(),
        "refused effects are never applied"
    );
}

#[gpui::test]
fn a_view_the_host_refuses_from_a_declarative_plugin_is_not_its_fault(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.install(declarative_package(1), &[], cx).unwrap();
    let oversized = View {
        nodes: (0..=MAX_NODES)
            .map(|_| Node::new(NodeKind::Spacer))
            .collect(),
        ..View::default()
    };
    rig.script.render_with(Ok(oversized));

    for _ in 0..(MAX_FAULTS + 2) {
        assert_eq!(
            rig.render("face", cx),
            Err(CallError::View(ViewError::TooManyNodes))
        );
        cx.executor().advance_clock(Duration::from_millis(100));
    }

    let status = rig
        .host
        .read_with(cx, |host, _| host.plugins()[0].status.clone());
    assert_eq!(status, PluginStatus::Running);
}

#[gpui::test]
fn a_missing_runtime_or_no_room_for_another_guest_is_not_the_plugins_fault(
    cx: &mut TestAppContext,
) {
    for refusal in [
        RuntimeError::Capacity,
        RuntimeError::InputTooLarge,
        RuntimeError::NoRuntime(nocterm_plugin_api::Runtime::Wasm),
    ] {
        let rig = Rig::new(cx);
        rig.installed(cx);
        rig.script.render_with(Err(refusal.clone()));
        for _ in 0..(MAX_FAULTS + 2) {
            assert_eq!(
                rig.render("face", cx),
                Err(CallError::Runtime(refusal.clone()))
            );
            cx.executor().advance_clock(Duration::from_millis(100));
        }
        let status = rig
            .host
            .read_with(cx, |host, _| host.plugins()[0].status.clone());
        assert_eq!(status, PluginStatus::Running, "{refusal}");
    }
}

#[gpui::test]
fn errors_that_are_not_traps_do_not_count_as_faults(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .render_with(Err(RuntimeError::UnknownContribution(
            nocterm_plugin_api::LocalId::parse("face").unwrap(),
        )));
    for _ in 0..5 {
        assert!(rig.render("face", cx).is_err());
        cx.executor().advance_clock(Duration::from_millis(100));
    }
    let status = rig
        .host
        .read_with(cx, |host, _| host.plugins()[0].status.clone());
    assert_eq!(status, PluginStatus::Running);
    let _ = plugin_id();
}
