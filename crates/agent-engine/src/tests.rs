//! Real local HTTP fixtures exercise the actual pinned Rig provider wires.
mod fixtures;
mod history;
mod model_admission;
mod provider_access;
mod provider_catalog;
mod provider_streams;
mod recovery;
mod transport;

use crate::{ProviderBinding, ProviderEndpoint, ProviderKind, ProviderLease, RigEngineFactory};
use futures::{
    FutureExt,
    future::{AbortHandle, BoxFuture},
};
use nocterm_cloud_model::{
    authority::RunBinding,
    engine::{
        AgentEngine, AgentEngineFactory, EngineConfig, EngineError, EngineEvent, EngineEventSink,
        EngineInput, EnginePrompt, EngineRequest, ModelBudget,
    },
    identity::{
        AccountIdentity, AccountScope, CloudChatId, Fingerprint, HubIdentity, NamespaceId,
        RunEpoch, RunId, RunnerId, RunnerKeyEpoch, StepId,
    },
};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

const KEY: &str = "fixture-api-key-never-checkpoint";
use nocterm_cloud_model::testing::{ScriptedEngineEvents as Events, ScriptedModelAdmission};
fn admission() -> Arc<ScriptedModelAdmission> {
    Arc::new(ScriptedModelAdmission::new(config().run))
}
fn scope() -> AccountScope {
    AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("hub").unwrap(),
            Fingerprint::from_bytes([1; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new("account").unwrap(),
            Fingerprint::from_bytes([2; 32]),
        ),
    )
}
pub(crate) fn config() -> EngineConfig {
    EngineConfig {
        run: RunBinding::new(
            scope(),
            RunnerId::from_bytes([3; 16]),
            RunnerKeyEpoch::new(1).unwrap(),
            CloudChatId::from_bytes([4; 16]),
            RunId::from_bytes([5; 16]),
            RunEpoch::new(1).unwrap(),
        ),
        model: NamespaceId::new("fixture-model").unwrap(),
        history: Default::default(),
        budget: ModelBudget::new(4, 1024 * 1024, 256 * 1024, 1024).unwrap(),
    }
}
fn binding(kind: ProviderKind, root: &str) -> ProviderBinding {
    ProviderBinding::new(
        scope(),
        kind,
        ProviderEndpoint::for_fixture(root),
        vec![NamespaceId::new("fixture-model").unwrap()],
    )
    .unwrap()
}
fn factory(kind: ProviderKind, root: &str) -> RigEngineFactory {
    let lease = ProviderLease::new(
        binding(kind, root),
        Zeroizing::new(KEY.into()),
        std::time::Duration::from_secs(300),
    )
    .unwrap();
    RigEngineFactory::new(lease).unwrap()
}

fn engine(kind: ProviderKind, root: &str) -> Box<dyn AgentEngine> {
    factory(kind, root).create(config(), None).unwrap()
}
fn input(step: u8) -> EngineInput {
    EngineInput {
        next_step: StepId::from_bytes([step; 16]),
        request: EngineRequest::Prompt(EnginePrompt::new(vec!["hello".into()]).unwrap()),
    }
}
async fn advance(
    engine: &mut dyn AgentEngine,
) -> Result<nocterm_cloud_model::engine::EngineAction, EngineError> {
    let (_handle, registration) = AbortHandle::new_pair();
    engine
        .advance(
            input(1),
            Arc::new(Events::default()),
            admission(),
            registration,
        )
        .await
}
