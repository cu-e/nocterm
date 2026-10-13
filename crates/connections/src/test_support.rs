use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use nocterm_ui::DesignTokens;
use nocterm_workspace::{SessionSpec, Workspace};

use crate::{Connections, Installed, ObservationDeps, ServerFacts};

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
            nocterm_ui::SettingsStore::in_memory(Default::default()),
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
                    let deps =
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
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

/// The shared pure repository supplies failures/documents while the existing driver owns writes.
pub(crate) fn prepared_repository(
    repository: std::sync::Arc<
        nocterm_connections_model::test_support::InMemoryConnectionRepository,
    >,
    observations: crate::ObservationDeps,
) -> crate::PreparedConnections {
    use nocterm_connections_model::{
        draft::EditorDefaults, repository::ConnectionRepository, value_ports::SuppliedValues,
    };
    let documents = repository.load();
    let profiles = repository.clone();
    let recents = repository.clone();
    let facts = repository;
    let writers = crate::ConnectionWriters::new(
        std::sync::Arc::new(move |snapshot| {
            let repository = profiles.clone();
            Box::pin(async move {
                repository
                    .save_profiles(&snapshot)
                    .map_err(nocterm_store::StoreError::backend)
            })
        }),
        std::sync::Arc::new(move |snapshot| {
            let repository = recents.clone();
            Box::pin(async move {
                repository
                    .save_recents(&snapshot)
                    .map_err(nocterm_store::StoreError::backend)
            })
        }),
        std::sync::Arc::new(move |snapshot| {
            let repository = facts.clone();
            Box::pin(async move {
                repository
                    .save_facts(&snapshot)
                    .map_err(nocterm_store::StoreError::backend)
            })
        }),
    );
    let values = std::sync::Arc::new(SuppliedValues::excluding(
        1,
        0,
        documents.profiles.iter().map(|profile| profile.id),
    ));
    crate::PreparedConnections::new(
        documents,
        writers,
        crate::EditorInputs::new(
            EditorDefaults {
                user: Some("tester".into()),
                home: None,
            },
            values.clone(),
        ),
        values,
        observations,
    )
}

/// Explicit alternate owners for tests; preserves the supplied settings installation.
pub(crate) fn install_owners(
    connections: Connections,
    facts: ServerFacts,
    settings: nocterm_store::SettingsHandle,
    cx: &mut gpui_kit::App,
) -> Installed {
    let inputs = model_inputs(&connections, cx);
    Installed::from_owners(
        connections,
        facts,
        inputs,
        settings,
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
        ServerFacts::in_memory(ObservationDeps::offline()),
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
