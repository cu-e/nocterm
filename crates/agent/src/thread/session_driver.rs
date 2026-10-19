//! Session and lease transitions retain their original connection and operation identity.
use super::Restore;
use crate::runtime::SessionLease;
use nocterm_ai::{
    BridgeRegistration,
    connection::ManagedCommandsHandle,
    history::{
        live::LiveEpoch,
        source::{ManagedCommandError, ManagedSourceFuture},
    },
    model::{ids::SessionId, json::deferred::NativeAllocationError},
    session::{SessionPhase, SessionState, Ticket},
};
use std::path::PathBuf;
pub(in crate::thread) mod authentication_operation;
pub(in crate::thread) mod cancel_operation;
pub(in crate::thread) mod open_operation;
mod operation_counter;
pub(in crate::thread) mod prompt_operation;
use operation_counter::OperationCounter;
pub(crate) use operation_counter::OperationGuard;

/// Awaits one admitted native command, mapping a refused admission to its typed error.
pub(super) async fn settle<T>(
    operation: Result<ManagedSourceFuture<Result<T, ManagedCommandError>>, NativeAllocationError>,
) -> Result<T, ManagedCommandError> {
    match operation {
        Ok(work) => work.await,
        Err(error) => Err(ManagedCommandError::Admission(error)),
    }
}

#[derive(Default)]
pub(super) struct SessionDriver {
    lease: Option<SessionLease>,
    phase: SessionState,
    shown: bool,
    operations: OperationCounter,
    restore: Option<Restore>,
    epoch: Option<LiveEpoch>,
}
#[derive(Clone)]
pub(super) struct SessionAuthority {
    ticket: Ticket,
    connection: Option<u64>,
    session: Option<SessionId>,
    registration: Option<u64>,
}
impl SessionAuthority {
    pub(super) fn ticket(&self) -> Ticket {
        self.ticket
    }
    pub(super) fn connection(&self) -> Option<u64> {
        self.connection
    }
    pub(super) fn registration(&self) -> Option<u64> {
        self.registration
    }
    pub(super) fn session(&self) -> &Option<SessionId> {
        &self.session
    }
}
impl SessionDriver {
    pub(super) fn authority(&self) -> SessionAuthority {
        SessionAuthority {
            ticket: self.ticket(),
            connection: self.connection_key(),
            session: self.session().clone(),
            registration: self.registration().as_ref().map(|r| r.id),
        }
    }
    pub(super) fn accepts_authority(&self, authority: &SessionAuthority) -> bool {
        self.session_current(authority.ticket())
            && self.connection_key() == authority.connection()
            && self.session() == authority.session()
            && self.registration().as_ref().map(|r| r.id) == authority.registration()
    }
    pub(super) fn session(&self) -> &Option<SessionId> {
        self.lease.as_ref().map_or(&None, SessionLease::session)
    }
    pub(super) fn commands(&self) -> &Option<ManagedCommandsHandle> {
        self.lease.as_ref().map_or(&None, SessionLease::commands)
    }
    pub(super) fn registration(&self) -> &Option<BridgeRegistration> {
        self.lease
            .as_ref()
            .map_or(&None, SessionLease::registration)
    }
    pub(super) fn workdir(&self) -> &Option<PathBuf> {
        self.lease.as_ref().map_or(&None, SessionLease::workdir)
    }
    pub(super) fn connection_key(&self) -> Option<u64> {
        self.lease.as_ref().map(SessionLease::connection_key)
    }
    pub(super) fn leased(&self) -> bool {
        self.lease.is_some()
    }
    pub(super) fn shown(&self) -> bool {
        self.shown
    }
    pub(super) fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }
    pub(super) fn hold_operation(&self) -> OperationGuard {
        self.operations.hold()
    }
    pub(super) fn busy(&self) -> bool {
        self.phase.busy() || self.operations.active()
    }
    pub(super) fn accept_lease(&mut self, lease: SessionLease) {
        self.lease = Some(lease);
        self.phase.leased();
    }
    pub(super) fn release_lease(&mut self) -> bool {
        self.epoch = None;
        self.lease.take().is_some()
    }
    pub(super) fn restore(&self) -> &Option<Restore> {
        &self.restore
    }
    pub(super) fn restore_descriptor(&self) -> Option<Restore> {
        self.session()
            .as_ref()
            .and_then(|session| {
                self.workdir().as_ref().map(|workdir| Restore {
                    session: session.clone(),
                    workdir: workdir.clone(),
                    fork: false,
                })
            })
            .or_else(|| self.restore.clone())
    }
    pub(super) fn import_restore(&mut self, restore: Option<Restore>) {
        self.restore = restore;
    }
    pub(super) fn discard_broken_restore(&mut self) {
        if !self.phase.may_resume() {
            self.restore = None;
        }
    }
    pub(super) fn epoch(&self) -> Option<&LiveEpoch> {
        self.epoch.as_ref()
    }
    pub(super) fn begin_open(&mut self, commands: ManagedCommandsHandle, workdir: PathBuf) -> bool {
        if !self.phase.opening() {
            return false;
        }
        let Some(lease) = &mut self.lease else {
            return false;
        };
        lease.begin_open(commands, workdir);
        self.epoch = None;
        true
    }
    pub(super) fn retain_failed_restore(
        &mut self,
        ticket: Ticket,
        restore: Option<Restore>,
    ) -> bool {
        if !self.phase.session_current(ticket) {
            return false;
        }
        self.restore = restore;
        true
    }
    pub(super) fn accept_open(
        &mut self,
        ticket: Ticket,
        session: SessionId,
        fresh_workdir: Option<PathBuf>,
    ) -> bool {
        if !self.phase.session_current(ticket) {
            return false;
        }
        let Some(lease) = &mut self.lease else {
            return false;
        };
        self.epoch = Some(LiveEpoch::new(
            ticket,
            lease.connection_key(),
            session.clone(),
        ));
        lease.accept_session(session, fresh_workdir);
        self.restore = None;
        true
    }
    pub(super) fn phase(&self) -> SessionPhase {
        self.phase.phase()
    }
    pub(super) fn ticket(&self) -> Ticket {
        self.phase.ticket()
    }
    pub(super) fn turn(&self) -> u64 {
        self.phase.turn()
    }
    pub(super) fn accepts_updates(&self) -> bool {
        self.phase.accepts_updates()
    }
    pub(super) fn failed(&self) -> bool {
        self.phase.failed()
    }
    pub(super) fn stopped(&self) -> bool {
        self.phase.stopped()
    }
    pub(super) fn queued(&self) -> bool {
        self.phase.queued()
    }
    pub(super) fn opening(&self) -> bool {
        self.phase.opening()
    }
    pub(super) fn sign_in_required(&self) -> bool {
        self.phase.sign_in_required()
    }
    pub(super) fn signing_in(&self) -> bool {
        self.phase.signing_in()
    }
    pub(super) fn closing(&self) -> bool {
        self.phase.closing()
    }
    pub(super) fn generating(&self) -> bool {
        self.phase.generating()
    }
    pub(super) fn session_current(&self, ticket: Ticket) -> bool {
        self.phase.session_current(ticket)
    }
    pub(super) fn turn_current(&self, ticket: Ticket) -> bool {
        self.phase.turn_current(ticket)
    }
    pub(super) fn queue_activation(&mut self) -> bool {
        self.phase.queue()
    }
    pub(super) fn withdraw_if_idle(&mut self) -> bool {
        self.phase.withdraw()
    }
    pub(super) fn begin_turn(&mut self) -> Option<Ticket> {
        self.phase.start_turn()
    }
    pub(super) fn begin_cancel(&mut self) -> bool {
        self.phase.stop()
    }
    pub(super) fn opened(&mut self) {
        self.phase.opened()
    }
    pub(super) fn detach(&mut self) {
        self.phase.detach()
    }
    pub(super) fn fail(&mut self) {
        self.phase.fail()
    }
    pub(super) fn require_sign_in(&mut self) {
        self.phase.require_sign_in()
    }
    pub(super) fn release(&mut self) {
        self.phase.release()
    }
    pub(super) fn connection_closed(&mut self) {
        self.phase.closed()
    }
    pub(super) fn begin_sign_in(&mut self) -> bool {
        self.phase.begin_sign_in()
    }
    pub(super) fn finish_turn(&mut self, ticket: Ticket, completed: bool) -> bool {
        self.phase.finish_turn(ticket, completed)
    }
    pub(super) fn finish_sign_in(&mut self, succeeded: bool, has_session: bool) {
        self.phase.sign_in_finished(succeeded, has_session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_work_keeps_its_original_counter_across_release() {
        let mut session = SessionDriver::default();
        let old = session.authority();
        let first = session.hold_operation();
        let second = session.hold_operation();
        session.detach();
        session.release();
        assert!(session.busy());
        assert!(!session.accepts_authority(&old));
        drop(first);
        assert!(session.busy());
        drop(second);
        assert!(!session.busy());
    }
    #[test]
    fn an_unadmitted_hidden_request_withdraws_without_acquiring_a_lease() {
        let mut session = SessionDriver::default();
        session.set_shown(true);
        assert!(session.queue_activation());
        assert!(session.shown());
        assert!(!session.leased());
        session.set_shown(false);
        assert!(session.withdraw_if_idle());
        assert_eq!(session.phase(), SessionPhase::Detached);
        assert!(!session.shown());
        assert!(!session.withdraw_if_idle());
    }
}
