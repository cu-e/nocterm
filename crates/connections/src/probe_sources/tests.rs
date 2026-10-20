//! Actual shared session Items drive original-source and window retry behavior.
use super::*;
use crate::{FactsPrepared, ObservationDeps};
use gpui_kit::{AnyWindowHandle, AppContext as _, TestAppContext, WindowOptions};
use nocterm_connections_model::{
    facts::Known,
    os::OsId,
    profiles::{Profile, Profiles},
    test_support::{ScriptedObservations, ScriptedOsDetector},
};
use nocterm_session::testing::{Gate, InMemoryFs};
use nocterm_workspace::{SessionSpec, test_support::SessionProbe};
use std::time::{Duration, Instant};
struct Scene {
    installed: crate::Installed,
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    profile: Profile,
    os: Arc<ScriptedOsDetector>,
    script: Arc<ScriptedObservations>,
}
impl Scene {
    fn new(network: bool, cx: &mut TestAppContext) -> Self {
        Self::with_known(
            network,
            Known::default(),
            Arc::new(ScriptedObservations::default()),
            cx,
        )
    }
    fn with_known(
        network: bool,
        known: Known,
        script: Arc<ScriptedObservations>,
        cx: &mut TestAppContext,
    ) -> Self {
        let (window, workspace, _, original) = crate::test_support::workspace(cx);
        let profile = Profile {
            id: nocterm_connections_model::test_support::profile_id_for("source"),
            name: "source".into(),
            description: String::new(),
            options: Default::default(),
            target: Target::new("ci", "example.com", 22),
            auth: Default::default(),
            group: None,
            credential: None,
            launch: None,
            icon: None,
            icon_color: None,
            country: None,
            jump_hosts: Vec::new(),
        };
        let mut profiles = Profiles::default();
        profiles.upsert(profile.clone());
        let os = Arc::new(ScriptedOsDetector::default());
        cx.executor().allow_parking();
        let executor =
            gpui_kit::BackgroundExecutor::new(Arc::new(gpui_kit::ThreadedDispatcher::new()));
        let installed = cx.update(|cx| {
            crate::test_support::install_owners_with_executor(
                Connections::with_documents(profiles, Default::default()),
                FactsPrepared::new(
                    known,
                    Arc::new(|_| Box::pin(async { Ok(()) })),
                    ObservationDeps::new(
                        script.clone(),
                        script.clone(),
                        script.clone(),
                        network,
                        os.clone(),
                    ),
                ),
                original.settings().clone(),
                executor,
                cx,
            )
        });
        installed.sources().register(workspace.downgrade());
        Self {
            installed,
            window,
            workspace,
            profile,
            os,
            script,
        }
    }
    fn second_window(&self, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
        let (window, workspace) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Workspace::new(
                        nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx)),
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        self.installed.sources().register(workspace.downgrade());
        (window, workspace)
    }
    fn spec(&self) -> SessionSpec {
        crate::test_support::direct_spec(&self.profile)
    }
    fn add(
        &self,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
        fs: Arc<dyn RemoteFs>,
        cx: &mut TestAppContext,
    ) -> Entity<SessionProbe> {
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let item = SessionProbe::with_fs(self.spec(), true, Some(fs), cx);
                workspace.add_item(item.clone(), window, cx);
                item
            })
        })
        .unwrap()
    }
    fn scan(&self, cx: &mut TestAppContext) {
        cx.update(|cx| {
            self.installed
                .sources()
                .scan(self.installed.connections(), self.installed.facts(), cx)
        });
    }
    fn os(&self, cx: &TestAppContext) -> Option<String> {
        self.installed.facts().read_with(cx, |facts, cx| {
            facts.os(self.profile.id, cx).map(str::to_owned)
        })
    }
    fn slots(&self, cx: &TestAppContext) -> usize {
        self.installed
            .facts()
            .read_with(cx, |facts, _| facts.available_jobs())
    }
}
fn until(cx: &mut TestAppContext, mut condition: impl FnMut(&TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if condition(cx) {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
}
mod source_authority;
mod window_capacity;

mod presentation;
