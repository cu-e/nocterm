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
    workspace_with_keys(cx, std::sync::Arc::new(nocterm_ssh_keys::NoVaultSshKeys))
}

/// A workspace whose connection editor offers the keys of `keys`.
pub(crate) fn workspace_with_keys(
    cx: &mut TestAppContext,
    keys: std::sync::Arc<dyn nocterm_ssh_keys::VaultSshKeys>,
) -> (AnyWindowHandle, Entity<Workspace>, Opened, Installed) {
    workspace_over(cx, memory(keys))
}

/// A workspace of a hub window, whose editor offers Agent access through `access`.
pub(crate) fn workspace_with_agent_access(
    cx: &mut TestAppContext,
    access: std::sync::Arc<dyn crate::ServerAgentAccess>,
) -> (AnyWindowHandle, Entity<Workspace>, Opened, Installed) {
    let prepared = memory(std::sync::Arc::new(nocterm_ssh_keys::NoVaultSshKeys));
    let inputs = prepared.inputs().with_agent_access(access);
    workspace_over(cx, crate::PreparedConnections::volatile(inputs))
}

fn memory(keys: std::sync::Arc<dyn nocterm_ssh_keys::VaultSshKeys>) -> crate::PreparedConnections {
    crate::PreparedConnections::memory_with_keys(
        nocterm_connections_model::draft::EditorDefaults {
            user: Some("tester".into()),
            home: None,
        },
        keys,
    )
}

fn workspace_over(
    cx: &mut TestAppContext,
    prepared: crate::PreparedConnections,
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
            prepared,
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

/// A space of the replica: `scope` named `name`, which the account `writes` to or only reads.
pub(crate) fn space(
    scope: nocterm_connections_model::replica::ReplicaScope,
    name: &str,
    writes: bool,
) -> nocterm_connections_model::replica::ReplicaSpace {
    use nocterm_connections_model::replica::ReplicaAccess;
    nocterm_connections_model::replica::ReplicaSpace {
        scope,
        name: name.to_owned(),
        access: if writes {
            ReplicaAccess::Write
        } else {
            ReplicaAccess::Read
        },
    }
}

/// An organization's scope.
pub(crate) fn organization(id: &str) -> nocterm_connections_model::replica::ReplicaScope {
    nocterm_connections_model::replica::ReplicaScope::Organization(id.to_owned())
}

/// Binds a replication to `installed` that no replica is attached to: the state of an
/// account that is signed out.
pub(crate) fn unattached(
    installed: &Installed,
    cx: &mut TestAppContext,
) -> Entity<nocterm_connections_store::ProfileReplication> {
    use nocterm_connections_model::replica::ReplicaState;
    use nocterm_connections_store::ProfileReplication;
    let replication = cx.update(|cx| {
        let store = installed.connections().clone();
        let writer: nocterm_store::SnapshotWriter<ReplicaState> =
            std::sync::Arc::new(|_| Box::pin(async { Ok(()) }));
        let document =
            cx.new(|_| ProfileReplication::state_document(ReplicaState::default(), writer));
        cx.new(|cx| ProfileReplication::new(store, document, cx))
    });
    installed.bind_replication(replication.clone());
    replication
}

/// Keeps `installed` in step with `replica`, as the application does for the chosen
/// account, and runs the first synchronization.
pub(crate) fn replicate(
    installed: &Installed,
    replica: std::sync::Arc<nocterm_connections_model::test_support::MemoryReplica>,
    cx: &mut TestAppContext,
) -> Entity<nocterm_connections_store::ProfileReplication> {
    let replication = unattached(installed, cx);
    replication.update(cx, |replication, cx| replication.attach(replica, cx));
    cx.run_until_parked();
    replication
}
