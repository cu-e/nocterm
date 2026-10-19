use super::*;
/// Plain supplied documents/writer/capabilities, before GPUI installation.
pub struct FactsPrepared {
    known: Known,
    writer: SnapshotWriter<Known>,
    observations: ObservationDeps,
}
impl FactsPrepared {
    pub fn new(known: Known, writer: SnapshotWriter<Known>, observations: ObservationDeps) -> Self {
        Self {
            known,
            writer,
            observations,
        }
    }
    pub fn in_memory(observations: ObservationDeps) -> Self {
        Self::new(
            Known::default(),
            std::sync::Arc::new(|_| Box::pin(async { Ok(()) })),
            observations,
        )
    }
    pub fn install(
        self,
        carrier: Entity<crate::ProfilePublications>,
        policy: std::rc::Rc<dyn CountryPolicySource>,
        executor: gpui::BackgroundExecutor,
        shutdown: &ShutdownCoordinator,
        cx: &mut App,
    ) -> Result<(Entity<FactsService>, Vec<gpui::Subscription>), StoreError> {
        let countries: Vec<_> = self
            .known
            .countries()
            .filter_map(|code| CountryCode::new(code).ok())
            .collect();
        let initial_ids: HashSet<_> = carrier
            .read(cx)
            .current()
            .iter()
            .map(|(id, _)| id)
            .collect();
        let mut initial = self.known;
        initial.retain_profiles(&initial_ids);
        let locations = location::LocationAdmission::new(&initial);
        // Import establishes inert authority without eagerly rewriting untouched Known bytes.
        let known = cx.new(|_| Persisted::new(initial, self.writer, Publication::Optimistic, 1));
        shutdown.register(&known)?;
        let authority = authority::ProfileAuthority::new(carrier.clone(), cx);
        let policy = policy::DetectionPolicy::new(policy, cx);
        let storage = known.clone();
        let entity = cx.new(|_| FactsService {
            known,
            observations: self.observations,
            authority,
            probes: admission::ProbeLedger::new(),
            locations,
            flags: flags::RawFlags::new(),
            jobs: jobs::ObservationJobs::new(executor),
            policy,
        });
        entity.update(cx, |facts, cx| {
            facts.synchronize(cx);
            for code in countries {
                let _ = facts.request_flag(code, cx);
            }
            facts.progress(cx);
        });
        let observed = entity.downgrade();
        let storage_subscription = cx.observe(&storage, move |storage, cx| {
            if let Some(error) = storage.read(cx).error() {
                tracing::warn!(%error,"could not save detected server details");
            }
            let _ = observed.update(cx, |_, cx| cx.notify());
        });
        let observed = entity.downgrade();
        let carrier_subscription = cx.observe(&carrier, move |_, cx| {
            let _ = observed.update(cx, |facts, cx| {
                facts.synchronize(cx);
                facts.progress(cx);
            });
        });
        Ok((entity, vec![storage_subscription, carrier_subscription]))
    }
}
