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

/// A workspace whose connection editor keeps a typed password in `passwords`.
pub(crate) fn workspace_with_passwords(
    cx: &mut TestAppContext,
    passwords: std::sync::Arc<dyn crate::SavedPasswords>,
) -> (AnyWindowHandle, Entity<Workspace>, Opened, Installed) {
    let prepared = memory(std::sync::Arc::new(nocterm_ssh_keys::NoVaultSshKeys));
    let inputs = prepared.inputs().with_passwords(passwords);
    workspace_over(cx, crate::PreparedConnections::volatile(inputs))
}

/// A vault of `place` that records every password kept in it, or refuses with `locked`.
pub(crate) struct ScriptedPasswords {
    place: crate::VaultPlace,
    pub(crate) locked: std::sync::atomic::AtomicBool,
    /// The passwords the vault offers.
    pub(crate) saved: std::sync::Mutex<Vec<crate::SavedPassword>>,
    /// `(record, address)` of each saved password attached to an address, in order.
    pub(crate) attached:
        std::sync::Mutex<Vec<(nocterm_session::CredentialId, nocterm_session::Target)>>,
    /// `(replacing, label, target, password)` of each password kept, in order.
    pub(crate) kept: std::sync::Mutex<
        Vec<(
            Option<nocterm_session::CredentialId>,
            String,
            nocterm_session::Target,
            String,
        )>,
    >,
}

impl ScriptedPasswords {
    pub(crate) fn new(place: crate::VaultPlace) -> Self {
        Self {
            place,
            locked: Default::default(),
            saved: Default::default(),
            attached: Default::default(),
            kept: Default::default(),
        }
    }
}

impl crate::SavedPasswords for ScriptedPasswords {
    fn place(&self) -> crate::VaultPlace {
        self.place
    }

    fn list(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        Result<Vec<crate::SavedPassword>, crate::PasswordNotKept>,
    > {
        Box::pin(std::future::ready(Ok(self.saved.lock().unwrap().clone())))
    }

    fn attach(
        &self,
        id: nocterm_session::CredentialId,
        target: nocterm_session::Target,
    ) -> futures::future::BoxFuture<'static, Result<(), crate::PasswordNotKept>> {
        if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(std::future::ready(Err(crate::PasswordNotKept::Locked)));
        }
        self.attached.lock().unwrap().push((id, target));
        Box::pin(std::future::ready(Ok(())))
    }

    fn keep(
        &self,
        replacing: Option<nocterm_session::CredentialId>,
        label: String,
        target: nocterm_session::Target,
        password: nocterm_session::Secret,
    ) -> futures::future::BoxFuture<
        'static,
        Result<nocterm_session::CredentialId, crate::PasswordNotKept>,
    > {
        if self.locked.load(std::sync::atomic::Ordering::SeqCst) {
            return Box::pin(std::future::ready(Err(crate::PasswordNotKept::Locked)));
        }
        let mut kept = self.kept.lock().unwrap();
        let id = replacing.unwrap_or_else(|| {
            nocterm_session::CredentialId::from_bytes([u8::try_from(kept.len() + 1).unwrap(); 16])
        });
        kept.push((replacing, label, target, password.expose().to_owned()));
        Box::pin(std::future::ready(Ok(id)))
    }
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

/// An Agent access port that records each change and grants it, or refuses it with `refusal`.
/// `session` tells whether a session to the server is open; `state` is what the hub records.
#[derive(Default)]
pub(crate) struct ScriptedAgentAccess {
    pub(crate) state: std::sync::Mutex<Option<crate::AgentAccessState>>,
    pub(crate) session: std::sync::atomic::AtomicBool,
    pub(crate) changes: std::sync::Mutex<Vec<(crate::AgentServer, bool)>>,
    pub(crate) refusal: Option<&'static str>,
}

impl ScriptedAgentAccess {
    pub(crate) fn with_session() -> Self {
        let access = Self::default();
        access.open_session();
        access
    }

    pub(crate) fn open_session(&self) {
        self.session
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub(crate) fn granted(&self) -> Vec<bool> {
        let changes = self.changes.lock().unwrap();
        changes.iter().map(|(_, granted)| *granted).collect()
    }
}

impl crate::ServerAgentAccess for ScriptedAgentAccess {
    fn state(&self, _: &crate::AgentServer, _: &gpui_kit::App) -> Option<crate::AgentAccessState> {
        *self.state.lock().unwrap()
    }

    fn session_open(&self, _: &crate::AgentServer, _: &gpui_kit::App) -> bool {
        self.session.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn set(
        &self,
        server: crate::AgentServer,
        granted: bool,
        _: &mut gpui_kit::App,
    ) -> gpui_kit::Task<Result<(), crate::AgentAccessRefused>> {
        self.changes.lock().unwrap().push((server, granted));
        match self.refusal {
            Some(reason) => gpui_kit::Task::ready(Err(crate::AgentAccessRefused::new(reason))),
            None => {
                *self.state.lock().unwrap() = granted.then_some(crate::AgentAccessState::Unchecked);
                gpui_kit::Task::ready(Ok(()))
            }
        }
    }

    fn observe(
        &self,
        _: Box<dyn Fn(&mut gpui_kit::App)>,
        _: &mut gpui_kit::App,
    ) -> gpui_kit::Subscription {
        gpui_kit::Subscription::new(|| {})
    }
}
