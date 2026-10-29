use super::{fixtures::*, *};
use std::time::Duration;

#[tokio::test]
async fn revoked_provider_access_is_unavailable_to_existing_factories_and_engines() {
    let server = Server::start(vec![]).await;
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        Duration::from_secs(30),
    )
    .unwrap();
    let factory = RigEngineFactory::new(lease.clone()).unwrap();
    let mut engine = factory.create(config(), None).unwrap();
    lease.revoke();
    assert!(!lease.is_available());
    assert!(matches!(
        advance(engine.as_mut()).await,
        Err(EngineError::NeedsProviderAccess)
    ));
    assert!(server.requests().is_empty());
    assert!(!format!("{lease:?}").contains(KEY));
}
#[tokio::test]
async fn provider_expiry_interrupts_backpressured_progress_without_waiting_for_sink() {
    struct Blocked;
    impl EngineEventSink for Blocked {
        fn publish(
            &self,
            _: StepId,
            _: EngineEvent,
        ) -> BoxFuture<'static, Result<(), EngineError>> {
            futures::future::pending().boxed()
        }
    }
    let server = Server::start(vec![http(&chat_text("partial", true))]).await;
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        Duration::from_millis(100),
    )
    .unwrap();
    let factory = RigEngineFactory::new(lease.clone()).unwrap();
    let mut engine = factory.create(config(), None).unwrap();
    let (_, registration) = AbortHandle::new_pair();
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        engine.advance(input(1), Arc::new(Blocked), admission(), registration),
    )
    .await
    .unwrap();
    assert!(
        matches!(result, Err(EngineError::NeedsProviderAccess)),
        "{result:?}"
    );
    assert!(!lease.is_available());
    assert!(engine.checkpoint(Default::default()).is_err());
}
#[tokio::test]
async fn idle_provider_lease_expires_and_never_becomes_an_unbounded_secret_store() {
    let server = Server::start(vec![]).await;
    assert!(
        ProviderLease::new(
            binding(ProviderKind::OpenAiChat, server.root()),
            Zeroizing::new(KEY.into()),
            Duration::ZERO
        )
        .is_err()
    );
    assert!(
        ProviderLease::new(
            binding(ProviderKind::OpenAiChat, server.root()),
            Zeroizing::new(KEY.into()),
            crate::MAX_PROVIDER_LEASE + Duration::from_secs(1)
        )
        .is_err()
    );
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        Duration::from_millis(20),
    )
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!lease.is_available());
}

#[tokio::test]
async fn explicit_revoke_interrupts_an_already_blocked_progress_delivery() {
    struct Blocked(Mutex<Option<tokio::sync::oneshot::Sender<()>>>);
    impl EngineEventSink for Blocked {
        fn publish(
            &self,
            _: StepId,
            _: EngineEvent,
        ) -> BoxFuture<'static, Result<(), EngineError>> {
            if let Some(sender) = self.0.lock().unwrap().take() {
                let _ = sender.send(());
            }
            futures::future::pending().boxed()
        }
    }
    let server = Server::start(vec![http(&chat_text("partial", true))]).await;
    let lease = ProviderLease::new(
        binding(ProviderKind::OpenAiChat, server.root()),
        Zeroizing::new(KEY.into()),
        Duration::from_secs(30),
    )
    .unwrap();
    let factory = RigEngineFactory::new(lease.clone()).unwrap();
    let mut engine = factory.create(config(), None).unwrap();
    let (entered, wait) = tokio::sync::oneshot::channel();
    let (_, registration) = AbortHandle::new_pair();
    let work = engine.advance(
        input(1),
        Arc::new(Blocked(Mutex::new(Some(entered)))),
        admission(),
        registration,
    );
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("progress should block: {result:?}"), _=wait=>{} }
    lease.revoke();
    assert!(matches!(work.await, Err(EngineError::NeedsProviderAccess)));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn destroying_the_host_runtime_revokes_its_idle_memory_only_provider_lease() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let lease = runtime.block_on(async {
        ProviderLease::new(
            binding(ProviderKind::OpenAiChat, "http://127.0.0.1:1"),
            Zeroizing::new(KEY.into()),
            Duration::from_secs(300),
        )
        .unwrap()
    });
    drop(runtime);
    assert!(!lease.is_available());
}
