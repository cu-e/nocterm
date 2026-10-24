use std::time::Duration;

use gpui::TestAppContext;
use nocterm_plugin_api::{
    Effect, Event, HostError, LocalId,
    capability::panels::{SetBadge, SetOpen},
};
use nocterm_plugin_ports::PluginProviders;

use super::rig::{Rig, activated, effects, open_effect, post_effect};
use crate::{
    Input,
    test_support::{package_with, plugin_id},
};

fn denied() -> HostError {
    HostError::Denied
}

#[gpui::test]
fn a_posted_notice_reaches_its_provider_for_the_plugin_that_asked(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.press("face", cx).unwrap();

    let requests = rig.post.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].0, plugin_id());
    assert_eq!(requests[0].1.text, "hello");
    assert_eq!(rig.outcomes(cx), [(Some("n".to_owned()), Ok(()))]);
}

#[gpui::test]
fn effects_are_applied_after_the_current_update_not_inside_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(None, post_effect("hello"))])));
    let call = rig.call("face");
    rig.host.update(cx, |host, cx| {
        host.handle(&call, Input::Pointer, activated(), &rig.providers, cx)
            .unwrap();
        assert!(rig.post.requests().is_empty(), "still inside the update");
    });
    assert_eq!(rig.post.requests().len(), 1);
}

#[gpui::test]
fn outcomes_come_back_once_in_the_next_snapshot_of_an_event(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.press("face", cx).unwrap();

    assert_eq!(rig.outcomes(cx), [(Some("n".to_owned()), Ok(()))]);
    assert!(rig.outcomes(cx).is_empty(), "reported once");
}

#[gpui::test]
fn a_plugin_cannot_use_a_capability_it_did_not_declare(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.install(package_with("clock", 1, &["clock.now"]), &[], cx)
        .unwrap();
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.press("face", cx).unwrap();

    assert!(rig.post.requests().is_empty());
    assert_eq!(
        rig.outcomes(cx),
        [(Some("n".to_owned()), Err(HostError::NotGranted))]
    );
}

#[gpui::test]
fn a_replacement_package_is_granted_only_what_the_user_accepted_for_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    let id = rig.installed(cx);
    rig.host
        .update(cx, |host, cx| {
            host.replace(package_with("clock", 2, &["clock.now"]), &[], true, cx)
        })
        .unwrap();
    assert_eq!(id, plugin_id());
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.press("face", cx).unwrap();
    assert!(rig.post.requests().is_empty());
}

#[gpui::test]
fn a_window_without_a_provider_answers_unavailable(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.providers = PluginProviders::new();
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.press("face", cx).unwrap();
    assert_eq!(
        rig.outcomes(cx),
        [(Some("n".to_owned()), Err(HostError::Unavailable))]
    );
}

#[gpui::test]
fn opening_a_panel_needs_a_fresh_gesture_and_spends_it(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);

    // A timer or a settings change is not a gesture.
    rig.script
        .answer_event(Ok(effects(vec![(Some("o"), open_effect("board", true))])));
    rig.event("face", Input::Host, Event::SettingsChanged, cx)
        .unwrap();
    assert!(rig.open.requests().is_empty());
    assert_eq!(rig.outcomes(cx), [(Some("o".to_owned()), Err(denied()))]);

    // A press is one, and opens one panel: the second effect of the same event finds it spent.
    rig.script.answer_event(Ok(effects(vec![
        (Some("a"), open_effect("board", true)),
        (Some("b"), open_effect("board", false)),
    ])));
    rig.press("face", cx).unwrap();
    assert_eq!(
        rig.open
            .requests()
            .into_iter()
            .map(|(_, r)| r)
            .collect::<Vec<_>>(),
        [SetOpen {
            local: LocalId::parse("board").unwrap(),
            open: true
        }]
    );
    assert_eq!(
        rig.outcomes(cx),
        [
            (Some("a".to_owned()), Ok(())),
            (Some("b".to_owned()), Err(denied()))
        ]
    );
}

#[gpui::test]
fn a_repeating_key_does_not_mint_a_gesture_but_one_press_of_enter_does(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("o"), open_effect("board", true))])));
    rig.event("face", Input::Key { repeat: true }, activated(), cx)
        .unwrap();
    assert!(rig.open.requests().is_empty());

    rig.script
        .answer_event(Ok(effects(vec![(Some("o"), open_effect("board", true))])));
    rig.event("face", Input::Key { repeat: false }, activated(), cx)
        .unwrap();
    assert_eq!(rig.open.requests().len(), 1);
}

#[gpui::test]
fn a_toggle_is_a_gesture_and_visibility_is_not(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(None, open_effect("board", true))])));
    rig.event(
        "face",
        Input::Pointer,
        Event::VisibilityChanged { visible: true },
        cx,
    )
    .unwrap();
    assert!(rig.open.requests().is_empty());

    rig.script
        .answer_event(Ok(effects(vec![(None, open_effect("board", true))])));
    rig.event(
        "face",
        Input::Pointer,
        Event::Toggled {
            key: "t".into(),
            on: true,
        },
        cx,
    )
    .unwrap();
    assert_eq!(rig.open.requests().len(), 1);
}

#[gpui::test]
fn the_surface_is_asked_when_the_effect_is_applied_not_when_it_was_planned(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.render("face", cx).unwrap();
    // Attention is raised between the snapshot and the apply.
    rig.surface.answer_with(Err(denied()));
    rig.script
        .answer_event(Ok(effects(vec![(Some("o"), open_effect("board", true))])));
    rig.press("face", cx).unwrap();

    assert!(rig.open.requests().is_empty());
    assert_eq!(rig.outcomes(cx), [(Some("o".to_owned()), Err(denied()))]);
    // The contribution that queued the effect is refused first; the target is not asked.
    assert_eq!(rig.surface.asked().len(), 1);
    assert_eq!(rig.surface.asked()[0].1, LocalId::parse("face").unwrap());
}

#[gpui::test]
fn an_effect_with_no_target_is_refused_once_its_contribution_left_the_surface(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("n"), post_effect("hello"))])));
    rig.render("face", cx).unwrap();
    // The widget that raised the event is moved to the tray before the deferred apply runs.
    rig.surface.answer_with(Err(HostError::Unavailable));
    rig.press("face", cx).unwrap();

    assert!(rig.post.requests().is_empty());
    assert_eq!(
        rig.outcomes(cx),
        [(Some("n".to_owned()), Err(HostError::Unavailable))]
    );
}

#[gpui::test]
fn an_effect_that_names_another_contribution_asks_the_surface_about_both(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script
        .answer_event(Ok(effects(vec![(Some("o"), open_effect("board", true))])));
    rig.press("face", cx).unwrap();

    let asked: Vec<_> = rig
        .surface
        .asked()
        .into_iter()
        .map(|(_, local)| local)
        .collect();
    assert_eq!(
        asked,
        [
            LocalId::parse("face").unwrap(),
            LocalId::parse("board").unwrap()
        ]
    );
    assert_eq!(rig.open.requests().len(), 1);
}

#[gpui::test]
fn an_effect_may_only_name_the_plugins_own_contributions(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script.answer_event(Ok(effects(vec![
        (Some("o"), open_effect("ghost", true)),
        (
            Some("b"),
            Effect::PanelsSetBadge(SetBadge {
                local: LocalId::parse("ghost").unwrap(),
                badge: Some(1),
            }),
        ),
    ])));
    rig.press("face", cx).unwrap();
    assert!(rig.open.requests().is_empty() && rig.badge.requests().is_empty());
    assert!(rig.surface.asked().is_empty());
    assert_eq!(
        rig.outcomes(cx),
        [
            (Some("o".to_owned()), Err(HostError::Unavailable)),
            (Some("b".to_owned()), Err(HostError::Unavailable)),
        ]
    );
}

#[gpui::test]
fn a_badge_needs_no_gesture(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script.answer_event(Ok(effects(vec![(
        None,
        Effect::PanelsSetBadge(SetBadge {
            local: LocalId::parse("board").unwrap(),
            badge: Some(3),
        }),
    )])));
    rig.event("face", Input::Host, Event::SettingsChanged, cx)
        .unwrap();
    assert_eq!(rig.badge.requests()[0].1.badge, Some(3));
}

#[gpui::test]
fn a_notice_is_limited_to_one_per_ten_seconds(cx: &mut TestAppContext) {
    let rig = Rig::new(cx);
    rig.installed(cx);
    rig.script.answer_event(Ok(effects(vec![
        (Some("a"), post_effect("one")),
        (Some("b"), post_effect("two")),
    ])));
    rig.press("face", cx).unwrap();
    assert_eq!(rig.post.requests().len(), 1);
    assert_eq!(
        rig.outcomes(cx),
        [
            (Some("a".to_owned()), Ok(())),
            (
                Some("b".to_owned()),
                Err(HostError::RateLimited {
                    retry_after_ms: 10_000
                })
            ),
        ]
    );

    cx.executor().advance_clock(Duration::from_secs(4));
    rig.script
        .answer_event(Ok(effects(vec![(Some("c"), post_effect("three"))])));
    rig.press("face", cx).unwrap();
    assert_eq!(
        rig.outcomes(cx),
        [(
            Some("c".to_owned()),
            Err(HostError::RateLimited {
                retry_after_ms: 6_000
            })
        )]
    );

    cx.executor().advance_clock(Duration::from_secs(6));
    rig.script
        .answer_event(Ok(effects(vec![(Some("d"), post_effect("four"))])));
    rig.press("face", cx).unwrap();
    assert_eq!(rig.post.requests().len(), 2);
}

#[gpui::test]
fn a_call_that_returns_too_many_or_repeated_or_oversized_effects_applies_none_of_them(
    cx: &mut TestAppContext,
) {
    let rig = Rig::new(cx);
    rig.installed(cx);

    let nine = (0..9)
        .map(|index| (None, open_effect("board", index % 2 == 0)))
        .collect();
    rig.script.answer_event(Ok(effects(nine)));
    rig.press("face", cx).unwrap();

    rig.script.answer_event(Ok(effects(vec![
        (Some("k"), post_effect("one")),
        (Some("k"), post_effect("two")),
    ])));
    rig.press("face", cx).unwrap();

    rig.script.answer_event(Ok(effects(vec![
        (Some("k"), post_effect("fine")),
        (None, post_effect(&"x".repeat(201))),
    ])));
    rig.press("face", cx).unwrap();

    assert!(rig.open.requests().is_empty());
    assert!(rig.post.requests().is_empty());
}
