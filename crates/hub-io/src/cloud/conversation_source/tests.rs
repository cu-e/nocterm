//! Real crypto/private files and real finite ownership; no transport fake.
use super::*;
use nocterm_cloud_model::{
    coordinator::IntakePoolLimits,
    journal::{PublicationAllocation, PublicationBudget, PublicationWorkBudget},
};
use nocterm_hub_model::cloud::{
    ChatCoordinate, CloudNamespace, CloudRoute, CloudTextId, CommandContext, CommandHeader,
    CommandIssuer, CommandKeys, CommandKind, RunCoordinate, RunnerCoordinate, SignedCommand,
    payload::CloudCommandPayload,
    relay::{PublicationMetadata, RelayBinding},
};
use nocterm_keys::{
    AccountKeyPair, DeviceKey,
    cloud::{ChatKey, RunnerEncryptionKey, open_command, seal_command},
};
use sha2::{Digest, Sha256};

fn budget(bytes: usize) -> PublicationBudget {
    PublicationBudget::maintenance(IntakePoolLimits::new(bytes, 1, 1).unwrap())
}
fn reserve(budget: &PublicationBudget, bytes: usize) -> AllocationLease {
    budget
        .try_reserve(PublicationAllocation::new(bytes, 1).unwrap())
        .unwrap()
}
fn chat() -> ChatCoordinate {
    ChatCoordinate::new(
        CloudNamespace {
            hub_instance_id: CloudTextId::new("source-hub").unwrap(),
            account_id: CloudTextId::new("source-account").unwrap(),
        },
        RunnerCoordinate {
            id: [1; 16],
            key_epoch: 2,
        },
        [3; 16],
        4,
    )
    .unwrap()
}
fn command() -> (outbox::StoredCommand, RunnerEncryptionKey, AccountKeyPair) {
    let account = AccountKeyPair::generate().unwrap();
    let runner = RunnerEncryptionKey::generate().unwrap();
    let device = DeviceKey::generate().unwrap();
    let route = CloudRoute {
        namespace: chat().namespace().clone(),
        runner: chat().runner(),
        chat_id: [3; 16],
        command_id: [5; 16],
    };
    let payload = CloudCommandPayload::Stop.canonical_bytes().unwrap();
    let header = CommandHeader::new(
        route.clone(),
        CommandKeys {
            account: account.public_key().to_bytes(),
            runner: runner.public_key().to_bytes(),
        },
        CommandIssuer {
            device_id: CloudTextId::new("device").unwrap(),
            signing_key: device.public_key(),
            access_token_digest: [6; 32],
        },
        CommandContext {
            nonce: [7; 16],
            expected_chat_revision: 8,
            run: Some(RunCoordinate {
                id: [9; 16],
                expected_epoch: 10,
            }),
            kind: CommandKind::Stop,
            issued_at_ms: 100_000,
            accept_before_ms: 160_000,
        },
        &payload,
    )
    .unwrap();
    let canonical = header.canonical_bytes().unwrap();
    let semantic = Sha256::digest(&canonical).into();
    let signed = SignedCommand::new(header, &payload, device.sign(&canonical)).unwrap();
    let ciphertext = seal_command(&account, &runner.public_key(), &signed).unwrap();
    let metadata = PublicationMetadata::new(
        [11; 16],
        RelayBinding::command(route).unwrap(),
        ciphertext.len() as u32,
        Sha256::digest(&ciphertext).into(),
    )
    .unwrap();
    (
        outbox::StoredCommand {
            intent: Fingerprint::from_bytes([12; 32]),
            semantic,
            control: 8,
            metadata,
            ciphertext,
        },
        runner,
        account,
    )
}

#[test]
fn conversation_source_relogin_shares_physical_driver_and_retries_exact_durable_ciphertext() {
    let directory = tempfile::tempdir().unwrap();
    let resident = ConversationStorage::resident_bytes(2).unwrap();
    let storage = ConversationStorage::new(
        directory.path().to_path_buf(),
        2,
        reserve(&budget(resident), resident),
    )
    .unwrap();
    let namespace = directory.path().join("same-original-account-and-device");
    let old_view = storage.open(namespace.clone()).unwrap();
    let (original, runner, account) = command();
    let key = ChatKey::generate().unwrap();
    let id = CommandId::from_bytes([5; 16]);
    let writer = WriterEpoch::new(13).unwrap();
    let sealed = original.seal(&key, chat(), id, writer).unwrap();
    {
        let file = old_view.outbox().lock().unwrap();
        let committed = file
            .stage(id, &sealed, writer.get(), true)
            .unwrap()
            .publish_new()
            .unwrap();
        file.confirm(&committed).unwrap();
    }
    // Original view remains alive, including its exclusive OS lock. The new
    // capture upgrades the installation's same physical driver without relocking.
    let new_login = storage.open(namespace.clone()).unwrap();
    assert!(std::ptr::eq(old_view.outbox(), new_login.outbox()));
    let cell = new_login
        .outbox()
        .lock()
        .unwrap()
        .read(id, 16384)
        .unwrap()
        .unwrap();
    let restored = outbox::StoredCommand::open(&key, chat(), id, writer, &cell).unwrap();
    assert_eq!(restored.metadata, original.metadata);
    assert_eq!(restored.ciphertext, original.ciphertext);
    assert_eq!(restored.semantic, original.semantic);
    require_original_command(&restored, &runner, &account);
    assert!(
        outbox::StoredCommand::open(&key, chat(), CommandId::from_bytes([14; 16]), writer, &cell)
            .is_err()
    );
    let mut tampered = storage::SealedCell {
        writer: cell.writer,
        ciphertext: cell.ciphertext.clone(),
    };
    tampered.ciphertext[10] ^= 1;
    assert!(outbox::StoredCommand::open(&key, chat(), id, writer, &tampered).is_err());
    new_login.outbox().lock().unwrap().archive(id).unwrap();
    assert!(
        old_view
            .outbox()
            .lock()
            .unwrap()
            .pending()
            .unwrap()
            .is_empty()
    );
    drop(new_login);
    drop(old_view);
    let restarted = storage.open(namespace).unwrap();
    let cell = restarted
        .outbox()
        .lock()
        .unwrap()
        .read(id, 16384)
        .unwrap()
        .unwrap();
    assert_eq!(
        outbox::StoredCommand::open(&key, chat(), id, writer, &cell)
            .unwrap()
            .ciphertext,
        original.ciphertext
    );
}

#[tokio::test]
async fn conversation_source_cancellation_retains_physical_work_and_urgent_lane() {
    let workers = ConversationWorkers::new(tokio::runtime::Handle::current(), 2).unwrap();
    let quota = budget(8192);
    let lease = reserve(&quota, 8192);
    let slices = work::WorkSlices::new(lease.clone(), 8192).unwrap();
    let body = slices.reserve(8192).unwrap();
    assert!(slices.reserve(1).is_err());
    drop(body);
    assert!(slices.reserve(8192).is_ok());
    drop(slices);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let physical = workers
        .blocking(lease, false, move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(vec![1u8; 4096])
        })
        .unwrap();
    entered_rx.await.unwrap();
    // Dropping/aborting the waiter does not cancel an already running closure.
    physical.abort();
    assert!(
        quota
            .try_reserve(PublicationAllocation::new(1, 1).unwrap())
            .is_err()
    );
    assert!(
        workers
            .blocking(reserve(&budget(1), 1), false, || Ok(()))
            .is_err()
    );
    let urgent = workers
        .blocking(reserve(&budget(1), 1), true, || Ok(()))
        .unwrap();
    let urgent = urgent.await.unwrap();
    assert_eq!(urgent.result(), Ok(&()));
    drop(urgent);
    release_tx.send(()).unwrap();
    let result = physical.await.unwrap();
    assert!(
        quota
            .try_reserve(PublicationAllocation::new(1, 1).unwrap())
            .is_err()
    );
    let admitted = result.into_allocated().unwrap();
    assert_eq!(admitted.value().len(), 4096);
    assert!(
        quota
            .try_reserve(PublicationAllocation::new(1, 1).unwrap())
            .is_err()
    );
    drop(admitted);
    assert!(
        quota
            .try_reserve(PublicationAllocation::new(8192, 1).unwrap())
            .is_ok()
    );
}

fn require_original_command(
    restored: &outbox::StoredCommand,
    runner: &RunnerEncryptionKey,
    account: &AccountKeyPair,
) {
    let route = restored.metadata.binding().command_route().unwrap();
    let opened = open_command(runner, &account.public_key(), route, &restored.ciphertext).unwrap();
    assert_eq!(
        <[u8; 32]>::from(Sha256::digest(
            opened
                .command()
                .unwrap()
                .header()
                .canonical_bytes()
                .unwrap()
        )),
        restored.semantic
    );
}

#[tokio::test]
async fn conversation_source_dropped_offered_page_reopens_without_rewinding_verified_highwater() {
    let key = ChatKey::generate().unwrap();
    let signer = nocterm_keys::cloud::RunnerSigningKey::generate().unwrap();
    require_attested_proof_separate_from_record_signature(&signer);
    let origin = ProjectionCursor::origin();
    let first = verified_cursor(&key, &signer, 1);
    let second = verified_cursor(&key, &signer, 2);
    let routes = Arc::new(Mutex::new(routes::ReplayRoutes::new()));
    let workers = ConversationWorkers::new(tokio::runtime::Handle::current(), 2).unwrap();
    let physical_routes = routes.clone();
    let result = workers
        .blocking(reserve(&budget(8192), 8192), false, move || {
            physical_routes.lock().unwrap().offer(origin, first, 11)?;
            Ok(first)
        })
        .unwrap()
        .await
        .unwrap();
    // Physical completion offered the page; the observer abandons that result
    // before publication. Its previous exact cursor remains valid for retry.
    drop(result);
    let mut routes = routes.lock().unwrap();
    assert_eq!(routes.high(), first);
    assert_eq!(routes.delivery(origin), Ok(0));
    assert_eq!(
        routes.requirements(origin, first).unwrap().bytes(),
        requirements::replay(MAX_WIRE_BYTES).unwrap()
    );
    routes.offer(origin, first, 11).unwrap();
    assert_eq!(
        routes.requirements(first, first).unwrap().bytes(),
        heads::work_bytes()
    );
    assert_eq!(
        routes.requirements(first, second).unwrap().bytes(),
        requirements::replay(MAX_WIRE_BYTES).unwrap()
    );
    routes.offer(first, second, 18).unwrap();
    routes.offer(origin, first, 11).unwrap();
    assert_eq!(routes.high(), second);
    assert_eq!(routes.delivered(), 18);
    assert_eq!(routes.delivery(first), Ok(11));
    let conflicting = ProjectionCursor::at(first.sequence(), Fingerprint::from_bytes([255; 32]));
    assert_eq!(routes.delivery(conflicting), Err(ProjectionError::Rollback));
    assert_eq!(
        routes.offer(origin, first, 12),
        Err(ProjectionError::Rollback)
    );
    let mut previous = second;
    for sequence in 3..=70 {
        let next = verified_cursor(&key, &signer, sequence);
        routes.offer(previous, next, sequence * 10).unwrap();
        previous = next;
    }
    assert_eq!(routes.delivery(first), Err(ProjectionError::Unsupported));
    assert_eq!(routes.delivery(origin), Ok(0));
    assert_eq!(routes.high(), previous);
    assert_eq!(
        routes.requirements(origin, previous).unwrap().bytes(),
        requirements::replay(MAX_WIRE_BYTES).unwrap()
    );
}
fn verified_cursor(
    key: &ChatKey,
    signer: &nocterm_keys::cloud::RunnerSigningKey,
    sequence: u64,
) -> ProjectionCursor {
    use nocterm_hub_model::cloud::{
        RecordContext, RecordPosition,
        events::{CloudEvent, CloudEventPayload, CloudRunStatus},
    };
    let run = RunCoordinate {
        id: [9; 16],
        expected_epoch: 10,
    };
    let context = RecordContext::new(
        chat(),
        RecordPosition {
            writer_epoch: 13,
            sequence,
            journal_sequence: sequence,
            revision: sequence,
        },
        [sequence as u8; 16],
        Some(run),
    )
    .unwrap();
    let bytes = CloudEvent {
        control_revision: 1,
        timestamp_ms: 100_000 + sequence,
        payload: CloudEventPayload::Status {
            run,
            status: CloudRunStatus::Running,
        },
    }
    .canonical_bytes()
    .unwrap();
    let sealed = nocterm_keys::cloud::seal_event(key, signer, context.clone(), &bytes).unwrap();
    let opened =
        nocterm_keys::cloud::open_event(key, &signer.public_key(), &context, &sealed).unwrap();
    let record = opened.record().unwrap();
    CloudEvent::from_record(&record).unwrap();
    ProjectionCursor::at(
        EventSequence::from_u64(record.header().context().position().sequence),
        Fingerprint::from_bytes(Sha256::digest(&sealed).into()),
    )
}

fn require_attested_proof_separate_from_record_signature(
    signer: &nocterm_keys::cloud::RunnerSigningKey,
) {
    let proof = DeviceKey::generate().unwrap().public_key();
    let stream = nocterm_cloud_model::projection::testing::run_binding(9).stream();
    let origin = |key| nocterm_hub_client::cloud::CloudDeliveryOrigin::Runner {
        account: CloudTextId::new(stream.scope().account().id().as_str()).unwrap(),
        runner: *stream.runner().as_bytes(),
        key,
    };
    assert_ne!(proof, signer.public_key());
    assert_eq!(
        replay::require_origin(&stream, &proof, &origin(proof)),
        Ok(())
    );
    assert_eq!(
        replay::require_origin(&stream, &proof, &origin(signer.public_key())),
        Err(ProjectionError::WrongSource)
    );
}

#[tokio::test]
async fn conversation_source_observed_clock_rollback_wakes_ended_and_fences_publication() {
    use nocterm_cloud_model::testing::ManualToolTimer;
    use std::cell::Cell;
    for under_publication in [false, true] {
        let timer = Arc::new(ManualToolTimer::default());
        timer.set(1000);
        let clock = SourceClock::new(timer.clone());
        let lifetime = lifetime::SourceLifetime::new();
        let ended = lifetime.ended(|| clock.now().map(|_| ()), futures::future::pending());
        tokio::pin!(ended);
        assert!(futures::poll!(ended.as_mut()).is_pending());
        let mut notice = lifetime.subscribe();
        let invoked = Cell::new(false);
        timer.set(999);
        let refusal = if under_publication {
            lifetime.publish(
                || clock.now().map(|_| ()),
                || {
                    invoked.set(true);
                    Ok(())
                },
            )
        } else {
            // Timestamp-returning path is also used by direct preparation.
            lifetime.check(|| clock.now()).map(|_| ())
        };
        assert_eq!(refusal, Err(ProjectionError::Rollback));
        assert!(!invoked.get());
        assert!(futures::poll!(ended.as_mut()).is_ready());
        assert!(notice.has_changed().unwrap());
        notice.borrow_and_update();
        timer.set(2000);
        assert_eq!(clock.now(), Err(ProjectionError::Rollback));
        assert_eq!(
            lifetime.check(|| clock.now().map(|_| ())),
            Err(ProjectionError::Rollback)
        );
        assert_eq!(
            lifetime.publish(
                || clock.now().map(|_| ()),
                || {
                    invoked.set(true);
                    Ok(())
                }
            ),
            Err(ProjectionError::Rollback)
        );
        assert!(!invoked.get());
        assert!(!notice.has_changed().unwrap());
        let already_retired =
            lifetime.ended(|| clock.now().map(|_| ()), futures::future::pending());
        tokio::pin!(already_retired);
        assert!(futures::poll!(already_retired.as_mut()).is_ready());
    }
}

#[test]
fn conversation_source_controls_need_no_provider_and_encode_exact_canonical_approval() {
    use nocterm_cloud_model::{
        authority::RunBinding,
        coordinator::{ApprovalAnswer, ApprovalChoice},
        projection::{ApprovalRequest, ConversationIntent},
    };
    let mut policy = policy::ConversationPolicy::controls(60_000).unwrap();
    policy.bind_capture().unwrap();
    let issued = Timestamp::from_millis(100_000);
    assert!(
        policy
            .preparation(
                issued,
                170_000,
                &ConversationIntent::Prompt("no provider".into())
            )
            .is_err()
    );
    let stop = ConversationIntent::Stop;
    let prep = policy.preparation(issued, 170_000, &stop).unwrap();
    assert!(matches!(
        commands::payload(&policy, &stop, prep).unwrap(),
        CloudCommandPayload::Stop
    ));
    let run = RunBinding::new(
        AccountScope::new(
            HubIdentity::new(
                NamespaceId::new("source-hub").unwrap(),
                Fingerprint::from_bytes([1; 32]),
            ),
            AccountIdentity::new(
                NamespaceId::new("source-account").unwrap(),
                Fingerprint::from_bytes([2; 32]),
            ),
        ),
        RunnerId::from_bytes([1; 16]),
        RunnerKeyEpoch::new(2).unwrap(),
        CloudChatId::from_bytes([3; 16]),
        RunId::from_bytes([9; 16]),
        RunEpoch::new(10).unwrap(),
    );
    let authority = nocterm_cloud_model::projection::testing::execution_authority(run);
    let answer = ApprovalAnswer::new(
        IntentId::from_bytes([17; 16]),
        authority.clone(),
        Fingerprint::from_bytes([18; 32]),
        ApprovalChoice::Approve,
    );
    let action = ConversationIntent::AnswerApproval(Box::new(
        ApprovalRequest::new(answer, Timestamp::from_millis(150_000)).unwrap(),
    ));
    let prep = policy.preparation(issued, 170_000, &action).unwrap();
    assert_eq!(prep.expires().as_millis(), 150_000);
    assert!(
        policy
            .preparation(Timestamp::from_millis(150_000), 170_000, &action)
            .is_err()
    );
    let payload = commands::payload(&policy, &action, prep).unwrap();
    let body = payload.canonical_bytes().unwrap();
    require_encoded_approval(&body, prep, &authority);
}

fn require_encoded_approval(
    body: &[u8],
    prep: nocterm_cloud_model::projection::ports::ConversationPreparation,
    authority: &nocterm_cloud_model::authority::ExecutionAuthority,
) {
    let (account, runner, device) = (
        AccountKeyPair::generate().unwrap(),
        RunnerEncryptionKey::generate().unwrap(),
        DeviceKey::generate().unwrap(),
    );
    let route = CloudRoute {
        namespace: chat().namespace().clone(),
        runner: chat().runner(),
        chat_id: chat().chat_id(),
        command_id: [19; 16],
    };
    let header = CommandHeader::new(
        route.clone(),
        CommandKeys {
            account: account.public_key().to_bytes(),
            runner: runner.public_key().to_bytes(),
        },
        CommandIssuer {
            device_id: CloudTextId::new("device").unwrap(),
            signing_key: device.public_key(),
            access_token_digest: [20; 32],
        },
        CommandContext {
            nonce: [21; 16],
            expected_chat_revision: 22,
            run: Some(RunCoordinate {
                id: *authority.run().run().as_bytes(),
                expected_epoch: authority.run().epoch().get(),
            }),
            kind: CommandKind::ResolveApproval,
            issued_at_ms: prep.issued().as_millis() as i64,
            accept_before_ms: prep.expires().as_millis() as i64,
        },
        body,
    )
    .unwrap();
    let canonical = header.canonical_bytes().unwrap();
    let signed = SignedCommand::new(header, body, device.sign(&canonical)).unwrap();
    let sealed = seal_command(&account, &runner.public_key(), &signed).unwrap();
    let opened = open_command(&runner, &account.public_key(), &route, &sealed).unwrap();
    let signed = opened.command().unwrap();
    let CloudCommandPayload::ResolveApproval(decoded) =
        CloudCommandPayload::from_command(&signed).unwrap()
    else {
        panic!("expected actual approval codec");
    };
    assert_eq!(decoded.intent, [17; 16]);
    assert_eq!(decoded.arguments_digest, [18; 32]);
    assert_eq!(
        decoded.execution.session.id,
        *authority.session().id().as_bytes()
    );
    assert_eq!(decoded.execution.grant.id, *authority.grant().as_bytes());
    assert_eq!(
        decoded.choice,
        nocterm_hub_model::cloud::payload::ApprovalChoice::Approve
    );
}

#[test]
fn conversation_source_provider_grant_carries_the_sealed_capsule_and_never_outlives_it() {
    use nocterm_cloud_model::projection::{ConversationIntent, ProviderAccess};
    use nocterm_hub_model::cloud::provider::{
        BootDescriptor, BootWindow, ProviderLeaseContext,
    };
    use nocterm_keys::cloud::{RunnerBootKey, RunnerSigningKey, seal_provider_lease};
    let account = AccountKeyPair::generate().unwrap();
    let boot = RunnerBootKey::generate().unwrap();
    let signing = RunnerSigningKey::from_seed(&[1; 32]);
    let descriptor = BootDescriptor::new(
        chat().namespace().clone(),
        chat().runner(),
        [4; 16],
        boot.public_key().to_bytes(),
        BootWindow {
            issued_at_ms: 100_000,
            expires_at_ms: 160_000,
        },
    )
    .unwrap();
    let signed = signing.attest_boot(descriptor).unwrap();
    let verified = signed
        .verify(
            &signing.public_key(),
            signed.descriptor().namespace(),
            signed.descriptor().runner(),
            110_000,
        )
        .unwrap();
    let context = ProviderLeaseContext::new(
        signed.descriptor().clone(),
        CloudTextId::new("openai").unwrap(),
        [5; 16],
        150_000,
    )
    .unwrap();
    let capsule =
        seal_provider_lease(&account, &verified, &context, b"sk-secret", 110_000).unwrap();
    let access = ProviderAccess::new(
        [5; 16],
        2,
        context.canonical_bytes().unwrap(),
        vec!["gpt-5".into()],
        capsule.clone(),
        Timestamp::from_millis(150_000),
    )
    .unwrap();
    assert!(!format!("{access:?}").contains("sk-secret"));
    let mut policy = policy::ConversationPolicy::controls(60_000).unwrap();
    policy.bind_capture().unwrap();
    let action = ConversationIntent::GrantProvider(Box::new(access));
    let prep = policy
        .preparation(Timestamp::from_millis(110_000), 400_000, &action)
        .unwrap();
    assert_eq!(prep.expires().as_millis(), 150_000);
    let CloudCommandPayload::IssueGrant(grant) =
        commands::payload(&policy, &action, prep).unwrap()
    else {
        panic!("a provider lease is an IssueGrant command");
    };
    let nocterm_hub_model::cloud::payload::GrantPayload::Provider {
        grant,
        context: carried,
        allowed_models,
        capsule: carried_capsule,
    } = *grant
    else {
        panic!("expected the provider grant");
    };
    assert_eq!((grant.id, grant.epoch), ([5; 16], 2));
    assert_eq!(carried, context);
    assert_eq!(allowed_models.len(), 1);
    assert_eq!(carried_capsule, capsule.as_slice());
    // The runner side opens exactly what was sealed on this device.
    let opened = boot
        .open_provider_lease(&account.public_key(), &carried, carried_capsule, 110_000)
        .unwrap();
    assert_eq!(opened.secret(), b"sk-secret");
}
