//! Authenticated declarations for original unknown effects, including tombstones.
use super::{frame::Frame, services::CoordinatorServices};
use nocterm_cloud_model::{coordinator::*, journal::*};

pub(super) async fn accept(
    services: &CoordinatorServices,
    command: &ReceivedCommand,
    work: &super::intake::PublicationWork,
) -> Result<CommandReceipt, CoordinatorError> {
    services.source().check_current()?;
    let CommandBody::Approval(answer) = command.body() else {
        return Err(CoordinatorError::InvalidCommand);
    };
    let ApprovalChoice::ReconcileKnown(declared) = answer.choice() else {
        return Err(CoordinatorError::InvalidCommand);
    };
    let now = services.timer().now();
    if now < command.window().issued()
        || now >= command.window().expires()
        || command.hard_expires().is_some_and(|expires| now >= expires)
    {
        return Err(CoordinatorError::Expired);
    }
    let (accepted_body, allocation) = work.encode(&Frame::accepted(command, None))?;
    let disk = services.io().acquire_resident(allocation).await?;
    let stream = command.stream().clone();
    let lookup = stream.clone();
    let head = services
        .io()
        .locked(&disk, move |journal| journal.head(lookup))
        .await?
        .ok_or(JournalError::NotFound)?;
    if command.acceptance().expected_control() != head.position().control() {
        return Err(CoordinatorError::Conflict);
    }
    let scope = stream.scope().clone();
    let target = answer.authority().session().target().clone();
    let unresolved = services
        .io()
        .locked(&disk, move |journal| {
            journal.unresolved_target(scope, target, 256)
        })
        .await?;
    let original = unresolved
        .into_iter()
        .find(|effect| effect.intent().id() == answer.intent())
        .ok_or(CoordinatorError::Conflict)?
        .intent()
        .clone();
    if original.authority() != answer.authority()
        || original.arguments() != answer.arguments()
        || original.authority().run().stream() != stream
    {
        return Err(CoordinatorError::Conflict);
    }
    let writer = services.io().owned_writer(&stream);
    let lease = if writer.as_ref() == Some(head.lease()) {
        CleanupLease::for_writer(head.lease().clone())
    } else if head.deleted() {
        let claim = WriterClaim::new(
            stream.clone(),
            Some(head.lease().epoch()),
            head.lease().epoch().next()?,
        )?;
        let lease = services
            .io()
            .cleanup_locked(&disk, move |cleanup| cleanup.claim_cleanup(claim))
            .await?;
        services.io().remember_writer(lease.writer().clone())?;
        lease
    } else {
        return Err(JournalError::Fenced.into());
    };
    // Native durable frames are independent from the sole public signed codec.
    // Both preserve the actual account declaration, never a remote exit/success.
    let declaration = CleanupDeclaration::new(
        command.acceptance().clone(),
        original.clone(),
        declared,
        accepted_body,
        Frame::Reconciled {
            original: (&original).into(),
            declared: *declared.as_bytes(),
        }
        .bytes()?,
    )?;
    let expected = head.position();
    let next = expected.next(2, 2, true)?;
    let accepted = CoordinatorEvent::CommandAccepted {
        command: command.acceptance().clone(),
        run: command.run().cloned(),
    };
    let fact = CoordinatorEvent::ApprovalResolved {
        command: command.acceptance().id(),
        answer: answer.clone(),
    };
    let events = [accepted, fact]
        .iter()
        .enumerate()
        .map(|(index, event)| {
            let position = JournalPosition::new(
                next.revision(),
                next.control(),
                expected.records().advance(index + 1)?,
                expected.events().advance(index + 1)?,
            );
            services.encoder().encode(
                lease.writer(),
                position,
                event,
                services.publication_work().cleanup().as_ref(),
            )
        })
        .collect::<Result<Vec<_>, JournalError>>()?;
    let batch = CleanupReconciliation::new(lease, expected, declaration, events)?;
    match services
        .io()
        .cleanup_locked(&disk, move |cleanup| cleanup.reconcile(batch))
        .await?
    {
        CommitOutcome::Duplicate(receipt) => Ok(*receipt),
        CommitOutcome::Committed(receipt) => receipt
            .command()
            .cloned()
            .ok_or_else(|| JournalError::Corrupt.into()),
    }
}
