//! Real Rig factories consume bounded scripted native credentials; no crypto claims.
use super::*;
use crate::{ConfiguredProviderCatalog, ProviderPolicy};
use nocterm_cloud_model::{
    authority::{CommandIssuer, StreamBinding},
    coordinator::{
        CommandAcknowledgement, CommandReceiveBudget, CoordinatorProviders, GrantClass,
        GrantReference, IntakeLane, IntakeLimits, ProviderOffer,
    },
    identity::{ChatControlRevision, CommandId, GrantEpoch, GrantId, Timestamp},
    inference::{ConservativeModelPricing, ModelSelection},
    journal::{CommandAcceptance, CommandKind, JournalCatalogBinding},
    testing::{ManualToolTimer, ScriptedProviderCredentials},
};

struct CatalogFixture {
    source: Arc<ScriptedProviderCredentials>,
    catalog: ConfiguredProviderCatalog,
    timer: Arc<ManualToolTimer>,
    budget: CommandReceiveBudget,
    selection: ModelSelection,
}
impl CatalogFixture {
    fn new(root: &str) -> Self {
        let run = config().run;
        let binding =
            JournalCatalogBinding::new(run.scope().clone(), run.runner(), run.runner_epoch());
        let source = Arc::new(ScriptedProviderCredentials::new(binding));
        let timer = Arc::new(ManualToolTimer::default());
        timer.set(100_000);
        let selection = ModelSelection::new(
            NamespaceId::new("openai").unwrap(),
            config().model,
            NamespaceId::new("fixed-price").unwrap(),
        );
        let pricing = Arc::new(
            ConservativeModelPricing::new(selection.clone(), 1024 * 1024, 1024, 1).unwrap(),
        );
        let configured = super::binding(ProviderKind::OpenAiChat, root);
        let configured = ProviderBinding::new(
            configured.scope().clone(),
            configured.kind(),
            configured.endpoint().clone(),
            vec![
                selection.model().clone(),
                NamespaceId::new("not-granted").unwrap(),
            ],
        )
        .unwrap();
        let policy =
            ProviderPolicy::new(selection.provider().clone(), configured, vec![pricing]).unwrap();
        let catalog =
            ConfiguredProviderCatalog::new(source.clone(), timer.clone(), vec![policy], 16)
                .unwrap();
        Self {
            source,
            catalog,
            timer,
            budget: CommandReceiveBudget::new(IntakeLimits::default()),
            selection,
        }
    }
    fn grant(&self, id: u8, epoch: u64, command: u8) -> (CommandAcknowledgement, ProviderOffer) {
        let run = config().run;
        let stream = StreamBinding::new(
            run.scope().clone(),
            run.runner(),
            run.runner_epoch(),
            run.chat(),
        );
        let command = CommandAcknowledgement::new(
            stream,
            CommandAcceptance::new(
                CommandId::from_bytes([command; 16]),
                CommandKind::IssueGrant,
                Fingerprint::from_bytes([command; 32]),
                CommandIssuer::new(
                    NamespaceId::new("device").unwrap(),
                    Fingerprint::from_bytes([9; 32]),
                    Fingerprint::from_bytes([10; 32]),
                ),
                ChatControlRevision::from_u64(u64::from(command)),
            ),
        );
        let offer = ProviderOffer::new(
            GrantReference::new(
                GrantClass::Provider,
                GrantId::from_bytes([id; 16]),
                GrantEpoch::new(epoch).unwrap(),
            ),
            self.selection.provider().clone(),
            vec![self.selection.model().clone()],
            Fingerprint::from_bytes([11; 32]),
            Fingerprint::from_bytes([12; 32]),
            Timestamp::from_millis(110_000),
        )
        .unwrap();
        (command, offer)
    }
    fn stage(&self, command: &CommandAcknowledgement, offer: &ProviderOffer) {
        let allocation = self
            .budget
            .try_reserve(IntakeLane::Ordinary, 64 * 1024, 1)
            .unwrap();
        self.source
            .stage(
                command.clone(),
                offer.clone(),
                KEY.as_bytes().to_vec(),
                allocation,
            )
            .unwrap();
    }
    async fn install(
        &self,
        id: u8,
        epoch: u64,
        command: u8,
    ) -> (CommandAcknowledgement, ProviderOffer) {
        let (command, offer) = self.grant(id, epoch, command);
        self.stage(&command, &offer);
        self.catalog
            .install(command.clone(), offer.clone())
            .await
            .unwrap();
        (command, offer)
    }
}

async fn issued_factory_keeps_the_actual_model_and_runner_grant(root: &str) {
    use nocterm_cloud_model::identity::{RunnerId, RunnerKeyEpoch};
    let f = CatalogFixture::new(root);
    f.install(1, 1, 1).await;
    let issued = f.catalog.model(&f.selection).unwrap();
    let mut excluded = config();
    excluded.model = NamespaceId::new("not-granted").unwrap();
    assert!(matches!(
        issued.factory().create(excluded, None),
        Err(EngineError::Unavailable)
    ));
    for (runner, epoch) in [
        (RunnerId::from_bytes([99; 16]), config().run.runner_epoch()),
        (config().run.runner(), RunnerKeyEpoch::new(99).unwrap()),
    ] {
        let mut foreign = config();
        let original = &foreign.run;
        foreign.run = RunBinding::new(
            original.scope().clone(),
            runner,
            epoch,
            original.chat(),
            original.run(),
            original.epoch(),
        );
        assert!(matches!(
            issued.factory().create(foreign, None),
            Err(EngineError::Unavailable)
        ));
    }
    assert!(issued.factory().create(config(), None).is_ok());
}

async fn install_raced_by_revocation(
    f: &CatalogFixture,
    pending: CommandAcknowledgement,
    third: ProviderOffer,
) {
    let (entered, release) = f.source.pause_next_delivery();
    let catalog = f.catalog.clone();
    let offer = third.clone();
    let runtime = tokio::runtime::Handle::current();
    let worker = std::thread::spawn(move || {
        let _runtime = runtime.enter();
        catalog
            .install(pending, offer)
            .now_or_never()
            .expect("synchronous capsule delivery")
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), entered.wait())
        .await
        .unwrap();
    f.catalog.revoke(third.reference());
    let _fourth = f.install(2, 1, 4).await;
    release.release();
    assert!(worker.join().unwrap().is_err());
}

async fn replacement_and_pending_revocation(root: &str) {
    let f = CatalogFixture::new(root);
    let (_, first) = f.install(1, 1, 1).await;
    let old = f.catalog.model(&f.selection).unwrap();
    let mut old_engine = old.factory().create(config(), None).unwrap();
    let (second_command, second) = f.install(1, 2, 2).await;
    assert!(matches!(
        old.factory().create(config(), None),
        Err(EngineError::NeedsProviderAccess)
    ));
    assert!(matches!(
        advance(old_engine.as_mut()).await,
        Err(EngineError::NeedsProviderAccess)
    ));
    assert_eq!(
        f.budget.usage(IntakeLane::Ordinary).unwrap().bytes,
        128 * 1024
    );
    f.catalog.revoke(first.reference());
    assert!(f.catalog.model(&f.selection).is_ok());
    let (pending, third) = f.grant(1, 3, 3);
    f.stage(&pending, &third);
    install_raced_by_revocation(&f, pending, third).await;
    assert!(f.catalog.install(second_command, second).await.is_err());
    let current = f.catalog.model(&f.selection).unwrap();
    let mut engine = current.factory().create(config(), None).unwrap();
    assert!(advance(engine.as_mut()).await.is_ok());
    f.source.revoke();
    assert!(f.catalog.model(&f.selection).is_err());
    assert!(matches!(
        current.factory().create(config(), None),
        Err(EngineError::NeedsProviderAccess)
    ));
    drop(old_engine);
    drop(old);
    drop(engine);
    drop(current);
    drop(f.catalog);
    assert_eq!(f.budget.usage(IntakeLane::Ordinary).unwrap().bytes, 0);
}

async fn signed_expiry_and_sticky_clock_rollback(root: &str) {
    let f = CatalogFixture::new(root);
    f.install(1, 1, 1).await;
    let issued = f.catalog.model(&f.selection).unwrap();
    f.timer.set(110_000);
    assert!(matches!(
        issued.factory().create(config(), None),
        Err(EngineError::NeedsProviderAccess)
    ));
    assert!(f.catalog.model(&f.selection).is_err());
    let g = CatalogFixture::new(root);
    g.install(1, 1, 1).await;
    let original = g.catalog.model(&g.selection).unwrap();
    g.timer.set(99_999);
    assert!(g.catalog.model(&g.selection).is_err());
    g.timer.set(100_001);
    assert!(g.catalog.model(&g.selection).is_err());
    assert!(matches!(
        original.factory().create(config(), None),
        Err(EngineError::NeedsProviderAccess)
    ));
}

#[tokio::test]
async fn provider_catalog_preserves_original_generations_and_key_owners_through_actual_rig_use() {
    let server = fixtures::Server::start(vec![fixtures::http(&fixtures::chat_text(
        "actual catalog",
        false,
    ))])
    .await;
    replacement_and_pending_revocation(server.root()).await;
    signed_expiry_and_sticky_clock_rollback(server.root()).await;
    issued_factory_keeps_the_actual_model_and_runner_grant(server.root()).await;
    assert_eq!(server.requests().len(), 1);
}
