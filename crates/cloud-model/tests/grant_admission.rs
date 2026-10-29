mod support;
use nocterm_ai::{
    CancelCommand, ExecCommand, OpenTerminal, ReadCommand, ReadTerminal, RunCommand, SendInput,
    TerminalCall, TerminalCallError,
};
use nocterm_cloud_model::{
    authority::{AuthorityError, ExecutionAuthority, SessionReceipt},
    grant::{
        Capabilities, GrantError, GrantRevocation, GrantState, GrantWindow, MAX_TASK_GRANT_MILLIS,
    },
    identity::{GrantEpoch, GrantId, RemoteSessionId, SessionEpoch, Timestamp},
};
use std::error::Error;
use support::*;

fn grant(capabilities: Capabilities, operations: u32) -> GrantState {
    GrantState::new(
        authority(run(1, 1, 1, 1)),
        origin(),
        capabilities,
        GrantWindow::new(Timestamp::from_millis(100), Timestamp::from_millis(1000)).unwrap(),
        operations,
    )
    .unwrap()
}
fn calls() -> Vec<TerminalCall> {
    vec![
        TerminalCall::ListTerminals,
        TerminalCall::ReadTerminal(ReadTerminal {
            terminal_id: "t".into(),
            lines: None,
            since: None,
        }),
        TerminalCall::ReadCommand(ReadCommand {
            terminal_id: "t".into(),
            command_id: "c".into(),
            yield_ms: None,
        }),
        TerminalCall::SendInput(SendInput {
            terminal_id: "t".into(),
            text: "hello".into(),
            press_enter: false,
        }),
        TerminalCall::RunCommand(RunCommand {
            terminal_id: "t".into(),
            command: "true".into(),
            timeout_ms: None,
            idle_ms: None,
        }),
        TerminalCall::ExecCommand(ExecCommand {
            terminal_id: "t".into(),
            program: "printf".into(),
            args: vec![],
            stdin: None,
            timeout_ms: None,
            yield_ms: None,
        }),
        TerminalCall::CancelCommand(CancelCommand {
            terminal_id: "t".into(),
            command_id: "c".into(),
        }),
        TerminalCall::OpenTerminal(OpenTerminal {
            server_id: "s".into(),
        }),
    ]
}

#[test]
fn read_only_grant_never_admits_shell_input_execution_connection_or_cancellation() {
    let expected = authority(run(1, 1, 1, 1));
    let mut state = grant(Capabilities::READ_ONLY, 3);
    for call in calls() {
        let result = state.authorize(&expected, &call, Timestamp::from_millis(200));
        if call.writes() {
            assert!(matches!(result, Err(GrantError::CapabilityDenied)));
        } else {
            assert!(result.is_ok());
        }
    }
    assert_eq!(state.remaining_operations(), 0);
    assert!(matches!(
        state.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(200)
        ),
        Err(GrantError::BudgetExceeded)
    ));
}

#[test]
fn changed_scope_run_target_session_or_grant_refuses_before_spending_budget() {
    let original = authority(run(1, 1, 1, 1));
    let changed = [
        (authority(run(2, 1, 1, 1)), AuthorityError::WrongScope),
        (authority(run(1, 1, 1, 2)), AuthorityError::StaleRun),
        (
            ExecutionAuthority::new(
                original.run().clone(),
                SessionReceipt::new(
                    original.session().id(),
                    original.session().epoch(),
                    target(2),
                    authentication(),
                ),
                original.grant(),
                original.grant_epoch(),
            ),
            AuthorityError::WrongTarget,
        ),
        (
            ExecutionAuthority::new(
                original.run().clone(),
                SessionReceipt::new(
                    RemoteSessionId::from_bytes([2; 16]),
                    SessionEpoch::new(2).unwrap(),
                    target(1),
                    authentication(),
                ),
                original.grant(),
                original.grant_epoch(),
            ),
            AuthorityError::StaleSession,
        ),
        (
            ExecutionAuthority::new(
                original.run().clone(),
                original.session().clone(),
                GrantId::from_bytes([2; 16]),
                GrantEpoch::new(2).unwrap(),
            ),
            AuthorityError::StaleGrant,
        ),
    ];
    for (expected, refusal) in changed {
        let mut state = grant(Capabilities::ALL, 2);
        assert!(
            matches!(state.authorize(&expected, &TerminalCall::ListTerminals, Timestamp::from_millis(200)),
            Err(GrantError::Authority(error)) if error == refusal)
        );
        assert_eq!(state.remaining_operations(), 2);
    }
}

#[test]
fn rollback_revokes_permanently_and_does_not_resume_when_time_catches_up() {
    let expected = authority(run(1, 1, 1, 1));
    let mut state = grant(Capabilities::ALL, 4);
    state
        .authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(200),
        )
        .unwrap();
    assert!(matches!(
        state.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(199)
        ),
        Err(GrantError::ClockRollback)
    ));
    assert_eq!(state.revocation(), Some(GrantRevocation::ClockRollback));
    assert!(matches!(
        state.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(300)
        ),
        Err(GrantError::Revoked)
    ));
    assert_eq!(state.remaining_operations(), 3);
}

#[test]
fn explicit_revoke_and_exact_expiry_fence_new_operations() {
    let expected = authority(run(1, 1, 1, 1));
    let mut revoked = grant(Capabilities::ALL, 2);
    revoked.revoke();
    assert!(matches!(
        revoked.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(200)
        ),
        Err(GrantError::Revoked)
    ));
    let mut expired = grant(Capabilities::ALL, 2);
    assert!(matches!(
        expired.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(1000)
        ),
        Err(GrantError::Expired)
    ));
    assert_eq!(expired.remaining_operations(), 2);
}

#[test]
fn malformed_arguments_keep_typed_cause_and_do_not_reserve_an_operation() {
    let expected = authority(run(1, 1, 1, 1));
    let mut state = grant(Capabilities::ALL, 2);
    let bad = TerminalCall::SendInput(SendInput {
        terminal_id: "t".into(),
        text: "\0".into(),
        press_enter: false,
    });
    let error = state
        .authorize(&expected, &bad, Timestamp::from_millis(200))
        .unwrap_err();
    assert!(matches!(
        error,
        GrantError::InvalidArguments(TerminalCallError::InputRejected)
    ));
    assert!(error.source().unwrap().is::<TerminalCallError>());
    assert_eq!(state.remaining_operations(), 2);
}

#[test]
fn deadlines_and_operation_budget_are_finite_and_overflow_safe() {
    for (issued, expires) in [
        (100, 100),
        (100, 99),
        (0, MAX_TASK_GRANT_MILLIS + 1),
        (0, u64::MAX),
    ] {
        assert!(matches!(
            GrantWindow::new(
                Timestamp::from_millis(issued),
                Timestamp::from_millis(expires)
            ),
            Err(GrantError::InvalidDeadline)
        ));
    }
    assert!(matches!(
        GrantState::new(
            authority(run(1, 1, 1, 1)),
            origin(),
            Capabilities::ALL,
            GrantWindow::new(
                Timestamp::from_millis(0),
                Timestamp::from_millis(MAX_TASK_GRANT_MILLIS)
            )
            .unwrap(),
            0
        ),
        Err(GrantError::InvalidBudget)
    ));
}

#[test]
fn selected_authentication_key_and_credential_revision_fence_unchanged_session_ids() {
    use nocterm_cloud_model::authority::AuthenticationBinding;
    let original = authority(run(1, 1, 1, 1));
    for authentication in [
        AuthenticationBinding::new(fingerprint(99), fingerprint(81)),
        AuthenticationBinding::new(fingerprint(80), fingerprint(99)),
    ] {
        let expected = ExecutionAuthority::new(
            original.run().clone(),
            SessionReceipt::new(
                original.session().id(),
                original.session().epoch(),
                original.session().target().clone(),
                authentication,
            ),
            original.grant(),
            original.grant_epoch(),
        );
        let mut state = grant(Capabilities::ALL, 2);
        assert!(matches!(
            state.authorize(
                &expected,
                &TerminalCall::ListTerminals,
                Timestamp::from_millis(200)
            ),
            Err(GrantError::Authority(AuthorityError::StaleSession))
        ));
        assert_eq!(state.remaining_operations(), 2);
    }
}

#[test]
fn original_grant_admission_remains_addressable_after_an_issuer_refresh() {
    use nocterm_cloud_model::identity::{ChatControlRevision, CommandId};
    use nocterm_cloud_model::{authority::CommandIssuer, grant::GrantOrigin};
    let original = origin();
    let refreshed = GrantOrigin::new(
        CommandIssuer::new(
            original.issuer().device().clone(),
            original.issuer().key(),
            fingerprint(99),
        ),
        CommandId::from_bytes([2; 16]),
        ChatControlRevision::from_u64(2),
    );
    let mut state = grant(Capabilities::ALL, 2);
    assert_eq!(state.origin(), &original);
    assert_ne!(state.origin().issuer(), refreshed.issuer());
    assert_ne!(state.origin().command(), refreshed.command());
    assert!(state.origin().control() < refreshed.control());
    state.revoke();
    assert_eq!(state.origin(), &original);
}

#[test]
fn an_admitted_effect_can_recheck_current_authority_without_spending_again() {
    let expected = authority(run(1, 1, 1, 1));
    let mut state = grant(Capabilities::READ_ONLY, 1);
    state
        .authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(200),
        )
        .unwrap();
    for now in [200, 201, 202] {
        state
            .revalidate_reserved(
                &expected,
                &TerminalCall::ListTerminals,
                Timestamp::from_millis(now),
            )
            .unwrap();
    }
    assert_eq!(state.remaining_operations(), 0);
    assert!(matches!(
        state.authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(202)
        ),
        Err(GrantError::BudgetExceeded)
    ));
    state.revoke();
    assert!(matches!(
        state.revalidate_reserved(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(203)
        ),
        Err(GrantError::Revoked)
    ));
}

#[test]
fn restored_grants_retain_exhaustion_rollback_and_original_policy() {
    use nocterm_cloud_model::grant::GrantSnapshot;
    let expected = authority(run(1, 1, 1, 1));
    let mut state = grant(Capabilities::READ_ONLY, 1);
    state
        .authorize(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(200),
        )
        .unwrap();
    let snapshot = state.snapshot();
    assert!(matches!(
        GrantSnapshot::new(snapshot.policy().clone(), 2, snapshot.observed(), None),
        Err(GrantError::InvalidRecovery)
    ));
    let mut restored = GrantState::from_snapshot(snapshot);
    assert_eq!(restored.remaining_operations(), 0);
    assert_eq!(restored.origin(), &origin());
    assert!(matches!(
        restored.revalidate_reserved(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(199)
        ),
        Err(GrantError::ClockRollback)
    ));
    let mut restored = GrantState::from_snapshot(restored.snapshot());
    assert!(matches!(
        restored.revalidate_reserved(
            &expected,
            &TerminalCall::ListTerminals,
            Timestamp::from_millis(300)
        ),
        Err(GrantError::Revoked)
    ));
    assert!(Capabilities::from_bits(16).is_err());
    assert_eq!(
        Capabilities::from_bits(Capabilities::READ_ONLY.bits()).unwrap(),
        Capabilities::READ_ONLY
    );
}
