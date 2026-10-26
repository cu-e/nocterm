//! Behaviour of `testing::Host`: what a plugin author sees when a test drives a plugin.

use std::{string::String, vec::Vec};

use nocterm_plugin_api::{
    EffectOutcome, Effects, EffectsError, Event, HostError, LocalId, NodeKind, View, ViewError,
    capability::{
        clock::ClockNow, monitor::Usage as UsageData, notifications::NoticeTone, storage::MAX_TOTAL,
    },
};
use nocterm_plugin_model::SettingValue;

use crate::{
    Cx, Plugin, ViewBuilder, effect,
    observe::{Clock, Notices, Usage},
    testing::{Host, TestError},
};

const MANIFEST: &str = r#"
api = "0.1"
runtime = "wasm"
publisher = "example"
id = "probe"
name = "Probe"
capabilities = ["clock.now", "storage.set", "storage.remove", "panels.set-open"]
optional_capabilities = ["monitor.usage", "notifications.post"]

[[contribution]]
local = "bar"
kind = "widget"
title = "Bar"

[[contribution]]
local = "panel"
kind = "panel"
title = "Panel"

[settings]
version = 1

[[settings.group]]
title = "Probe"

[[settings.group.field]]
key = "label"
label = "Label"
kind = "text"
default = "Hi"
max_len = 12
"#;

/// Records what a call could see, and answers events by key.
#[derive(Default)]
struct Probe {
    seen_outcomes: Vec<EffectOutcome>,
    seen_label: Option<SettingValue>,
    seen_stored: Option<String>,
    seen_clock: Option<i64>,
    seen_usage_error: Option<HostError>,
    bad_view: bool,
    chatty: bool,
}

fn local(name: &str) -> LocalId {
    LocalId::parse(name).expect("a valid local name")
}

impl Plugin for Probe {
    fn render(&mut self, cx: &mut Cx<'_>) -> View {
        self.seen_outcomes = cx.outcomes().to_vec();
        self.seen_label = cx.setting("label").cloned();
        self.seen_stored = cx.stored("k").map(String::from);
        self.seen_clock = cx.observe::<Clock>().ok().map(|c| c.unix_minutes);
        self.seen_usage_error = match cx.observe::<Usage>() {
            Err(crate::ObserveError::Host(error)) => Some(error),
            _ => None,
        };
        if self.chatty {
            for index in 0..20 {
                cx.log(std::format!("line {index}\u{7}"));
            }
        }
        let mut view = ViewBuilder::new();
        if self.bad_view {
            view.button(None, "ok", "Fine");
            view.button(None, "ok", "Twice");
        } else {
            view.text(None, "probe");
        }
        view.build()
    }

    fn on_event(&mut self, _cx: &mut Cx<'_>, event: &Event) -> Effects {
        let Event::Activated { key } = event else {
            return Effects::default();
        };
        match key.as_str() {
            "store" => effect::effects([effect::keyed(effect::store("k", "v"), "save")]),
            "remove" => effect::effects([effect::forget("k")]),
            "open" => {
                effect::effects([effect::keyed(effect::open_panel(local("panel"), true), "o")])
            }
            "ghost" => {
                effect::effects([effect::keyed(effect::open_panel(local("ghost"), true), "g")])
            }
            "badge" => {
                effect::effects([effect::keyed(effect::badge(local("panel"), Some(1)), "b")])
            }
            "notify" => {
                effect::effects([effect::keyed(effect::notify(NoticeTone::Info, "hi"), "n")])
            }
            "big" => effect::effects([effect::keyed(effect::store("big", "x".repeat(64)), "big")]),
            "many" => effect::effects((0..9).map(|n| effect::store(std::format!("k{n}"), "v"))),
            _ => Effects::default(),
        }
    }
}

fn host() -> Host {
    Host::new(MANIFEST).expect("the probe manifest is valid")
}

fn click(host: &mut Host, probe: &mut Probe, key: &str) -> Effects {
    host.event(probe, "bar", Event::Activated { key: key.into() })
        .expect("the event is answered")
}

fn next_outcomes(
    host: &mut Host,
    probe: &mut Probe,
) -> Vec<(Option<String>, Result<(), HostError>)> {
    host.render(probe, "bar").expect("renders");
    probe
        .seen_outcomes
        .iter()
        .map(|o| (o.key.clone(), o.result))
        .collect()
}

#[test]
fn a_bad_manifest_is_reported_with_its_cause() {
    assert!(matches!(Host::new("api = ["), Err(TestError::Toml(_))));
    let declarative = r#"
api = "0.1"
publisher = "example"
id = "plain"
name = "Plain"

[[contribution]]
local = "face"
kind = "widget"
title = "Face"

[contribution.view]
nodes = [{ kind = "text", text = "hi" }]
"#;
    assert!(matches!(Host::new(declarative), Err(TestError::NotCode)));
    let renamed = MANIFEST.replace("name = \"Probe\"", "name = \"Nocterm\"");
    let error = Host::new(&renamed).err().expect("a reserved name");
    assert!(matches!(error, TestError::Manifest(_)), "{error}");
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn a_view_the_host_would_refuse_is_an_error() {
    let mut probe = Probe {
        bad_view: true,
        ..Probe::default()
    };
    let error = host().render(&mut probe, "bar").expect_err("duplicate key");
    assert!(
        matches!(error, TestError::View(ViewError::DuplicateKey { .. })),
        "{error:?}"
    );
}

#[test]
fn unknown_and_malformed_contribution_names_are_refused() {
    let mut probe = Probe::default();
    assert!(matches!(
        host().render(&mut probe, "nope"),
        Err(TestError::UnknownContribution(_))
    ));
    assert!(matches!(
        host().render(&mut probe, "Not Valid"),
        Err(TestError::Local(..))
    ));
}

#[test]
fn a_render_shows_observations_settings_and_storage() {
    let mut host = host();
    host.observe::<Clock>(ClockNow {
        unix_minutes: 42,
        utc_offset_seconds: 0,
    })
    .set("label", SettingValue::Text("Hey".into()))
    .store("k", "stored");
    let mut probe = Probe::default();
    let view = host.render(&mut probe, "bar").expect("renders");
    assert_eq!(view.nodes[0].kind, NodeKind::Text("probe".into()));
    assert_eq!(probe.seen_clock, Some(42));
    assert_eq!(probe.seen_label, Some(SettingValue::Text("Hey".into())));
    assert_eq!(probe.seen_stored.as_deref(), Some("stored"));
}

#[test]
fn settings_start_at_the_schema_defaults() {
    let mut probe = Probe::default();
    host().render(&mut probe, "bar").expect("renders");
    assert_eq!(probe.seen_label, Some(SettingValue::Text("Hi".into())));
}

#[test]
fn an_undeclared_observation_is_absent_and_a_declined_optional_one_is_unavailable() {
    let mut host = host();
    host.observe::<Notices>(Default::default())
        .observe::<Usage>(UsageData {
            cpu_percent: 1.0,
            memory_percent: 2.0,
        })
        .deny("monitor.usage")
        .expect("monitor.usage is optional");
    let mut probe = Probe::default();
    host.render(&mut probe, "bar").expect("renders");
    assert_eq!(probe.seen_usage_error, Some(HostError::Unavailable));
}

#[test]
fn outcomes_arrive_once_in_the_next_snapshot() {
    let mut host = host();
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "open");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("o".into()), Ok(()))]
    );
    assert!(next_outcomes(&mut host, &mut probe).is_empty());
}

#[test]
fn effects_without_a_declared_capability_are_not_granted() {
    let mut host = host();
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "badge");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("b".into()), Err(HostError::NotGranted))]
    );
}

#[test]
fn a_declined_optional_effect_is_denied_and_a_granted_one_is_ok() {
    let mut host = host();
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "notify");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("n".into()), Ok(()))]
    );
    host.deny("notifications.post")
        .expect("notifications.post is optional");
    click(&mut host, &mut probe, "notify");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("n".into()), Err(HostError::Denied))]
    );
}

#[test]
fn an_effect_naming_another_contribution_is_unavailable() {
    let mut host = host();
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "ghost");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("g".into()), Err(HostError::Unavailable))]
    );
}

#[test]
fn storage_effects_change_the_stored_data_the_next_render_reads() {
    let mut host = host();
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "store");
    assert_eq!(host.storage().get("k").map(String::as_str), Some("v"));
    host.render(&mut probe, "bar").expect("renders");
    assert_eq!(probe.seen_stored.as_deref(), Some("v"));
    click(&mut host, &mut probe, "remove");
    assert!(host.storage().is_empty());
}

#[test]
fn storage_over_the_total_is_denied_and_changes_nothing() {
    let mut host = host();
    host.store("filler", &"x".repeat(MAX_TOTAL - 8));
    let mut probe = Probe::default();
    click(&mut host, &mut probe, "big");
    assert_eq!(
        next_outcomes(&mut host, &mut probe),
        [(Some("big".into()), Err(HostError::Denied))]
    );
    assert!(!host.storage().contains_key("big"));
}

#[test]
fn more_than_eight_effects_are_dropped_whole() {
    let mut host = host();
    let mut probe = Probe::default();
    let error = host
        .event(&mut probe, "bar", Event::Activated { key: "many".into() })
        .expect_err("nine effects");
    assert!(matches!(error, TestError::Effects(EffectsError::TooMany)));
    assert!(host.storage().is_empty());
}

#[test]
fn log_lines_are_capped_per_call_and_sanitized() {
    let mut host = host();
    let mut probe = Probe {
        chatty: true,
        ..Probe::default()
    };
    host.render(&mut probe, "bar").expect("renders");
    assert_eq!(host.log().len(), 8);
    assert_eq!(host.log()[0], "line 0");
}

#[test]
fn a_required_capability_cannot_be_denied_because_the_host_never_withholds_it() {
    let mut host = host();
    assert!(matches!(
        host.deny("storage.set"),
        Err(TestError::Required(id)) if id == "storage.set"
    ));
    assert!(matches!(
        host.deny("clock.now"),
        Err(TestError::Required(id)) if id == "clock.now"
    ));
}

#[test]
fn a_required_observation_that_fails_is_tested_with_an_error_not_a_denial() {
    let mut host = host();
    host.observe::<Clock>(ClockNow {
        unix_minutes: 7,
        utc_offset_seconds: 0,
    });
    let mut probe = Probe::default();
    host.render(&mut probe, "bar").expect("renders");
    assert_eq!(probe.seen_clock, Some(7));

    host.observe_error::<Clock>(HostError::Unavailable);
    host.render(&mut probe, "bar").expect("renders");
    assert_eq!(probe.seen_clock, None);
}

#[test]
fn a_reply_with_more_log_lines_than_the_host_accepts_is_refused_whole() {
    let lines: Vec<String> = (0..=nocterm_plugin_abi::MAX_LOG_LINES)
        .map(|index| std::format!("line {index}"))
        .collect();
    assert!(matches!(
        crate::testing::accepted_log(&lines),
        Err(nocterm_plugin_api::ProtocolError::TooManyLogLines { max })
            if max == nocterm_plugin_abi::MAX_LOG_LINES
    ));
    let kept = crate::testing::accepted_log(&lines[..nocterm_plugin_abi::MAX_LOG_LINES])
        .expect("the bound itself is accepted");
    assert_eq!(kept.len(), nocterm_plugin_abi::MAX_LOG_LINES);
}
