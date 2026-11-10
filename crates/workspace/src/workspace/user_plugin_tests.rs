//! The panels and widgets of installed user plugins: they register against the set in force,
//! stay out of reach until the user approved placing them and follow the plugin's enablement.
//! What a plugin may ask of its panels (open without taking focus, never replace a panel the
//! user has open) is proven through the window's provider in [`provider`].
use std::{cell::Cell, cell::RefCell, rc::Rc, sync::Arc};

use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, IntoElement, Render, TestAppContext,
    TestSupportExt as _, Window, div, prelude::*, px, size, test::TestWindowExt as _,
};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, Icon, PluginId, PluginSet, SettingsSchema, UserContribution,
    UserEntry, UserPlugin, Zone,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::SettingsConfig;
use nocterm_ui::PluginSetHandle;
use nocterm_window_layout::LayoutSettings;

use super::{
    PanelRegistration, WidgetRegistration, Workspace,
    status_bar_centre_tests::{layout, say_much},
};
use crate::{
    plugin,
    test_support::{EXPANDABLE, PANEL_A, TEST_PLUGIN, TestPanel, Toggle, toggle_id},
    workspace::tests::fixture_with_deps,
};

const PANEL: &str = "user-acme-notes.panel";
const WIDGET: &str = "user-acme-notes.clock";

fn entry(enabled: bool, placeable: bool) -> UserEntry {
    let id = PluginId::user("acme", "notes").expect("a valid user id");
    let contribution = |local: &str, kind, title: &str| {
        UserContribution::clamp(
            ContributionId::in_plugin(&id, local).unwrap(),
            kind,
            title,
            Icon::Plug,
            Zone::Right,
            1,
        )
        .unwrap()
    };
    UserEntry {
        plugin: UserPlugin::new(
            id.clone(),
            "notes",
            "Keeps notes",
            vec![
                contribution("panel", ContributionKind::Panel, "Notes"),
                contribution("clock", ContributionKind::Widget, "Time"),
            ],
            SettingsSchema::NONE,
        )
        .unwrap(),
        enabled,
        placeable,
    }
}

fn set(revision: u64, entries: Vec<UserEntry>) -> PluginSet {
    PluginSet::system(Arc::new(
        plugin::catalog(&[], &[&TEST_PLUGIN]).expect("the test catalog is valid"),
    ))
    .with_users(entries, revision)
    .expect("the user plugins form a valid set")
}

fn panel_id() -> ContributionId {
    ContributionId::owned(PANEL)
}

/// A window whose layout places a user panel in the sidebar group and a user widget on the bar.
struct Rig {
    handle: AnyWindowHandle,
    workspace: Entity<Workspace>,
    source: PluginSetHandle,
    made: Rc<Cell<usize>>,
    side_panel: Entity<TestPanel>,
}

fn stored() -> LayoutSettings {
    layout(
        serde_json::json!({
            "left": [{
                "group": "navigation",
                "primary": true,
                "members": ["test.panel-a", PANEL],
            }],
            "center": [],
            "right": [
                { "item": "workspace.notifications" },
                { "item": WIDGET },
                { "group": "assistant", "members": ["test.expandable"] },
            ],
        }),
        &[],
    )
}

fn rig(cx: &mut TestAppContext, width: f32, first: PluginSet) -> Rig {
    let kept = Rc::new(RefCell::new(None));
    let shared = kept.clone();
    let (handle, workspace, _) = fixture_with_deps(cx, move |layout, cx| {
        let mut document = SettingsDocument::default();
        document.register::<LayoutSettings>();
        document.set(stored());
        let settings = SettingsConfig::in_memory(document).install(cx);
        let source = PluginSetHandle::new(first, cx);
        *shared.borrow_mut() = Some(source.clone());
        crate::Deps::new(layout)
            .with_plugin_source(Rc::new(source))
            .with_settings(settings)
    });
    let source = kept.borrow_mut().take().expect("the source was composed");
    let made = Rc::new(Cell::new(0));
    let side_panel = cx
        .update_window(handle, |_, window, cx| {
            window.resize(size(px(width), px(700.)));
            workspace.update(cx, |workspace, cx| {
                let sidebar = TestPanel::builder("panel-a").build(cx);
                workspace
                    .register_panel(
                        PanelRegistration::new(PANEL_A, move |_, _, _| sidebar.clone()),
                        window,
                        cx,
                    )
                    .expect("the test panel registers");
                let side_panel = TestPanel::builder("side").title("Side").build(cx);
                let hosted = side_panel.clone();
                workspace
                    .register_panel(
                        PanelRegistration::new(EXPANDABLE, move |_, _, _| hosted.clone())
                            .expandable(),
                        window,
                        cx,
                    )
                    .expect("the side panel registers");
                register_user(&made, workspace, window, cx);
                side_panel
            })
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.run_until_parked();
    Rig {
        handle,
        workspace,
        source,
        made,
        side_panel,
    }
}

fn register_user(
    made: &Rc<Cell<usize>>,
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let made = made.clone();
    let _ = workspace.register_panel(
        PanelRegistration::user(&panel_id(), move |_, _, cx| {
            made.set(made.get() + 1);
            TestPanel::builder("user-panel").title("Notes").build(cx)
        }),
        window,
        cx,
    );
    let _ = workspace.register_widget(
        WidgetRegistration::user(&ContributionId::owned(WIDGET), |_, _, cx| {
            cx.new(|_| Wide(600.))
        }),
        window,
        cx,
    );
}

/// A widget of a width the test chooses.
struct Wide(f32);
impl Render for Wide {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("user-widget")
            .test_support()
            .w(px(self.0))
            .h(px(20.))
    }
}

fn publish(cx: &mut TestAppContext, rig: &Rig, next: PluginSet) {
    cx.update(|cx| rig.source.replace(next, cx));
    cx.run_until_parked();
}

fn settle(cx: &mut TestAppContext, rig: &Rig) {
    for _ in 0..4 {
        cx.update_window(rig.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

/// Whether the user panel is registered, enabled and open.
fn state(cx: &mut TestAppContext, rig: &Rig) -> (bool, bool, bool) {
    cx.update(|cx| {
        let workspace = rig.workspace.read(cx);
        (
            workspace.is_registered(&panel_id()),
            workspace.panel_is_enabled(&panel_id()),
            workspace.panel_is_open(&panel_id()),
        )
    })
}

/// Opens the panel the way the user does, from the menu, and says whether it shows.
fn open(cx: &mut TestAppContext, rig: &Rig) -> bool {
    cx.update_window(rig.handle, |_, window, cx| {
        rig.workspace.update(cx, |workspace, cx| {
            workspace.open_panel(&panel_id(), window, cx)
        })
    })
    .unwrap();
    settle(cx, rig);
    state(cx, rig).2
}

fn labels(cx: &mut TestAppContext, rig: &Rig) -> Vec<String> {
    cx.update(|cx| {
        rig.workspace
            .read(cx)
            .panel_menu()
            .concat()
            .iter()
            .map(|panel| panel.label())
            .collect()
    })
}

#[gpui_kit::test]
fn a_user_panel_cannot_open_until_its_package_may_be_placed(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, false)]));

    assert!(!open(cx, &rig), "an unapproved panel stays closed");
    assert!(!labels(cx, &rig).iter().any(|label| label.contains("notes")));

    publish(cx, &rig, set(2, vec![entry(true, true)]));

    assert!(open(cx, &rig), "the approved panel opens");
}

#[gpui_kit::test]
fn the_menu_names_the_plugin_before_a_user_panel(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));

    let labels = labels(cx, &rig);

    assert!(
        labels.contains(&"Plugin \u{b7} notes \u{2014} Notes".to_owned()),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn a_disabled_user_plugin_creates_nothing_until_it_is_enabled(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(false, true)]));

    assert_eq!(rig.made.get(), 0, "no panel was instantiated");
    assert!(!state(cx, &rig).1);
    assert!(!open(cx, &rig));
    assert_eq!(rig.made.get(), 0);

    publish(cx, &rig, set(2, vec![entry(true, true)]));
    settle(cx, &rig);
    assert!(open(cx, &rig));
    assert_eq!(rig.made.get(), 1);

    publish(cx, &rig, set(3, vec![entry(false, true)]));
    settle(cx, &rig);

    assert!(!state(cx, &rig).2, "disabling closes the panel");
    assert!(!state(cx, &rig).1);
}

#[gpui_kit::test]
fn a_user_widget_is_held_to_one_hundred_and_sixty_pixels(cx: &mut TestAppContext) {
    let rig = rig(cx, 1400., set(1, vec![entry(true, true)]));
    settle(cx, &rig);

    let width = cx
        .update_window(rig.handle, |_, window, _| {
            window.find("user-widget").bounds().size.width
        })
        .unwrap();

    assert!(width <= px(160.), "the widget was drawn {width:?} wide");
}

#[gpui_kit::test]
fn a_wide_user_widget_leaves_the_notice_line_its_room_at_520_pixels(cx: &mut TestAppContext) {
    let rig = rig(cx, 520., set(1, vec![entry(true, true)]));
    cx.update_window(rig.handle, |_, window, cx| {
        say_much(window, cx);
        rig.workspace.update(cx, |_, cx| cx.notify());
    })
    .unwrap();
    settle(cx, &rig);

    let (text, bell) = cx
        .update_window(rig.handle, |_, window, _| {
            (
                window.find("notice-text").bounds(),
                window.find("notice-toggle").bounds(),
            )
        })
        .unwrap();

    assert!(
        text.size.width + bell.size.width >= px(96.),
        "{text:?} and {bell:?}"
    );
}

#[gpui_kit::test]
fn a_user_group_and_a_pending_approval_leave_the_assistant_toggle_in_the_bar(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx, 520., set(1, vec![entry(true, true)]));
    cx.update_window(rig.handle, |_, _, cx| {
        rig.side_panel.update(cx, |panel, cx| {
            panel.set_attention(Some("approval pending".into()), cx)
        });
    })
    .unwrap();
    settle(cx, &rig);

    let drawn = cx
        .update_window(rig.handle, |_, window, _| {
            window.try_find(toggle_id(Toggle::SidePanel)).is_some()
        })
        .unwrap();

    assert!(drawn, "the approval toggle stays in the bar");
}

#[cfg(feature = "user-plugins")]
mod provider;
