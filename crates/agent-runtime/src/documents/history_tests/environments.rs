//! Retained Local account sources fence documents, readers and shared budgets.
use super::*;
use nocterm_ai::{local_environment::LocalEnvironmentAccess, storage::AgentStorageError};
use nocterm_hubs_model::{
    source_identity::{
        AccountIdentity, AccountScope, Fingerprint, HubIdentity, LocalAccountPartition,
        LocalAccountSourceFactory, NamespaceId,
    },
    testing::ScriptedLocalAccountFactory,
};

fn partition(hub: u8, account: &str) -> LocalAccountPartition {
    LocalAccountPartition::Stable(AccountScope::new(
        HubIdentity::new(
            NamespaceId::new("same-instance-id").unwrap(),
            Fingerprint::from_bytes([hub; 32]),
        ),
        AccountIdentity::new(
            NamespaceId::new(account).unwrap(),
            Fingerprint::from_bytes([3; 32]),
        ),
    ))
}
fn bind(base: &RuntimeHandle, factory: &ScriptedLocalAccountFactory) -> RuntimeHandle {
    base.for_environment(LocalEnvironmentAccess::capture(factory.capture().unwrap()).unwrap())
        .unwrap()
}
fn chat(handle: &RuntimeHandle, title: &str) -> Arc<SharedChat> {
    let mut metadata = snapshot().metadata.clone();
    metadata.version = 3;
    metadata.header.environment = handle.environment().unwrap().clone();
    metadata.header.title = Some(title.into());
    Arc::new(SharedChat {
        metadata,
        queue: Vec::new(),
    })
}
#[gpui::test]
fn identical_ids_in_different_authenticated_hubs_have_independent_writers_and_catalogs(
    cx: &mut TestAppContext,
) {
    let (base, _directory, repository, _, _) = contexts::scoped_fixture(cx);
    let first = ScriptedLocalAccountFactory::default();
    let second = ScriptedLocalAccountFactory::default();
    first.sign_in(partition(1, "same-account-id"));
    second.sign_in(partition(2, "same-account-id"));
    let a = bind(&base, &first);
    let b = bind(&base, &second);
    let a_chat = chat(&a, "Hub A");
    let b_chat = chat(&b, "Hub B");
    let a_client = ProbeClient::new(cx, a_chat.clone());
    let b_client = ProbeClient::new(cx, b_chat.clone());
    cx.update(|cx| {
        ownership::register_bound(&a, a_client.clone(), cx).unwrap();
        ownership::register_bound(&b, b_client.clone(), cx).unwrap();
        a.save_chat(a_chat.clone(), a_client.id(), 1, cx).unwrap();
        b.save_chat(b_chat.clone(), b_client.id(), 1, cx).unwrap();
    });
    cx.run_until_parked();
    cx.update(|cx| {
        a.reload_history(cx);
        b.reload_history(cx);
    });
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(a.catalog_snapshot(cx).unwrap().rows()[0].title(), "Hub A");
        assert_eq!(b.catalog_snapshot(cx).unwrap().rows()[0].title(), "Hub B");
        a.delete_chat(a_chat.header.id.clone(), cx);
    });
    cx.run_until_parked();
    assert!(
        repository
            .encoded_in_environment(&a.chat_scope().unwrap(), &a_chat.header.id)
            .is_none()
    );
    assert!(
        repository
            .encoded_in_environment(&b.chat_scope().unwrap(), &b_chat.header.id)
            .is_some()
    );
    assert!(
        repository.encoded(&a_chat.header.id).is_none(),
        "hub Local never writes ThisComputer"
    );
}
#[gpui::test]
async fn deferred_reader_refuses_the_old_owner_after_a_b_a_and_does_not_retarget(
    cx: &mut TestAppContext,
) {
    let (base, _directory, repository, _, _) = contexts::scoped_fixture(cx);
    let factory = ScriptedLocalAccountFactory::default();
    factory.sign_in(partition(1, "alice"));
    let a = bind(&base, &factory);
    let original = chat(&a, "A original");
    repository.seed_chat(&original.metadata).unwrap();
    let reader = cx.update(|cx| a.history_reader(cx));
    let release = repository.block_next(StorageOperation::Load(original.header.id.clone()));
    let pending = cx
        .background_executor
        .spawn(async move { reader.prepare_load(original.header.id.clone()).await });
    cx.run_until_parked();
    factory.sign_in(partition(1, "bob"));
    let b = bind(&base, &factory);
    factory.sign_in(partition(1, "alice"));
    let new_a = bind(&base, &factory);
    release.send(Ok(())).unwrap();
    let (loaded, permit) = pending.await;
    assert!(matches!(loaded, Err(AgentStorageError::AccountSource(_))));
    drop(permit);
    assert!(a.check_current().is_err());
    assert!(b.check_current().is_err());
    assert!(new_a.check_current().is_ok());
    let attempts = repository.operations().len();
    let (loaded, permit) = cx
        .update(|cx| a.history_reader(cx))
        .prepare_load(snapshot().header.id.clone())
        .await;
    assert!(loaded.is_err());
    drop(permit);
    assert_eq!(
        repository.operations().len(),
        attempts,
        "stale reads fail before repository I/O"
    );
}
#[gpui::test]
fn stale_claims_and_native_startup_cannot_acquire_the_replacement_account(cx: &mut TestAppContext) {
    let (base, _directory, _, connector, _) = contexts::scoped_fixture(cx);
    let factory = ScriptedLocalAccountFactory::default();
    factory.sign_in(partition(1, "alice"));
    let old = bind(&base, &factory);
    let client = ProbeClient::new(cx, chat(&old, "original"));
    let claim = cx.update(|cx| {
        ownership::owned_claim(
            old.claim_document(snapshot().header.id.clone(), client.id(), cx)
                .unwrap(),
        )
    });
    factory.sign_in(partition(1, "bob"));
    cx.update(|cx| {
        assert!(old.commit_document(claim, client.clone(), cx).is_err());
        assert!(old.request_activation(client.clone(), cx).is_err());
        assert!(
            old.register_bridge(&(client.clone() as Client), cx)
                .is_err()
        );
    });
    cx.run_until_parked();
    assert!(connector.requests().is_empty());
    let current = bind(&base, &factory);
    cx.update(|cx| {
        let claim = ownership::owned_claim(
            current
                .claim_document(snapshot().header.id.clone(), client.id(), cx)
                .unwrap(),
        );
        assert_eq!(claim.key().environment(), current.environment().unwrap());
    });
}
#[gpui::test]
fn all_account_context_views_consume_one_installation_catalog_budget(cx: &mut TestAppContext) {
    let (base, _directory, repository, _, _) = contexts::scoped_fixture(cx);
    cx.run_until_parked();
    let mut retained = Vec::new();
    let mut factories = Vec::new();
    for index in 0..crate::history_admission::MAX_NORMAL_HISTORY_PAYLOADS as u8 {
        let factory = ScriptedLocalAccountFactory::default();
        factory.sign_in(partition(index, "alice"));
        let handle = bind(&base, &factory);
        cx.update(|cx| handle.try_reload_history(cx).unwrap());
        cx.run_until_parked();
        retained.push(cx.update(|cx| handle.catalog_snapshot(cx).unwrap()));
        factories.push(factory);
    }
    let attempts = repository.operations().len();
    cx.update(|cx| {
        assert!(matches!(
            base.try_reload_history(cx),
            Err(crate::CatalogLoadError::Capacity)
        ))
    });
    assert_eq!(repository.operations().len(), attempts);
    drop(retained.remove(0));
    cx.update(|cx| base.try_reload_history(cx).unwrap());
    cx.run_until_parked();
    assert!(cx.update(|cx| base.catalog_snapshot(cx)).is_some());
}

#[gpui::test]
fn resident_history_budget_is_shared_across_accounts_and_windows(cx: &mut TestAppContext) {
    const MIB: usize = 1024 * 1024;
    let (base, _directory, _, _, _) = contexts::scoped_fixture(cx);
    let a_factory = ScriptedLocalAccountFactory::default();
    let b_factory = ScriptedLocalAccountFactory::default();
    a_factory.sign_in(partition(1, "alice"));
    b_factory.sign_in(partition(2, "alice"));
    let a = bind(&base, &a_factory);
    let b = bind(&base, &b_factory);
    let first = ProbeClient::new(cx, chat(&a, "A"));
    let requester = ProbeClient::new(cx, chat(&b, "B"));
    first
        .history
        .set(Some(crate::HistoryUsage::new(48 * MIB, 1, true)));
    first.allow_history_eviction.set(true);
    requester
        .history
        .set(Some(crate::HistoryUsage::new(0, 2, false)));
    cx.update(|cx| {
        ownership::register_bound(&a, first.clone(), cx).unwrap();
        ownership::register_bound(&b, requester.clone(), cx).unwrap();
        assert!(matches!(
            b.admit_history(requester.id(), 24 * MIB, cx),
            Err(AgentStorageError::ResidentCapacity { .. })
        ));
        first
            .history
            .set(Some(crate::HistoryUsage::new(48 * MIB, 1, false)));
        b.admit_history(requester.id(), 24 * MIB, cx).unwrap();
        assert!(first.history.get().is_none());
    });
}
