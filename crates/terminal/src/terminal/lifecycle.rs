//! Session lifetime, prompt authority and command completion.
use super::{CommandCompletion, Status};
use gpui_kit::Task;
use nocterm_session::{
    CloseReason, ConnectStage, HostKeyDecision, Prompt, RemoteFs, Secret, Session, Transport,
};
use nocterm_workspace::{SessionContext, SessionSpec};
use std::sync::Arc;

pub(super) struct SessionLifecycle {
    spec: SessionSpec,
    local: bool,
    transport: Option<Arc<dyn Transport>>,
    completion: Option<CommandCompletion>,
    session: Option<Session>,
    fs: Option<Arc<dyn RemoteFs>>,
    status: Status,
    prompt: Option<Prompt>,
    prompt_epoch: u64,
    connection_epoch: u64,
    pump: Option<Task<()>>,
}
impl SessionLifecycle {
    pub(super) fn new(
        spec: SessionSpec,
        local: bool,
        transport: Option<Arc<dyn Transport>>,
        completion: Option<CommandCompletion>,
    ) -> Self {
        Self {
            spec,
            local,
            transport,
            completion,
            session: None,
            fs: None,
            status: Status::Connecting(ConnectStage::Connecting { hop: None }),
            prompt: None,
            prompt_epoch: 0,
            connection_epoch: 0,
            pump: None,
        }
    }
    pub(super) fn spec(&self) -> &SessionSpec {
        &self.spec
    }
    pub(super) fn set_credential(&mut self, id: nocterm_session::CredentialId) {
        self.spec.credential = Some(id);
    }
    pub(super) fn apply_options(
        &mut self,
        options: nocterm_session::SessionOptions,
    ) -> Result<(), nocterm_session::SessionOptionsError> {
        options.validate()?;
        self.spec.options = options;
        Ok(())
    }
    pub(super) fn status(&self) -> &Status {
        &self.status
    }
    pub(super) fn prompt(&self) -> Option<&Prompt> {
        self.prompt.as_ref()
    }
    pub(super) fn is_local(&self) -> bool {
        self.local
    }
    pub(super) fn is_command(&self) -> bool {
        self.transport.is_some()
    }
    pub(super) fn transport(&self) -> Option<Arc<dyn Transport>> {
        self.transport.clone()
    }
    pub(super) fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }
    pub(super) fn epoch(&self) -> u64 {
        self.connection_epoch
    }
    pub(super) fn prompt_epoch(&self) -> u64 {
        self.prompt_epoch
    }
    pub(super) fn is_current(&self, epoch: u64) -> bool {
        self.connection_epoch == epoch
    }
    pub(super) fn install_pump(&mut self, task: Task<()>) {
        self.pump = Some(task);
    }
    pub(super) fn accept_session(&mut self, session: Session) {
        self.fs = session.fs();
        self.session = Some(session);
        self.prompt = None;
    }
    pub(super) fn accept_prompt(&mut self, prompt: Prompt) {
        self.prompt_epoch = self.prompt_epoch.wrapping_add(1);
        self.prompt = Some(prompt);
    }
    pub(super) fn finish(&mut self) {
        self.prompt = None;
        self.fs = None;
    }
    pub(super) fn retire_session(&mut self) -> Option<Session> {
        self.fs = None;
        self.prompt = None;
        self.prompt_epoch = self.prompt_epoch.wrapping_add(1);
        self.session.take()
    }
    pub(super) fn close(&mut self) -> Option<Session> {
        self.connection_epoch = self.connection_epoch.wrapping_add(1);
        self.pump = None;
        self.retire_session()
    }
    pub(super) fn set_status(
        &mut self,
        status: Status,
    ) -> (bool, Option<(CommandCompletion, CloseReason)>) {
        let completion = if let Status::Closed(reason) = &status {
            self.completion
                .take()
                .map(|callback| (callback, reason.clone()))
        } else {
            None
        };
        let changed = self.status != status;
        self.status = status;
        (changed, completion)
    }
    pub(super) fn answer_host_key(&mut self, decision: HostKeyDecision) {
        match self.prompt.take() {
            Some(Prompt::UnknownHostKey { reply, .. } | Prompt::ChangedHostKey { reply, .. }) => {
                reply.send(decision)
            }
            other => self.prompt = other,
        }
    }
    pub(super) fn answer_secret(&mut self, secret: Option<Secret>) {
        match self.prompt.take() {
            Some(Prompt::Secret { reply, .. }) => reply.send(secret),
            other => self.prompt = other,
        }
    }
    pub(super) fn context(&self) -> SessionContext {
        let target = self.spec.target.clone();
        let via = self.spec.address().via;
        let exec = self.session.as_ref().and_then(Session::exec);
        if !self.is_command() {
            return SessionContext::new(target, self.fs.clone(), self.status == Status::Connected)
                .with_via(via)
                .with_exec(exec);
        }
        let lost = matches!(self.status, Status::Closed(CloseReason::Failed(_)));
        SessionContext::new(target, self.session.as_ref().and_then(Session::fs), !lost)
            .with_via(via)
            .with_exec(exec)
    }
}
