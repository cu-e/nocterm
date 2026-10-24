use alloc::{string::ToString, vec, vec::Vec};

use nocterm_plugin_model::{SettingValue, SettingsSchema};
use proptest::prelude::*;

use super::support::{snapshot, text_schema, with_clock};
use crate::{
    NodeKind, Observed, Tone,
    capability::{
        monitor::Usage,
        notifications::{NoticeTone, OwnNotice, OwnNotices},
    },
    template::{
        Context, MAX_PLACEHOLDERS, MAX_TEMPLATE_LEN, NodeDto, NodeKindName, Output, RenderError,
        Scope, Template, TemplateError, ViewDto, ViewTemplate, ViewTemplateError,
    },
};

fn scope<'a>(declared: &'a [&'a str], settings: &'a SettingsSchema) -> Scope<'a> {
    Scope {
        declared,
        settings,
        item: false,
    }
}

fn check(text: &str, declared: &[&str]) -> Result<(), TemplateError> {
    let schema = text_schema("city");
    Template::parse(text)?.check(&scope(declared, &schema))
}

fn render(text: &str, snapshot: &crate::Snapshot) -> alloc::string::String {
    Template::parse(text).unwrap().render(&Context {
        snapshot,
        item: None,
    })
}

#[test]
fn literals_hold_escaped_braces_and_holes_are_resolved() {
    assert_eq!(render("{{a}} b", &snapshot()), "{a} b");
    assert_eq!(
        render("{clock.now|hh-mm}", &with_clock(snapshot())),
        "14:05"
    );
}

#[test]
fn malformed_braces_and_long_templates_are_refused() {
    for bad in ["{", "a}", "{a{b}}", "{clock.now"] {
        assert_eq!(Template::parse(bad), Err(TemplateError::Braces), "{bad}");
    }
    assert_eq!(
        Template::parse(&"x".repeat(MAX_TEMPLATE_LEN + 1)),
        Err(TemplateError::TooLong)
    );
    assert!(Template::parse(&"x".repeat(MAX_TEMPLATE_LEN)).is_ok());
}

#[test]
fn every_placeholder_must_resolve_when_the_package_is_read() {
    let declared = ["clock.now", "monitor.usage"];
    assert_eq!(check("{clock.now|hh-mm}", &declared), Ok(()));
    assert_eq!(
        check("{monitor.usage.cpu_percent|round(1)}", &declared),
        Ok(())
    );
    assert_eq!(check("{settings.city}", &declared), Ok(()));
    assert!(matches!(
        check("{clock.later}", &declared),
        Err(TemplateError::UnknownCapability(_))
    ));
    assert!(matches!(
        check("{notifications.post}", &declared),
        Err(TemplateError::UnknownCapability(_))
    ));
    assert!(matches!(
        check("{transfers.counts.active}", &declared),
        Err(TemplateError::Undeclared(_))
    ));
    assert!(matches!(
        check("{monitor.usage.nothing}", &declared),
        Err(TemplateError::UnknownField(_))
    ));
    assert!(matches!(
        check("{monitor.usage}", &declared),
        Err(TemplateError::NeedsField(_))
    ));
    assert!(matches!(
        check("{settings.missing}", &declared),
        Err(TemplateError::UnknownSetting(_))
    ));
    assert!(matches!(
        check("{item.text}", &declared),
        Err(TemplateError::Item(_))
    ));
    assert!(matches!(
        check("{clock.now}", &declared),
        Err(TemplateError::Output { .. })
    ));
}

#[test]
fn filters_are_a_closed_typed_set() {
    let declared = ["clock.now", "monitor.usage"];
    assert!(matches!(
        check("{clock.now|percent}", &declared),
        Err(TemplateError::FilterInput { .. })
    ));
    assert!(matches!(
        check("{monitor.usage.cpu_percent|hh-mm}", &declared),
        Err(TemplateError::FilterInput { .. })
    ));
    assert_eq!(
        check("{monitor.usage.cpu_percent|round(3)}", &declared),
        Err(TemplateError::Round)
    );
    for bad in [
        "round(1.5)",
        "round(-1)",
        "round",
        "upper",
        "threshold(5, 1)",
        "round(1",
        "threshold(1)",
    ] {
        let text = alloc::format!("{{monitor.usage.cpu_percent|{bad}}}");
        assert!(
            matches!(check(&text, &declared), Err(TemplateError::Filter(_))),
            "{bad}"
        );
    }
    assert!(matches!(
        check("{monitor.usage.cpu_percent|threshold(50, 80)}", &declared),
        Err(TemplateError::Output { .. })
    ));
}

#[test]
fn values_are_inserted_as_literal_text_and_never_parsed_again() {
    let mut snapshot = snapshot();
    snapshot.settings.insert(
        "city".to_string(),
        SettingValue::Text("{clock.now} {{x}}".to_string()),
    );
    assert_eq!(
        render("in {settings.city}", &snapshot),
        "in {clock.now} {{x}}"
    );
}

#[test]
fn unavailable_values_show_as_not_available() {
    let mut snapshot = snapshot();
    snapshot.observations.insert(
        "monitor.usage".to_string(),
        Err(crate::HostError::Unavailable),
    );
    assert_eq!(
        render("{monitor.usage.cpu_percent|round(0)}%", &snapshot),
        "n/a%"
    );
    assert_eq!(
        render("{clock.now|hh-mm}", &crate::tests::support::snapshot()),
        "n/a"
    );
}

#[test]
fn numbers_round_and_clock_times_wrap_the_day() {
    let mut snapshot = with_clock(snapshot());
    snapshot.observations.insert(
        "monitor.usage".to_string(),
        Ok(Observed::Usage(Usage {
            cpu_percent: 12.345,
            memory_percent: 99.5,
        })),
    );
    assert_eq!(
        render("{monitor.usage.cpu_percent|round(1)}", &snapshot),
        "12.3"
    );
    assert_eq!(
        render("{monitor.usage.memory_percent|percent}", &snapshot),
        "100%"
    );
}

fn nodes(list: Vec<NodeDto>) -> ViewDto {
    ViewDto {
        refresh: None,
        nodes: list,
    }
}

fn text(parent: Option<u16>, template: &str) -> NodeDto {
    NodeDto {
        kind: Some(NodeKindName::Text),
        parent,
        text: Some(template.to_string()),
        ..NodeDto::default()
    }
}

fn container(kind: NodeKindName, parent: Option<u16>) -> NodeDto {
    NodeDto {
        kind: Some(kind),
        parent,
        ..NodeDto::default()
    }
}

fn compile(dto: &ViewDto) -> Result<ViewTemplate, ViewTemplateError> {
    ViewTemplate::compile(
        dto,
        &["clock.now", "monitor.usage", "notifications.own"],
        &text_schema("city"),
    )
}

#[test]
fn a_view_draws_text_meter_and_tones_from_the_snapshot() {
    let dto = nodes(vec![
        container(NodeKindName::Row, None),
        text(Some(0), "{clock.now|hh-mm}"),
        NodeDto {
            kind: Some(NodeKindName::Meter),
            parent: Some(0),
            value: Some("monitor.usage.cpu_percent".into()),
            tone_when: Some("monitor.usage.cpu_percent|threshold(50, 80)".into()),
            ..NodeDto::default()
        },
    ]);
    let template = compile(&dto).unwrap();
    let mut snapshot = with_clock(snapshot());
    snapshot.observations.insert(
        "monitor.usage".to_string(),
        Ok(Observed::Usage(Usage {
            cpu_percent: 90.0,
            memory_percent: 1.0,
        })),
    );
    let view = template.render(&snapshot).unwrap();
    assert_eq!(view.nodes[1].kind, NodeKind::Text("14:05".into()));
    assert_eq!(view.nodes[2].kind, NodeKind::Meter(90));
    assert_eq!(view.nodes[2].tone, Tone::Caution);
    assert_eq!(view.nodes[2].parent, Some(0));
}

#[test]
fn drawn_text_is_sanitized_and_cut() {
    let template = compile(&nodes(vec![text(None, "{settings.city}")])).unwrap();
    let mut snapshot = snapshot();
    snapshot.settings.insert(
        "city".to_string(),
        SettingValue::Text(alloc::format!("a\u{202E}b{}", "z".repeat(500))),
    );
    let view = template.render(&snapshot).unwrap();
    let NodeKind::Text(shown) = &view.nodes[0].kind else {
        panic!()
    };
    assert!(shown.starts_with("ab") && shown.chars().count() == 200);
}

#[test]
fn authoring_mistakes_are_manifest_errors() {
    let bad = |list: Vec<NodeDto>| compile(&nodes(list)).unwrap_err();
    assert_eq!(
        bad(vec![NodeDto::default()]),
        ViewTemplateError::NoKind { index: 0 }
    );
    assert_eq!(
        bad(vec![text(Some(0), "a")]),
        ViewTemplateError::Parent { index: 0 }
    );
    assert_eq!(
        bad(vec![text(None, "a"), text(Some(0), "b")]),
        ViewTemplateError::Parent { index: 1 }
    );
    assert!(matches!(
        bad(vec![text(None, "{nope.x}")]),
        ViewTemplateError::Template { index: 0, .. }
    ));
    assert!(matches!(
        bad(vec![NodeDto {
            icon: Some(crate::PluginIcon::Puzzle),
            ..text(None, "a")
        }]),
        ViewTemplateError::Field { field: "icon", .. }
    ));
    assert!(matches!(
        bad(vec![container(NodeKindName::Button, None)]),
        ViewTemplateError::Missing { field: "text", .. }
    ));
    let button = NodeDto {
        text: Some("Go".into()),
        ..container(NodeKindName::Button, None)
    };
    assert!(matches!(
        bad(vec![button.clone()]),
        ViewTemplateError::Missing { field: "key", .. }
    ));
    let keyed = |key: &str| NodeDto {
        key: Some(key.into()),
        ..button.clone()
    };
    assert!(compile(&nodes(vec![keyed("go")])).is_ok());
    for key in ["has space", "UPPER", "", &"k".repeat(33)] {
        assert!(
            matches!(
                bad(vec![keyed(key)]),
                ViewTemplateError::Key { index: 0, .. }
            ),
            "{key:?}"
        );
    }
    assert_eq!(
        bad(vec![keyed("go"), keyed("go")]),
        ViewTemplateError::DuplicateKey { index: 1 }
    );
    assert!(compile(&nodes(vec![keyed("go"), keyed("stop")])).is_ok());
}

#[test]
fn refresh_depth_node_and_placeholder_bounds_hold() {
    let slow = ViewDto {
        refresh: Some(crate::Refresh::AfterMs(10)),
        nodes: vec![],
    };
    assert_eq!(compile(&slow).unwrap_err(), ViewTemplateError::Refresh);
    let deep: Vec<NodeDto> = (0..9)
        .map(|i| container(NodeKindName::Column, (i > 0).then(|| i - 1)))
        .collect();
    assert_eq!(
        compile(&nodes(deep)).unwrap_err(),
        ViewTemplateError::TooDeep { index: 8 }
    );
    let many: Vec<NodeDto> = (0..257)
        .map(|_| container(NodeKindName::Spacer, None))
        .collect();
    assert_eq!(
        compile(&nodes(many)).unwrap_err(),
        ViewTemplateError::TooManyNodes
    );
    let holes: Vec<NodeDto> = (0..=MAX_PLACEHOLDERS)
        .map(|_| text(None, "{clock.now|hh-mm}"))
        .collect();
    assert_eq!(
        compile(&nodes(holes)).unwrap_err(),
        ViewTemplateError::TooManyPlaceholders
    );
}

fn notices(count: usize) -> crate::Snapshot {
    let mut snapshot = snapshot();
    let items = (0..count)
        .map(|i| OwnNotice {
            tone: NoticeTone::Info,
            text: alloc::format!("n{i}"),
        })
        .collect();
    snapshot.observations.insert(
        "notifications.own".to_string(),
        Ok(Observed::Notices(OwnNotices(items))),
    );
    snapshot
}

fn each_list(rows: bool) -> ViewDto {
    let list = NodeDto {
        each: Some("notifications.own".into()),
        ..container(NodeKindName::List, None)
    };
    let mut list_nodes = vec![list];
    if rows {
        list_nodes.push(container(NodeKindName::Row, Some(0)));
        list_nodes.push(text(Some(1), "{item.text}"));
        list_nodes.push(container(NodeKindName::Spacer, Some(1)));
    } else {
        list_nodes.push(text(Some(0), "{item.text}"));
    }
    nodes(list_nodes)
}

#[test]
fn a_list_repeats_its_children_for_each_item() {
    let template = compile(&each_list(false)).unwrap();
    let view = template.render(&notices(3)).unwrap();
    let shown: Vec<_> = view.nodes.iter().map(|n| n.kind.clone()).collect();
    assert_eq!(
        shown,
        [
            NodeKind::List,
            NodeKind::Text("n0".into()),
            NodeKind::Text("n1".into()),
            NodeKind::Text("n2".into())
        ]
    );
    assert!(view.nodes[1..].iter().all(|n| n.parent == Some(0)));
    assert_eq!(template.render(&notices(0)).unwrap().nodes.len(), 1);
}

#[test]
fn list_expansion_counts_toward_the_node_limit() {
    let template = compile(&each_list(true)).unwrap();
    assert_eq!(
        template.render(&notices(50)).unwrap().nodes.len(),
        1 + 50 * 3
    );
    assert_eq!(
        template.render(&notices(100)).unwrap_err(),
        RenderError::TooManyNodes
    );
}

#[test]
fn item_names_and_repeated_controls_need_an_each_list() {
    let outside = compile(&nodes(vec![text(None, "{item.text}")]));
    assert!(matches!(outside, Err(ViewTemplateError::Template { .. })));
    let mut dto = each_list(false);
    dto.nodes.push(NodeDto {
        key: Some("x".into()),
        text: Some("Go".into()),
        ..container(NodeKindName::Button, Some(0))
    });
    assert_eq!(
        compile(&dto).unwrap_err(),
        ViewTemplateError::RepeatedKey { index: 2 }
    );
    let wrong = NodeDto {
        each: Some("clock.now".into()),
        ..container(NodeKindName::List, None)
    };
    assert_eq!(
        compile(&nodes(vec![wrong])).unwrap_err(),
        ViewTemplateError::Each { index: 0 }
    );
    assert_eq!(
        ViewTemplate::compile(&each_list(false), &["clock.now"], &text_schema("city")).unwrap_err(),
        ViewTemplateError::Each { index: 0 }
    );
}

#[test]
fn only_a_tone_expression_sets_a_tone() {
    let schema = text_schema("city");
    let declared = ["monitor.usage"];
    let hole =
        crate::template::Placeholder::parse("monitor.usage.cpu_percent|threshold(50, 80)").unwrap();
    assert_eq!(
        hole.expect(&scope(&declared, &schema), Output::Tone),
        Ok(())
    );
    assert!(
        hole.expect(&scope(&declared, &schema), Output::Number)
            .is_err()
    );
}

proptest! {
    #[test]
    fn parsing_never_panics_and_plain_text_renders_unchanged(text in "[^{}]{0,100}", junk in ".{0,60}") {
        let _ = Template::parse(&junk);
        let template = Template::parse(&text).unwrap();
        let snapshot = snapshot();
        prop_assert_eq!(template.render(&Context { snapshot: &snapshot, item: None }), text);
    }

    #[test]
    fn escaped_braces_round_trip(text in ".{0,80}") {
        let escaped = text.replace('{', "{{").replace('}', "}}");
        let snapshot = snapshot();
        let template = Template::parse(&escaped).unwrap();
        prop_assert_eq!(template.render(&Context { snapshot: &snapshot, item: None }), text);
    }
}
