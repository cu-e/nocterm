//! Behavior fixtures drive actual carrier, published policy and shared native ports.
use crate::{
    ConnectionStore, CountryPolicySnapshot, CountryPolicySource, FactsPrepared, FactsService,
    ObservationDeps, ProbeOutcome, ProbeRequest,
};
use gpui::{App, BackgroundExecutor, Entity, TestAppContext, ThreadedDispatcher};
use nocterm_connections_model::{
    facts::Known,
    observation::{CountryCode, PublicAddress},
    profiles::{Profile, Profiles},
    test_support::{ScriptedObservations, ScriptedOsDetector},
};
use nocterm_session::{RemoteFs, Target, testing::InMemoryFs};
use nocterm_store::{
    SettingsConfig, SettingsHandle, ShutdownCoordinator, SnapshotWriter, StoreError,
};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};
mod policy_settings {
    #[derive(
        Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
    )]
    pub(super) struct PolicySettings {
        pub(super) enabled: bool,
        pub(super) unrelated: u32,
    }
    impl Default for PolicySettings {
        fn default() -> Self {
            Self {
                enabled: true,
                unrelated: 0,
            }
        }
    }
    impl nocterm_settings::SettingsSection for PolicySettings {
        const KEY: &'static str = "test_country_policy";
    }
}
use policy_settings::PolicySettings;
struct SettingsPolicy {
    settings: SettingsHandle,
}
impl CountryPolicySource for SettingsPolicy {
    fn current(&self, cx: &App) -> CountryPolicySnapshot {
        CountryPolicySnapshot::new(
            self.settings.setting::<PolicySettings>(cx).enabled,
            self.settings.revision(cx),
        )
    }
}
fn profile() -> Profile {
    Profile {
        id: nocterm_connections_model::profiles::ProfileId::from_bytes([1; 16]),
        name: "host".into(),
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
    }
}
fn ready_writer() -> SnapshotWriter<Known> {
    Arc::new(|_| Box::pin(async { Ok(()) }))
}
struct Fixture {
    profile: Profile,
    leases: std::cell::RefCell<Vec<(std::sync::Weak<dyn RemoteFs>, crate::ProbeSourceLease)>>,
    connections: Entity<ConnectionStore>,
    facts: Entity<FactsService>,
    settings: SettingsHandle,
    script: Arc<ScriptedObservations>,
    os: Arc<ScriptedOsDetector>,
    shutdown: ShutdownCoordinator,
}
impl Fixture {
    fn new(
        profile: Profile,
        known: Known,
        network: bool,
        executor: BackgroundExecutor,
        writer: SnapshotWriter<Known>,
        cx: &mut TestAppContext,
    ) -> Self {
        let script = Arc::new(ScriptedObservations::default());
        let os = Arc::new(ScriptedOsDetector::default());
        let mut profiles = Profiles::default();
        profiles.upsert(profile.clone());
        let shutdown = ShutdownCoordinator::default();
        let connections = cx.update(|cx| {
            ConnectionStore::with_documents(profiles, Default::default())
                .into_entity(&shutdown, cx)
                .unwrap()
        });
        let carrier = connections.update(cx, |connections, cx| connections.publications(cx));
        let settings = cx.update(|cx| {
            SettingsConfig::in_memory(Default::default())
                .with(PolicySettings::default())
                .install(cx)
        });
        let prepared = FactsPrepared::new(
            known,
            writer,
            ObservationDeps::new(
                script.clone(),
                script.clone(),
                script.clone(),
                network,
                os.clone(),
            ),
        );
        let (facts, subscriptions) = cx.update(|cx| {
            prepared
                .install(
                    carrier,
                    Rc::new(SettingsPolicy {
                        settings: settings.clone(),
                    }),
                    executor,
                    &shutdown,
                    cx,
                )
                .unwrap()
        });
        // Admission/completion correctness must work WITHOUT carrier/settings observer delivery.
        drop(subscriptions);
        Self {
            profile,
            leases: Default::default(),
            connections,
            facts,
            settings,
            script,
            os,
            shutdown,
        }
    }
    fn memory(network: bool, cx: &mut TestAppContext) -> Self {
        Self::new(
            profile(),
            Default::default(),
            network,
            cx.background_executor.clone(),
            ready_writer(),
            cx,
        )
    }
    fn source(&self, fs: &Arc<dyn RemoteFs>) -> crate::ProbeSourceLease {
        let mut leases = self.leases.borrow_mut();
        leases.retain(|(fs, _)| fs.strong_count() > 0);
        if let Some((_, source)) = leases
            .iter()
            .find(|(original, _)| std::sync::Weak::ptr_eq(original, &Arc::downgrade(fs)))
        {
            return source.clone();
        }
        let source = lease(fs, gpui::EntityId::from((leases.len() + 1) as u64), 0);
        leases.push((Arc::downgrade(fs), source.clone()));
        source
    }
    fn request(
        &self,
        fs: Arc<dyn RemoteFs>,
        cx: &mut TestAppContext,
    ) -> Result<ProbeOutcome, crate::FactsAdmissionError> {
        self.facts.update(cx, |facts, cx| {
            facts.request_probe(
                ProbeRequest::new(
                    self.profile.id,
                    self.profile.target.clone(),
                    fs.clone(),
                    self.source(&fs),
                ),
                cx,
            )
        })
    }
    fn publish_policy(
        &self,
        edit: impl FnOnce(&mut PolicySettings) + 'static,
        cx: &mut TestAppContext,
    ) {
        let receipt = cx.update(|cx| self.settings.update::<PolicySettings>(cx, edit));
        cx.run_until_parked();
        futures::executor::block_on(receipt).unwrap();
    }
    fn save(&self, profile: Profile, cx: &mut TestAppContext) {
        let receipt = self
            .connections
            .update(cx, |connections, cx| connections.save_profile(profile, cx));
        cx.run_until_parked();
        futures::executor::block_on(receipt).unwrap();
    }
    fn country_answer(&self) {
        self.script
            .resolve_next(Ok(PublicAddress::new("8.8.8.8".parse().unwrap())));
        self.script
            .country_next(Ok(Some(CountryCode::new("de").unwrap())));
    }
    fn raw_country(&self, cx: &TestAppContext) -> Option<String> {
        self.facts.read_with(cx, |facts, cx| {
            facts.country(self.profile.id, cx).map(str::to_owned)
        })
    }
    fn raw_os(&self, cx: &TestAppContext) -> Option<String> {
        self.facts.read_with(cx, |facts, cx| {
            facts.os(self.profile.id, cx).map(str::to_owned)
        })
    }
    fn jobs(&self, cx: &TestAppContext) -> usize {
        self.facts.read_with(cx, |facts, _| facts.available_jobs())
    }
}
struct OriginalFs {
    fs: std::sync::Weak<dyn RemoteFs>,
}
impl crate::ProbeSourceAuthority for OriginalFs {
    fn same_session_alive(&self, _: &App) -> bool {
        self.fs.upgrade().is_some()
    }
    fn is_current(&self, _: &App) -> bool {
        self.fs.upgrade().is_some()
    }
}
fn lease(fs: &Arc<dyn RemoteFs>, item: gpui::EntityId, generation: u64) -> crate::ProbeSourceLease {
    crate::ProbeSourceLease::new(
        crate::ProbeSourceIdentity::new(item, generation),
        Rc::new(OriginalFs {
            fs: Arc::downgrade(fs),
        }),
    )
}
fn fs() -> Arc<dyn RemoteFs> {
    Arc::new(InMemoryFs::default())
}
fn threaded(cx: &TestAppContext) -> BackgroundExecutor {
    // External native workers use the supported GPUI real-activity mode; gate assertions stay bounded.
    cx.executor().allow_parking();
    BackgroundExecutor::new(Arc::new(ThreadedDispatcher::new()))
}
/// Bounded condition pumping for real worker signals; no timing-based result assumption.
fn until(cx: &mut TestAppContext, mut ready: impl FnMut(&TestAppContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "public native/foreground condition did not complete"
        );
        std::thread::yield_now();
    }
}
mod admission;
mod authority;
mod flags;
mod location;
mod persistence;
