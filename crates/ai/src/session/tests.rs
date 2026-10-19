use super::*;

fn ready() -> SessionState {
    let mut state = SessionState::default();
    assert!(state.queue());
    state.leased();
    state.opened();
    state
}

#[test]
fn a_detached_chat_queues_once_and_opens() {
    let mut state = SessionState::default();
    assert_eq!(state.phase(), SessionPhase::Detached);
    assert!(state.queue());
    assert!(!state.queue(), "a queued chat asks the runtime only once");
    assert!(state.queued());
    state.leased();
    assert!(state.opening() && state.busy());
    state.opened();
    assert_eq!(state.phase(), SessionPhase::Ready);
    assert!(!state.busy());
}

#[test]
fn turns_need_an_open_session_and_stale_results_change_nothing() {
    let mut state = SessionState::default();
    assert_eq!(state.start_turn(), None);
    let mut state = ready();
    let first = state.start_turn().expect("ready session starts a turn");
    assert!(state.generating());
    assert_eq!(state.start_turn(), None, "one turn at a time");
    assert!(state.finish_turn(first, true));
    let second = state.start_turn().unwrap();
    assert!(!state.finish_turn(first, true), "a stale turn is ignored");
    assert!(state.generating());
    assert!(state.finish_turn(second, false));
    assert_eq!(state.phase(), SessionPhase::Ready);
}

#[test]
fn stopping_drops_late_updates_until_the_next_turn() {
    let mut state = ready();
    let turn = state.start_turn().unwrap();
    assert!(state.stop());
    assert_eq!(state.phase(), SessionPhase::Cancelling);
    assert!(state.generating() && !state.accepts_updates());
    assert!(state.finish_turn(turn, false));
    assert!(state.stopped());
    state.start_turn().unwrap();
    assert!(state.accepts_updates() && !state.stopped());
    let mut idle = ready();
    assert!(!idle.stop(), "an idle session has nothing to cancel");
    assert!(idle.stopped() && idle.accepts_updates());
}

#[test]
fn a_failure_invalidates_requests_and_the_next_prompt_recovers() {
    let mut state = ready();
    let turn = state.start_turn().unwrap();
    state.fail();
    assert!(state.failed() && !state.accepts_updates() && !state.generating());
    assert!(!state.session_current(turn));
    assert!(!state.finish_turn(turn, true));
    state.detach();
    assert!(state.failed(), "detaching keeps the failure visible");
    assert!(state.closing());
    state.closed();
    assert!(state.queue(), "a failed chat reconnects on demand");
    state.leased();
    state.opened();
    assert!(state.start_turn().is_some());
}

#[test]
fn repeated_failures_stop_resuming_until_a_turn_completes() {
    let mut state = ready();
    assert!(state.may_resume());
    state.fail();
    assert!(state.may_resume(), "one failure retries the same session");
    state.queue();
    state.leased();
    state.fail();
    assert!(!state.may_resume());
    state.queue();
    state.leased();
    state.opened();
    let turn = state.start_turn().unwrap();
    state.finish_turn(turn, true);
    assert!(state.may_resume());
}

#[test]
fn signing_in_returns_to_where_the_chat_was() {
    let mut state = ready();
    assert!(!state.begin_sign_in());
    state.require_sign_in();
    assert!(state.sign_in_required() && state.busy());
    assert!(state.begin_sign_in());
    assert!(state.signing_in());
    state.sign_in_finished(false, true);
    assert_eq!(state.phase(), SessionPhase::SignInRequired);
    state.begin_sign_in();
    state.sign_in_finished(true, true);
    assert_eq!(state.phase(), SessionPhase::Ready);

    let mut state = SessionState::default();
    state.queue();
    state.leased();
    state.require_sign_in();
    state.begin_sign_in();
    state.sign_in_finished(true, false);
    assert!(state.opening(), "without a session it goes on opening one");
}

#[test]
fn detaching_an_idle_chat_invalidates_its_session() {
    let mut state = ready();
    let ticket = state.ticket();
    state.detach();
    assert_eq!(state.phase(), SessionPhase::Detached);
    assert!(!state.session_current(ticket));
    state.fail();
    state.release();
    assert_eq!(state.phase(), SessionPhase::Detached);
}

#[test]
fn a_missing_session_or_a_failed_fork_starts_over() {
    let auth = AgentError::AuthRequired("login".into());
    let gone = AgentError::RestoreUnavailable("no rollout".into());
    let other = AgentError::Io("broken pipe".into());
    let last = RESTORE_ATTEMPTS - 1;
    for fork in [false, true] {
        assert_eq!(restore_fallback(&auth, 0, fork), RestoreFallback::SignIn);
        assert_eq!(
            restore_fallback(&gone, 0, fork),
            RestoreFallback::StartFresh
        );
        assert_eq!(restore_fallback(&other, 0, fork), RestoreFallback::Retry);
    }
    assert_eq!(
        restore_fallback(&other, last, true),
        RestoreFallback::StartFresh
    );
    assert_eq!(
        restore_fallback(&other, last, false),
        RestoreFallback::Fail,
        "a transient error keeps the chat's own session for the next try"
    );
}
