//! Terminals a plugin drives: what reaches the screen, their bars, and toolbar contributions.

use alloc::string::String;

use super::support::{dto, host};
use crate::{
    ContributionError, LocalId, Manifest, ManifestError,
    capability::terminals::{Bar, Open, Write, terminal_output},
};
use nocterm_plugin_model::{ContributionKind, Edge};

#[test]
fn terminal_text_keeps_colours_and_line_breaks_and_drops_every_other_sequence() {
    assert_eq!(terminal_output("a\nb\tc"), "a\r\nb\tc");
    assert_eq!(
        terminal_output("\u{1b}[1;32mok\u{1b}[0m"),
        "\u{1b}[1;32mok\u{1b}[0m"
    );
    // Cursor moves, private modes, the title, a link and the clipboard are dropped whole.
    assert_eq!(terminal_output("x\u{1b}[2J\u{1b}[?25ly"), "xy");
    assert_eq!(terminal_output("\u{1b}]0;title\u{7}z"), "z");
    assert_eq!(terminal_output("\u{1b}]52;c;Y2xpcA==\u{1b}\\z"), "z");
    assert_eq!(terminal_output("\u{1b}]8;;https://x\u{1b}\\link"), "link");
    assert_eq!(terminal_output("\u{9b}31mq"), "q");
    // Direction overrides and zero-width characters never reach the screen.
    assert_eq!(terminal_output("a\u{202e}b\u{200b}c\r"), "abc");
}

#[test]
fn a_terminal_has_a_key_and_a_plain_title_and_a_write_is_bounded() {
    let local = LocalId::parse("servers").unwrap();
    let open = |terminal: &str, title: &str| Open {
        local: local.clone(),
        terminal: String::from(terminal),
        title: String::from(title),
        files: false,
    };
    assert!(open("srv-1a2b", "Build").check().is_ok());
    assert!(open("Bad Key", "Build").check().is_err());
    assert!(open("t", "a\u{202e}b").check().is_err());
    assert!(open("t", &"x".repeat(49)).check().is_err());
    let write = |text: String| Write {
        terminal: String::from("t"),
        text,
    };
    assert!(write("x".repeat(64 * 1024)).check().is_ok());
    assert!(write("x".repeat(64 * 1024 + 1)).check().is_err());
}

#[test]
fn a_bar_names_one_terminal_and_an_edge_or_none_to_take_it_away() {
    let bar = |terminal: &str, edge| Bar {
        terminal: String::from(terminal),
        edge,
    };
    assert!(bar("srv-1a2b", Some(crate::Edge::Top)).check().is_ok());
    assert!(bar("srv-1a2b", None).check().is_ok());
    assert!(bar("Bad Key", Some(crate::Edge::Left)).check().is_err());
    let taken: Bar = serde_json::from_str(r#"{"terminal":"t"}"#).unwrap();
    assert_eq!(taken.edge, None, "a bar without an edge is taken away");
    let shown: Bar = serde_json::from_str(r#"{"terminal":"t","edge":"right"}"#).unwrap();
    assert_eq!(shown.edge, Some(crate::Edge::Right));
}

const TOOLBAR: &str = r#"
api = "0.1"
runtime = "wasm"
publisher = "acme"
id = "bar"
name = "Acme Bar"

[[contribution]]
local = "controls"
kind = "toolbar"
title = "Controls"
edge = "bottom"

[[contribution]]
local = "list"
kind = "panel"
title = "List"
"#;

#[test]
fn a_toolbar_suggests_an_edge_and_waits_in_the_tray() {
    let manifest = Manifest::validate(&dto(TOOLBAR), &host()).expect("a valid package");
    let toolbar = &manifest.plugin().contributions()[0];
    assert_eq!(toolbar.kind(), ContributionKind::Toolbar);
    assert_eq!(toolbar.edge(), Some(Edge::Bottom));
    assert_eq!(
        toolbar.placement().auto,
        nocterm_plugin_model::AutoPlace::Tray
    );
    let defaulted = Manifest::validate(&dto(&TOOLBAR.replace("edge = \"bottom\"", "")), &host())
        .expect("the edge is optional");
    assert_eq!(
        defaulted.plugin().contributions()[0].edge(),
        Some(Edge::Top)
    );
}

#[test]
fn only_a_toolbar_names_an_edge() {
    let text = TOOLBAR.replace(
        "kind = \"panel\"\ntitle = \"List\"",
        "kind = \"panel\"\ntitle = \"List\"\nedge = \"left\"",
    );
    let error = Manifest::validate(&dto(&text), &host()).unwrap_err();
    assert!(
        matches!(
            error,
            ManifestError::Contribution {
                source: ContributionError::EdgeNotToolbar,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_toolbar_can_ask_to_be_shown_from_the_start_and_nothing_else_can() {
    let shown = TOOLBAR.replace("edge = \"bottom\"", "edge = \"bottom\"\nshown = true");
    let manifest = Manifest::validate(&dto(&shown), &host()).expect("a valid package");
    let toolbar = &manifest.plugin().contributions()[0];
    assert_eq!(toolbar.edge(), Some(Edge::Bottom));
    assert_eq!(
        toolbar.placement().auto,
        nocterm_plugin_model::AutoPlace::Show
    );
    let panel = TOOLBAR.replace(
        "kind = \"panel\"\ntitle = \"List\"",
        "kind = \"panel\"\ntitle = \"List\"\nshown = true",
    );
    let error = Manifest::validate(&dto(&panel), &host()).unwrap_err();
    assert!(
        matches!(
            error,
            ManifestError::Contribution {
                source: ContributionError::ShownNotToolbar,
                ..
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_toolbar_keeps_to_its_own_tabs_unless_it_names_all_and_has_one_such_toolbar() {
    let manifest = Manifest::validate(&dto(TOOLBAR), &host()).expect("a valid package");
    assert_eq!(
        manifest.plugin().contributions()[0].tabs(),
        Some(nocterm_plugin_model::ToolbarTabs::Own)
    );
    let all = TOOLBAR.replace("edge = \"bottom\"", "edge = \"bottom\"\ntabs = \"all\"");
    let manifest = Manifest::validate(&dto(&all), &host()).expect("a valid package");
    assert_eq!(
        manifest.plugin().contributions()[0].tabs(),
        Some(nocterm_plugin_model::ToolbarTabs::All)
    );
    let second = alloc::format!(
        "{TOOLBAR}\n[[contribution]]\nlocal = \"more\"\nkind = \"toolbar\"\ntitle = \"More\"\n"
    );
    let error = Manifest::validate(&dto(&second), &host()).unwrap_err();
    assert!(
        matches!(
            error,
            ManifestError::Contribution {
                source: ContributionError::SecondOwnTabsToolbar,
                ..
            }
        ),
        "{error:?}"
    );
    let panel = TOOLBAR.replace(
        "kind = \"panel\"\ntitle = \"List\"",
        "kind = \"panel\"\ntitle = \"List\"\ntabs = \"all\"",
    );
    assert!(matches!(
        Manifest::validate(&dto(&panel), &host()).unwrap_err(),
        ManifestError::Contribution {
            source: ContributionError::TabsNotToolbar,
            ..
        }
    ));
}
