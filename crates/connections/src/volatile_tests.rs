//! Connections kept only in memory, for a window whose list something else keeps.
use crate::{ConnectionInputs, EditorInputs, Installed, PreparedConnections, open_editor};
use gpui_kit::{AppContext as _, TestAppContext, WindowOptions, test::TestWindowExt as _};
use nocterm_connections_model::{
    draft::EditorDefaults,
    profiles::ProfileId,
    test_support::profile_id_for,
    value_ports::{ProfileIds, SuppliedValues},
};
use nocterm_store::{SettingsConfig, ShutdownCoordinator};
use nocterm_ui::DesignTokens;
use nocterm_workspace::Workspace;
use std::sync::Arc;

/// Hands out one identifier, standing in for the UUIDs of the application.
struct FixedId(ProfileId);
impl ProfileIds for FixedId {
    fn next(&self) -> ProfileId {
        self.0
    }
}

fn inputs(id: ProfileId) -> ConnectionInputs {
    let values = Arc::new(SuppliedValues::new(1, 0));
    ConnectionInputs::new(
        EditorInputs::new(
            EditorDefaults {
                user: Some("tester".into()),
                home: None,
            },
            Arc::new(FixedId(id)),
        ),
        values,
        crate::ObservationDeps::offline(),
    )
}

fn ready(cx: &mut gpui_kit::App) -> nocterm_ui::UiReady {
    gpui_kit::init(cx);
    nocterm_ui::init(
        DesignTokens::builtin(),
        SettingsConfig::in_memory(Default::default()).install(cx),
        cx,
    )
}

fn bindings(cx: &gpui_kit::App) -> usize {
    cx.key_bindings().borrow().bindings().count()
}

#[gpui_kit::test]
fn installing_volatile_connections_twice_adds_no_key_bindings(cx: &mut TestAppContext) {
    let (before, after) = cx.update(|cx| {
        let ready = ready(cx);
        crate::install(
            PreparedConnections::volatile(inputs(profile_id_for("a"))),
            &ready,
            &ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let before = bindings(cx);
        for _ in 0..2 {
            crate::install_volatile(
                PreparedConnections::volatile(inputs(profile_id_for("a"))),
                &ready,
                cx,
            )
            .unwrap();
        }
        (before, bindings(cx))
    });
    assert_eq!(before, after);
}

#[gpui_kit::test]
fn a_new_connection_in_volatile_connections_takes_the_supplied_identifier(cx: &mut TestAppContext) {
    let id = profile_id_for("fresh");
    let (handle, workspace, installed): (_, _, Installed) = cx.update(|cx| {
        cx.set_reduce_motion(true);
        let ready = ready(cx);
        let installed =
            crate::install_volatile(PreparedConnections::volatile(inputs(id)), &ready, cx).unwrap();
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps = nocterm_workspace::test_support::deps_for_core(&[], &[], cx);
                    Workspace::new(deps, window, cx)
                })
            })
            .unwrap();
        (window, workspace, installed)
    });
    cx.update_window(handle, |_, window, cx| {
        open_editor(installed.clone(), None, workspace.downgrade(), window, cx);
        window.render_frame(cx);
        window.input("host.test", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.read(|cx| {
        let connections = installed.connections().read(cx);
        assert!(connections.profiles(cx).get(id).is_some());
        assert_eq!(connections.profiles(cx).iter().count(), 1);
    });
}
