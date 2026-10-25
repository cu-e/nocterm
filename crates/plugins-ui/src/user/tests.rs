//! The User section asks before it installs, and acts on installed plugins through its port.

use std::{
    borrow::Cow,
    cell::RefCell,
    rc::Rc,
    sync::{Arc, LazyLock},
    time::Duration,
};

use gpui_kit::{
    App, AppContext as _, Entity, Subscription, TestAppContext, WindowOptions,
    test::TestWindowExt as _,
};
use nocterm_core::error::SharedCause;
use nocterm_plugin_model::{FieldGroup, FieldKind, FieldSpec, PluginId, SettingsSchema};
use nocterm_ui::{
    FakePluginSettings, PluginSettingsDirectory, PluginSettingsSource, SchemaRef,
    UserPluginSettings,
};

use super::{port::describe, *};

#[derive(Debug, PartialEq, Eq)]
enum Call {
    Install(InstallRequest),
    Enabled(PluginId, bool),
    Placeable(PluginId, bool),
    Remove(PluginId, bool),
    RemoveData(PluginId),
}

struct FakeUserPlugins {
    rows: RefCell<Vec<UserPluginRow>>,
    candidate: RefCell<Result<Candidate, UserPluginError>>,
    dev: RefCell<Option<String>>,
    calls: RefCell<Vec<Call>>,
    changed: Entity<()>,
}

impl FakeUserPlugins {
    fn calls(&self) -> Vec<Call> {
        self.calls.take()
    }
}

impl UserPlugins for Rc<FakeUserPlugins> {
    fn rows(&self, _: &App) -> Vec<UserPluginRow> {
        self.rows.borrow().clone()
    }
    fn audit(&self, _: &App) -> Vec<AuditLine> {
        vec![
            AuditLine {
                plugin: PluginId::owned("user-acme-clock"),
                action: "installed".into(),
            },
            AuditLine {
                plugin: PluginId::owned("user-acme-clock"),
                action: "disabled".into(),
            },
        ]
    }
    fn dev_location(&self, _: &App) -> Option<String> {
        self.dev.borrow().clone()
    }
    fn inspect(&self, _: &str, _: &App) -> Result<Candidate, UserPluginError> {
        self.candidate.borrow().clone()
    }
    fn install(&self, request: InstallRequest, _: &mut App) -> Result<(), UserPluginError> {
        self.calls.borrow_mut().push(Call::Install(request));
        Ok(())
    }
    fn set_enabled(
        &self,
        plugin: &PluginId,
        enabled: bool,
        _: &mut App,
    ) -> Result<(), UserPluginError> {
        self.calls
            .borrow_mut()
            .push(Call::Enabled(plugin.clone(), enabled));
        Ok(())
    }
    fn set_placeable(
        &self,
        plugin: &PluginId,
        placeable: bool,
        _: &mut App,
    ) -> Result<(), UserPluginError> {
        self.calls
            .borrow_mut()
            .push(Call::Placeable(plugin.clone(), placeable));
        Ok(())
    }
    fn remove(
        &self,
        plugin: &PluginId,
        with_data: bool,
        _: &mut App,
    ) -> Result<(), UserPluginError> {
        self.calls
            .borrow_mut()
            .push(Call::Remove(plugin.clone(), with_data));
        Err(UserPluginError::Manage(SharedCause::new(
            std::io::Error::other("disk full"),
        )))
    }
    fn remove_data(&self, plugin: &PluginId, _: &mut App) -> Result<(), UserPluginError> {
        self.calls
            .borrow_mut()
            .push(Call::RemoveData(plugin.clone()));
        Ok(())
    }
    fn observe(&self, cx: &mut App, mut on_change: Box<dyn FnMut(&mut App)>) -> Subscription {
        cx.observe(&self.changed, move |_, cx| on_change(cx))
    }
}

static SCHEMA: LazyLock<SettingsSchema> = LazyLock::new(|| SettingsSchema {
    version: 1,
    groups: Cow::Owned(vec![FieldGroup {
        title: Cow::Borrowed("Display"),
        contribution: None,
        fields: Cow::Owned(vec![FieldSpec {
            renamed_from: None,
            key: Cow::Borrowed("verbose"),
            label: Cow::Borrowed("Verbose"),
            help: None,
            kind: FieldKind::Toggle { default: false },
        }]),
    }]),
});

struct OneUser(PluginId, Rc<dyn PluginSettingsSource>);

impl UserPluginSettings for OneUser {
    fn find(
        &self,
        plugin: &PluginId,
        _: &App,
    ) -> Option<(SchemaRef, Rc<dyn PluginSettingsSource>)> {
        (plugin == &self.0).then(|| (SchemaRef::User(Arc::new(SCHEMA.clone())), self.1.clone()))
    }
}

fn id() -> PluginId {
    PluginId::owned("user-acme-clock")
}

fn row() -> UserPluginRow {
    UserPluginRow {
        id: id(),
        name: "Clock".into(),
        publisher: "acme".into(),
        version: "1.0.0".into(),
        fingerprint: "0123456789abcdef".into(),
        state: RowState::Ready,
        enabled: true,
        placeable: true,
        granted: vec!["Read the time".into()],
        contributions: vec!["Clock".into()],
    }
}

fn candidate() -> Candidate {
    Candidate {
        location: "/tmp/clock.toml".into(),
        id: id(),
        name: "Clock".into(),
        publisher: "acme".into(),
        version: "1.0.0".into(),
        hash: "ab".repeat(32),
        fingerprint: "abababababababab".into(),
        capabilities: vec![
            CapabilityLine {
                id: "clock.now".into(),
                description: "Read the time".into(),
                reason: Some("x".repeat(400)),
                required: true,
                added: false,
            },
            CapabilityLine {
                id: "monitor.usage".into(),
                description: "Read CPU and memory use".into(),
                reason: None,
                required: false,
                added: false,
            },
        ],
        fields: vec![FieldLine {
            label: "Verbose".into(),
            added: false,
        }],
        replaces: None,
        runs_code: false,
        dev: false,
    }
}

struct Fixture {
    window: gpui_kit::AnyWindowHandle,
    section: Entity<UserPluginsSection>,
    plugins: Rc<FakeUserPlugins>,
}

fn open(
    rows: Vec<UserPluginRow>,
    candidate: Result<Candidate, UserPluginError>,
    cx: &mut TestAppContext,
) -> Fixture {
    cx.update(|cx| {
        gpui_kit::init(cx);
        let store = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), store, cx);
        let plugins = Rc::new(FakeUserPlugins {
            rows: RefCell::new(rows),
            candidate: RefCell::new(candidate),
            dev: RefCell::default(),
            calls: RefCell::default(),
            changed: cx.new(|_| ()),
        });
        let values: Rc<dyn PluginSettingsSource> = Rc::new(FakePluginSettings::new(
            Box::leak(Box::new(SCHEMA.clone())),
            cx,
        ));
        let directory = PluginSettingsDirectory::new().with_user(Rc::new(OneUser(id(), values)));
        let manager: Rc<dyn UserPlugins> = Rc::new(plugins.clone());
        let (window, section) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
            cx.new(|cx| UserPluginsSection::new(manager, directory, cx))
        })
        .unwrap();
        Fixture {
            window,
            section,
            plugins,
        }
    })
}

fn frame(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    check: impl FnOnce(&mut gpui_kit::Window, &mut App),
) {
    cx.update_window(fixture.window, |_, window, cx| {
        window.render_frame(cx);
        check(window, cx);
    })
    .unwrap();
}

fn begin(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.section.update(cx, |section, cx| {
            section.begin_install("/tmp/clock.toml", window, cx)
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_consent_names_the_unverified_publisher_and_quotes_the_author_briefly(
    cx: &mut TestAppContext,
) {
    let fixture = open(vec![], Ok(candidate()), cx);
    begin(&fixture, cx);
    frame(&fixture, cx, |window, _| {
        assert_eq!(window.find("user-consent").label(), Some("Install Clock?"));
        let required = window
            .find("user-consent-capability:clock.now")
            .label()
            .unwrap()
            .to_owned();
        assert!(required.starts_with("Read the time | Author says (unverified): \u{201c}"));
        let quoted = required.split('\u{201c}').nth(1).unwrap();
        assert_eq!(
            quoted.trim_end_matches('\u{201d}').chars().count(),
            161,
            "160 and an ellipsis"
        );
        assert!(
            window
                .try_find("user-consent-capability:monitor.usage")
                .is_some()
        );
    });
}

#[gpui_kit::test]
fn the_consent_is_inert_for_a_moment_and_denying_is_the_default(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    begin(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-consent-state").label(), Some("inert"));
        let deny = fixture.section.read(cx).deny_focus().clone();
        assert!(deny.is_focused(window), "Deny holds focus");
        window.click("user-consent-accept", cx);
    });
    cx.run_until_parked();
    assert!(
        fixture.plugins.calls().is_empty(),
        "an early click installs nothing"
    );

    cx.executor().advance_clock(Duration::from_millis(751));
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-consent-state").label(), Some("ready"));
        window.click("user-consent-accept", cx);
    });
    cx.run_until_parked();
    assert_eq!(
        fixture.plugins.calls(),
        vec![Call::Install(InstallRequest {
            location: "/tmp/clock.toml".into(),
            hash: "ab".repeat(32),
            replaces: None,
            accepted: vec![],
            keep_data: false,
        })],
        "optional capabilities start off"
    );
}

#[gpui_kit::test]
fn an_allowed_optional_capability_is_part_of_the_request(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    begin(&fixture, cx);
    cx.executor().advance_clock(Duration::from_millis(751));
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-allow:monitor.usage", cx);
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-accept", cx)
    });
    cx.run_until_parked();
    let calls = fixture.plugins.calls();
    assert!(
        matches!(&calls[..], [Call::Install(request)] if request.accepted == ["monitor.usage"])
    );
}

#[gpui_kit::test]
fn denying_closes_the_consent_and_installs_nothing(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    begin(&fixture, cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-deny", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-consent").is_none());
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn a_package_the_port_refuses_shows_why_and_opens_no_consent(cx: &mut TestAppContext) {
    let fixture = open(
        vec![],
        Err(UserPluginError::Package(SharedCause::new(
            std::io::Error::other("The manifest is not valid."),
        ))),
        cx,
    );
    begin(&fixture, cx);
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-notice").label(),
            Some("The package was refused: The manifest is not valid.")
        );
        assert!(window.try_find("user-consent").is_none());
    });
}

fn replacing(downgrade: bool) -> Candidate {
    Candidate {
        replaces: Some(Replaced {
            version: "2.0.0".into(),
            hash: "11".repeat(32),
            fingerprint: "1111111111111111".into(),
            downgrade,
            runs_code: false,
        }),
        ..candidate()
    }
}

/// What the consent says about running code for `candidate`.
fn code_line(candidate: Candidate, cx: &mut TestAppContext) -> String {
    let fixture = open(vec![], Ok(candidate), cx);
    begin(&fixture, cx);
    let mut text = String::new();
    frame(&fixture, cx, |window, _| {
        text = window
            .find("user-consent-code")
            .label()
            .expect("the consent states whether the package runs code")
            .to_owned();
    });
    text
}

#[gpui_kit::test]
fn the_consent_always_says_whether_the_package_runs_code(cx: &mut TestAppContext) {
    assert_eq!(code_line(candidate(), cx), "No");
    let code = Candidate {
        runs_code: true,
        ..candidate()
    };
    assert!(code_line(code, cx).starts_with("Yes: a WebAssembly module"));
}

#[gpui_kit::test]
fn an_update_from_a_template_to_code_marks_the_change_and_the_reverse_says_so_too(
    cx: &mut TestAppContext,
) {
    let from_template = Candidate {
        runs_code: true,
        ..replacing(false)
    };
    assert_eq!(
        code_line(from_template, cx),
        "Yes, new: the installed version runs no code"
    );

    let mut from_code = replacing(false);
    from_code.replaces.as_mut().unwrap().runs_code = true;
    assert_eq!(
        code_line(from_code.clone(), cx),
        "No: the installed version runs code"
    );

    from_code.runs_code = true;
    assert!(code_line(from_code, cx).starts_with("Yes: a WebAssembly module"));
}

#[gpui_kit::test]
fn an_update_names_both_versions_and_keeps_the_data_only_when_asked(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(replacing(false)), cx);
    begin(&fixture, cx);
    cx.executor().advance_clock(Duration::from_millis(751));
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-consent").label(),
            Some("Replace Clock (unverified publisher)")
        );
        assert_eq!(window.find("user-consent-keep").checked(), Some(false));
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-accept", cx)
    });
    cx.run_until_parked();
    assert!(matches!(&fixture.plugins.calls()[..], [Call::Install(request)] if !request.keep_data));
}

#[gpui_kit::test]
fn an_update_names_the_installed_package_it_replaces(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(replacing(false)), cx);
    begin(&fixture, cx);
    cx.executor().advance_clock(Duration::from_millis(751));
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-accept", cx)
    });
    cx.run_until_parked();

    assert!(
        matches!(&fixture.plugins.calls()[..],
            [Call::Install(request)] if request.replaces == Some("11".repeat(32))),
        "the request carries the hash it replaces, so a window that lost the race is refused"
    );
}

#[test]
fn an_error_keeps_the_cause_the_port_gave_it() {
    let error = UserPluginError::Manage(SharedCause::new(std::io::Error::other("disk full")));

    let source = std::error::Error::source(&error).expect("the cause is retained");

    assert_eq!(source.to_string(), "disk full");
    assert_eq!(describe(&error), "The change was refused: disk full");
}

#[gpui_kit::test]
fn a_broken_plugin_shows_why_it_does_not_run(cx: &mut TestAppContext) {
    let mut broken = row();
    broken.state = RowState::Broken(SharedCause::new(std::io::Error::other(
        "the stored package belongs to another plugin",
    )));
    let fixture = open(vec![broken], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });

    frame(&fixture, cx, |window, _| {
        assert!(
            window
                .try_find("user-chip:user-acme-clock:Broken")
                .is_some()
        );
    });
}

#[gpui_kit::test]
fn a_downgrade_waits_for_confirmation(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(replacing(true)), cx);
    begin(&fixture, cx);
    cx.executor().advance_clock(Duration::from_millis(751));
    cx.run_until_parked();
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-accept", cx)
    });
    cx.run_until_parked();
    assert!(fixture.plugins.calls().is_empty());
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-downgrade", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-consent-accept", cx)
    });
    cx.run_until_parked();
    assert_eq!(fixture.plugins.calls().len(), 1);
}

#[gpui_kit::test]
fn the_list_shows_state_and_the_detail_acts_through_the_port(cx: &mut TestAppContext) {
    let mut off = row();
    off.enabled = false;
    off.placeable = false;
    let fixture = open(vec![off], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        assert!(
            window
                .try_find("user-chip:user-acme-clock:Disabled")
                .is_some()
        );
        assert!(
            window
                .try_find("user-chip:user-acme-clock:Not placed")
                .is_some()
        );
        window.click("user-plugin-row:user-acme-clock", cx);
    });
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-plugin-detail").label(), Some("Clock"));
        window.click("user-plugin-enabled", cx);
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-placeable", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove-data", cx)
    });
    assert_eq!(
        fixture.plugins.calls(),
        vec![
            Call::Enabled(id(), true),
            Call::Placeable(id(), true),
            Call::RemoveData(id()),
        ]
    );
}

#[gpui_kit::test]
fn a_refused_removal_says_why(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-remove", cx)
    });
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-notice").label(),
            Some("The change was refused: disk full")
        );
    });
}

#[gpui_kit::test]
fn the_detail_shows_the_unverified_publisher_and_the_form_in_the_plugin_frame(
    cx: &mut TestAppContext,
) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture
            .section
            .update(cx, |section, cx| section.select(&id(), window, cx));
    })
    .unwrap();
    frame(&fixture, cx, |window, _| {
        assert!(
            window
                .try_find("plugin-settings-frame:user-acme-clock")
                .is_some()
        );
        assert_eq!(
            window.find("plugin-settings-frame:user-acme-clock").label(),
            Some("Plugin \u{b7} Clock")
        );
    });
}

#[gpui_kit::test]
fn the_audit_log_lists_the_newest_first(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, _| {
        assert_eq!(
            window.find("user-audit:0").label(),
            Some("user-acme-clock disabled")
        );
        assert_eq!(
            window.find("user-audit:1").label(),
            Some("user-acme-clock installed")
        );
    });
}

#[gpui_kit::test]
fn a_plugin_that_goes_away_leaves_no_selection(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    fixture.plugins.rows.borrow_mut().clear();
    cx.update(|cx| fixture.plugins.changed.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-plugin-detail").is_none());
    });
}

#[gpui_kit::test]
fn a_replaced_plugin_does_not_keep_the_form_of_its_old_package(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });
    cx.update(|cx| assert!(fixture.section.read(cx).form().is_some()));

    cx.update(|cx| {
        fixture.plugins.rows.borrow_mut()[0].fingerprint = "fedcba9876543210".into();
        fixture.plugins.changed.update(cx, |_, cx| cx.notify());
    });
    cx.run_until_parked();

    cx.update(|cx| {
        let section = fixture.section.read(cx);
        assert!(section.form().is_none(), "the old form is dropped");
        assert!(section.selected().is_none());
    });
}

#[gpui_kit::test]
fn an_unchanged_plugin_keeps_its_form_when_the_list_changes(cx: &mut TestAppContext) {
    let fixture = open(vec![row()], Ok(candidate()), cx);
    frame(&fixture, cx, |window, cx| {
        window.click("user-plugin-row:user-acme-clock", cx)
    });

    cx.update(|cx| fixture.plugins.changed.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();

    cx.update(|cx| assert!(fixture.section.read(cx).form().is_some()));
}

#[gpui_kit::test]
fn there_is_no_developer_banner_without_a_dev_location(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-dev").is_none());
        assert!(window.try_find("user-dev-load").is_none());
    });
}

#[gpui_kit::test]
fn a_developer_session_offers_its_file_and_the_consent_says_dev(cx: &mut TestAppContext) {
    let fixture = open(
        vec![],
        Ok(Candidate {
            dev: true,
            ..candidate()
        }),
        cx,
    );
    fixture.plugins.dev.replace(Some("/work/clock.wasm".into()));
    fixture.section.update(cx, |_, cx| cx.notify());
    frame(&fixture, cx, |window, cx| {
        assert_eq!(window.find("user-dev").label(), Some("Developer session"));
        assert_eq!(
            window.find("user-dev-load").label(),
            Some("Load clock.wasm")
        );
        window.click("user-dev-load", cx);
    });
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-consent").is_some());
        assert_eq!(window.find("user-consent-dev").label(), Some("DEV"));
    });
    assert!(fixture.plugins.calls().is_empty());
}

#[gpui_kit::test]
fn the_consent_of_a_package_outside_a_developer_session_has_no_dev_label(cx: &mut TestAppContext) {
    let fixture = open(vec![], Ok(candidate()), cx);
    begin(&fixture, cx);
    frame(&fixture, cx, |window, _| {
        assert!(window.try_find("user-consent").is_some());
        assert!(window.try_find("user-consent-dev").is_none());
    });
}
