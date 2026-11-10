//! Requests to the user's address and timers: the credential stays the host's, answers come
//! back as events to the contribution that asked, and a stopped plugin hears nothing.

use std::time::Duration;

use gpui::TestAppContext;
use nocterm_plugin_api::{
    Effect, Event, HostError,
    capability::{
        net::{Method, Request},
        timers::SetTimer,
    },
};
use nocterm_plugin_model::{SettingValue, SettingValues};

use super::rig::{Rig, effects};
use crate::{Input, test_support::network_package};
use nocterm_plugin_api::capability::net::NetAnswer;

fn request(path: &str) -> Effect {
    Effect::NetRequest(Request {
        method: Method::Get,
        path: path.into(),
        body: None,
        bytes: false,
    })
}

/// A rig whose plugin reaches `url` with `token`.
fn connected(url: &str, token: &str, cx: &mut TestAppContext) -> Rig {
    let rig = Rig::new(cx);
    let plugin = rig
        .install(network_package(1), &[], cx)
        .expect("the package installs");
    let mut values = SettingValues::default();
    values.insert("url", SettingValue::Text(url.into()));
    values.insert("token", SettingValue::Text(token.into()));
    rig.host
        .update(cx, |host, cx| host.replace_values(&plugin, &values, cx))
        .expect("the settings are stored");
    rig
}

fn ask(rig: &Rig, effects_list: Vec<(Option<&str>, Effect)>, cx: &mut TestAppContext) {
    rig.script.answer_event(Ok(effects(effects_list)));
    rig.event("face", Input::Host, Event::SettingsChanged, cx)
        .expect("the event runs");
    cx.run_until_parked();
}

#[gpui::test]
fn a_request_goes_to_the_users_address_with_the_credential_and_answers_as_an_event(
    cx: &mut TestAppContext,
) {
    let rig = connected("https://service.example/", "a-key", cx);
    rig.net.answer(NetAnswer {
        status: 200,
        body: "{\"servers\":[]}".into(),
    });
    ask(&rig, vec![(Some("list"), request("/api/items"))], cx);
    let calls = rig.net.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].url, "https://service.example/api/items");
    assert_eq!(
        calls[0].credential,
        Some(("Authorization".into(), "Bearer a-key".into()))
    );
    assert!(rig.script.events().contains(&Event::Response {
        key: "list".into(),
        status: 200,
        body: "{\"servers\":[]}".into(),
    }));
    for snapshot in rig.script.snapshots() {
        assert!(snapshot.settings.get("token").is_none(), "never shown");
        assert!(snapshot.settings.get("url").is_some());
    }
}

#[gpui::test]
fn without_an_address_the_plugin_is_told_why_and_nothing_is_sent(cx: &mut TestAppContext) {
    let rig = connected("service.example", "a-key", cx);
    ask(&rig, vec![(Some("list"), request("/api"))], cx);
    assert!(rig.net.calls().is_empty());
    let answered = rig
        .script
        .events()
        .into_iter()
        .find_map(|event| match event {
            Event::Response { key, status, body } if key == "list" => Some((status, body)),
            _ => None,
        });
    let (status, body) = answered.expect("an answer");
    assert_eq!(status, 0);
    assert!(body.contains("address"), "{body}");
}

#[gpui::test]
fn changing_the_address_forgets_the_credential(cx: &mut TestAppContext) {
    let rig = connected("https://service.example", "a-key", cx);
    let plugin = crate::test_support::plugin_id();
    let mut moved = rig
        .host
        .read_with(cx, |host, _| host.values(&plugin))
        .unwrap();
    moved.insert(
        "url",
        SettingValue::Text("https://elsewhere.example".into()),
    );
    rig.host
        .update(cx, |host, cx| host.replace_values(&plugin, &moved, cx))
        .unwrap();
    let stored = rig
        .host
        .read_with(cx, |host, _| host.values(&plugin))
        .unwrap();
    assert_eq!(
        stored.get("token"),
        Some(&SettingValue::Text(String::new()))
    );
    ask(&rig, vec![(Some("list"), request("/api"))], cx);
    assert_eq!(rig.net.calls()[0].credential, None);
}

#[gpui::test]
fn at_most_four_requests_are_in_flight(cx: &mut TestAppContext) {
    let rig = connected("https://service.example", "k", cx);
    let keys = ["a", "b", "c", "d", "e"];
    ask(
        &rig,
        keys.iter().map(|key| (Some(*key), request("/x"))).collect(),
        cx,
    );
    assert_eq!(rig.net.calls().len(), 4);
    // The answers' own events report the outcomes, once.
    let limited = rig.script.snapshots().iter().any(|snapshot| {
        snapshot.outcomes.iter().any(|outcome| {
            outcome.key.as_deref() == Some("e")
                && outcome.result
                    == Err(HostError::RateLimited {
                        retry_after_ms: 1000,
                    })
        })
    });
    assert!(limited, "the fifth request is refused");
}

#[gpui::test]
fn a_timer_wakes_the_plugin_once_and_a_disabled_plugin_hears_nothing(cx: &mut TestAppContext) {
    let rig = connected("https://service.example", "k", cx);
    let timer = |ms| Effect::TimersSet(SetTimer { after_ms: ms });
    ask(&rig, vec![(Some("poll"), timer(1000))], cx);
    cx.executor().advance_clock(Duration::from_millis(1100));
    cx.run_until_parked();
    let fired = |rig: &Rig| {
        rig.script
            .events()
            .iter()
            .filter(|event| **event == Event::Timer { key: "poll".into() })
            .count()
    };
    assert_eq!(fired(&rig), 1);
    cx.executor().advance_clock(Duration::from_millis(5000));
    cx.run_until_parked();
    assert_eq!(fired(&rig), 1, "a timer fires once");

    ask(&rig, vec![(Some("poll"), timer(2000))], cx);
    let plugin = crate::test_support::plugin_id();
    rig.host
        .update(cx, |host, cx| host.set_enabled(&plugin, false, cx))
        .expect("the plugin is disabled");
    cx.executor().advance_clock(Duration::from_millis(3000));
    cx.run_until_parked();
    assert_eq!(fired(&rig), 1, "a stopped plugin's timer is gone");
}
