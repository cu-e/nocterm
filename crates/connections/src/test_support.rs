use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use nocterm_ui::DesignTokens;
use nocterm_workspace::{SessionSpec, Workspace};

use crate::{Connections, FactsPrepared, Installed, ObservationDeps};

/// The session for a profile that jumps through nothing.
pub(crate) fn direct_spec(profile: &nocterm_connections_model::profiles::Profile) -> SessionSpec {
    crate::spec_for_profile(profile, &Default::default()).unwrap()
}

pub(crate) type Opened = Rc<RefCell<Vec<SessionSpec>>>;

pub(crate) fn workspace(
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<Workspace>, Opened, Installed) {
    let opened = Opened::default();
    let calls = opened.clone();
    let (window, workspace, installed) = cx.update(|cx| {
        // Keep dialog hitboxes stable between rendering and simulated clicks.
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            DesignTokens::builtin(),
            (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let installed = crate::install(
            crate::PreparedConnections::memory(nocterm_connections_model::draft::EditorDefaults {
                user: Some("tester".into()),
                home: None,
            }),
            &ready,
            &nocterm_store::ShutdownCoordinator::default(),
            cx,
        )
        .unwrap();
        let (window, workspace) =
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let deps = nocterm_workspace::test_support::deps_for_core(
                        &[crate::plugin::CORE],
                        &[],
                        cx,
                    );
                    let mut workspace = Workspace::new(deps, window, cx);
                    workspace
                        .set_session_opener(move |_, spec, _, _| calls.borrow_mut().push(spec));
                    workspace
                })
            })
            .unwrap();
        (window, workspace, installed)
    });
    assert!(cx.read(|cx| {
        installed
            .connections()
            .read(cx)
            .profiles(cx)
            .iter()
            .next()
            .is_none()
    }));
    (window, workspace, opened, installed)
}

/// Explicit alternate owners for tests; preserves the supplied settings installation.
pub(crate) fn install_owners(
    connections: Connections,
    facts: FactsPrepared,
    settings: nocterm_store::SettingsHandle,
    cx: &mut gpui_kit::App,
) -> Installed {
    let executor = cx.background_executor().clone();
    install_owners_with_executor(connections, facts, settings, executor, cx)
}
pub(crate) fn install_owners_with_executor(
    connections: Connections,
    facts: FactsPrepared,
    settings: nocterm_store::SettingsHandle,
    executor: gpui_kit::BackgroundExecutor,
    cx: &mut gpui_kit::App,
) -> Installed {
    let inputs = model_inputs(&connections, cx);
    Installed::from_owners_with_executor(
        connections,
        facts,
        inputs,
        settings,
        executor,
        &nocterm_store::ShutdownCoordinator::default(),
        cx,
    )
    .unwrap()
}
pub(crate) fn install_model(
    connections: Connections,
    installed: &Installed,
    cx: &mut gpui_kit::App,
) -> Installed {
    install_owners(
        connections,
        FactsPrepared::in_memory(ObservationDeps::offline()),
        installed.settings().clone(),
        cx,
    )
}

fn model_inputs(connections: &Connections, cx: &gpui_kit::App) -> crate::EditorInputs {
    let values = std::sync::Arc::new(
        nocterm_connections_model::value_ports::SuppliedValues::excluding(
            1,
            0,
            connections.profiles(cx).iter().map(|p| p.id),
        ),
    );
    crate::EditorInputs::new(
        nocterm_connections_model::draft::EditorDefaults::default(),
        values,
    )
}
