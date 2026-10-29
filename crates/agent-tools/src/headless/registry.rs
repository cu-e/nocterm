//! Run-local handles retain the exact receipt and native session that were attached.
use super::{
    HeadlessError,
    physical::{PhysicalPublication, PhysicalTerminal},
    retained::RetainedOwner,
    view::ViewFence,
};
use futures::future::BoxFuture;
use nocterm_cloud_model::{
    authority::{RunBinding, SessionReceipt},
    identity::RemoteSessionId,
    tool::{ScopedToolRequest, ToolTimer},
};
use nocterm_session::{Charset, Prompt, PtySize, Session, SessionError};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError, Weak},
};

const MAX_SESSIONS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct HeadlessTerminalOptions {
    pub size: PtySize,
    pub charset: Charset,
}
impl HeadlessTerminalOptions {
    pub(super) fn validate(self) -> Result<(), HeadlessError> {
        if !(2..=240).contains(&self.size.cols) || !(1..=120).contains(&self.size.rows) {
            return Err(HeadlessError::InvalidGeometry);
        }
        Ok(())
    }
}
impl Default for HeadlessTerminalOptions {
    fn default() -> Self {
        Self {
            size: PtySize::default(),
            charset: Charset::Utf8,
        }
    }
}

/// A bounded session driver that native composition must schedule and retain.
pub struct HeadlessSessionTask {
    terminal: String,
    run: BoxFuture<'static, ()>,
    original: Arc<Mutex<PhysicalTerminal>>,
}
impl HeadlessSessionTask {
    pub(super) fn original(&self) -> Arc<Mutex<PhysicalTerminal>> {
        self.original.clone()
    }
    pub fn terminal(&self) -> &str {
        &self.terminal
    }
    pub fn into_run(self) -> BoxFuture<'static, ()> {
        self.run
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadlessSessionStatus {
    Connecting,
    Connected,
    NeedsUnlock,
    Closed,
}

#[derive(Clone, Debug)]
pub struct HeadlessSessionInfo {
    pub terminal_id: String,
    pub receipt: SessionReceipt,
    pub status: HeadlessSessionStatus,
    pub execution: bool,
    pub files: bool,
    failure: Option<SessionError>,
}

impl HeadlessSessionInfo {
    pub(super) fn new(
        terminal_id: String,
        receipt: SessionReceipt,
        status: HeadlessSessionStatus,
        execution: bool,
        files: bool,
        failure: Option<SessionError>,
    ) -> Self {
        Self {
            terminal_id,
            receipt,
            status,
            execution,
            files,
            failure,
        }
    }

    pub fn failure(&self) -> Option<&SessionError> {
        self.failure.as_ref()
    }
}

/// One run's injected native sessions; a receipt constructor does not authenticate them.
/// The caller supplies fresh opaque handles and schedules every returned event task.
pub struct SessionRegistry {
    run: RunBinding,
    entries: BTreeMap<String, Arc<Mutex<PhysicalTerminal>>>,
    fence: Arc<ViewFence>,
    retained: Option<Weak<RetainedOwner>>,
}
impl SessionRegistry {
    pub fn new(run: RunBinding) -> Self {
        Self {
            run,
            entries: BTreeMap::new(),
            fence: Arc::new(ViewFence::default()),
            retained: None,
        }
    }
    pub(super) fn retained(run: RunBinding, owner: Weak<RetainedOwner>) -> Self {
        Self {
            run,
            entries: BTreeMap::new(),
            fence: Arc::new(ViewFence::default()),
            retained: Some(owner),
        }
    }
    /// Attach a fresh run-local handle to an already retained exact physical receipt.
    /// This neither opens SSH nor creates an event reader; new tools still need grants.
    pub fn attach_retained(
        &mut self,
        handle: RemoteSessionId,
        receipt: &SessionReceipt,
    ) -> Result<String, HeadlessError> {
        let terminal = terminal_handle(handle);
        self.require_fresh_handle(&terminal)?;
        if self.entries.len() >= MAX_SESSIONS {
            return Err(HeadlessError::Capacity);
        }
        let owner = self
            .retained
            .as_ref()
            .and_then(Weak::upgrade)
            .ok_or(HeadlessError::Closed)?;
        let entry = owner.reattach(receipt)?;
        self.entries.insert(terminal.clone(), entry);
        Ok(terminal)
    }
    pub(super) fn view_fence(&self) -> Arc<ViewFence> {
        self.fence.clone()
    }
    pub(super) fn retained_owner(&self) -> Option<Arc<RetainedOwner>> {
        self.retained.as_ref().and_then(Weak::upgrade)
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn attach(
        &mut self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        session: Session,
        options: HeadlessTerminalOptions,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<HeadlessSessionTask, HeadlessError> {
        self.attach_physical(
            handle,
            receipt,
            session,
            options,
            timer,
            PhysicalPublication::Published,
        )
    }
    pub(super) fn attach_pending(
        &mut self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        session: Session,
        options: HeadlessTerminalOptions,
        timer: Arc<dyn ToolTimer>,
    ) -> Result<HeadlessSessionTask, HeadlessError> {
        self.attach_physical(
            handle,
            receipt,
            session,
            options,
            timer,
            PhysicalPublication::Pending,
        )
    }
    pub(super) fn attach_initial(
        &mut self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        session: Session,
        options: HeadlessTerminalOptions,
        timer: Arc<dyn ToolTimer>,
        lifecycle: super::initial::InitialLifecycle,
    ) -> Result<HeadlessSessionTask, HeadlessError> {
        if self.retained_owner().is_none() {
            session.close();
            return Err(HeadlessError::Closed);
        }
        self.attach_physical(
            handle,
            receipt,
            session,
            options,
            timer,
            PhysicalPublication::Initial(lifecycle),
        )
    }
    pub(super) fn publish_initial(
        &self,
        terminal: &str,
        original: &Arc<Mutex<PhysicalTerminal>>,
    ) -> Result<(), HeadlessError> {
        if self.fence.is_closed() {
            return Err(HeadlessError::Closed);
        }
        let current = self
            .entries
            .get(terminal)
            .ok_or(HeadlessError::StaleSession)?;
        if !Arc::ptr_eq(current, original) {
            return Err(HeadlessError::StaleSession);
        }
        self.retained_owner()
            .ok_or(HeadlessError::Closed)?
            .publish_initial(original)
    }
    fn attach_physical(
        &mut self,
        handle: RemoteSessionId,
        receipt: SessionReceipt,
        session: Session,
        options: HeadlessTerminalOptions,
        timer: Arc<dyn ToolTimer>,
        publication: PhysicalPublication,
    ) -> Result<HeadlessSessionTask, HeadlessError> {
        if let Err(error) = options.validate() {
            session.close();
            return Err(error);
        }
        let terminal = terminal_handle(handle);
        if self.fence.is_closed() {
            session.close();
            return Err(HeadlessError::Closed);
        }
        if self.entries.contains_key(&terminal) {
            session.close();
            return Err(HeadlessError::IdentityConflict);
        }
        if self.entries.len() >= MAX_SESSIONS {
            session.close();
            return Err(HeadlessError::Capacity);
        }
        let (retirement, retired) = async_channel::bounded(1);
        let entry = Arc::new(Mutex::new(PhysicalTerminal::new(
            receipt,
            session.clone(),
            options,
            retirement,
            publication,
        )));
        if let Some(owner) = &self.retained {
            let Some(owner) = owner.upgrade() else {
                session.close();
                return Err(HeadlessError::Closed);
            };
            if let Err(error) = owner.insert(entry.clone()) {
                session.close();
                return Err(error);
            }
        }
        self.entries.insert(terminal.clone(), entry.clone());
        let original = entry.clone();
        let run = super::session_watch::SessionWatch::new(entry, session, timer, retired).run();
        Ok(HeadlessSessionTask {
            terminal,
            run,
            original,
        })
    }
    pub fn list(&self, run: &RunBinding) -> Result<Vec<HeadlessSessionInfo>, HeadlessError> {
        self.require_run(run)?;
        Ok(self
            .entries
            .iter()
            .map(|(id, entry)| {
                let entry = entry.lock().unwrap_or_else(PoisonError::into_inner);
                entry.info(id.clone())
            })
            .collect())
    }
    pub fn take_prompt(
        &self,
        run: &RunBinding,
        terminal: &str,
        receipt: &SessionReceipt,
    ) -> Result<Option<Prompt>, HeadlessError> {
        self.require_run(run)?;
        let entry = self
            .entries
            .get(terminal)
            .ok_or(HeadlessError::StaleSession)?;
        let mut entry = entry.lock().unwrap_or_else(PoisonError::into_inner);
        entry.take_prompt(receipt)
    }
    pub fn remove(&mut self, terminal: &str) -> Result<(), HeadlessError> {
        if self.fence.is_closed() {
            return Err(HeadlessError::Closed);
        }
        self.remove_entry(terminal)
    }
    fn remove_entry(&mut self, terminal: &str) -> Result<(), HeadlessError> {
        let entry = self
            .entries
            .remove(terminal)
            .ok_or(HeadlessError::StaleSession)?;
        if let Some(owner) = self.retained_owner() {
            owner.remove_if_original(&entry);
        }
        entry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retire();
        Ok(())
    }
    pub(super) fn remove_if_original(
        &mut self,
        terminal: &str,
        original: &Arc<Mutex<PhysicalTerminal>>,
    ) {
        if self
            .entries
            .get(terminal)
            .is_some_and(|current| Arc::ptr_eq(current, original))
        {
            if let Some(owner) = self.retained_owner() {
                owner.discard_pending(original);
                self.entries.remove(terminal);
            } else if original
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire_pending()
            {
                self.entries.remove(terminal);
            }
            // A published unretained entry stays owned by its registry; losing
            // the open waiter must not orphan the reader after another tool
            // has already accepted the connected physical capability.
        }
    }
    pub fn close(&mut self) {
        self.fence.close();
        if self.retained.is_some() {
            return;
        }
        for entry in self.entries.values() {
            entry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retire();
        }
    }
    pub(super) fn require_fresh_handle(&self, terminal: &str) -> Result<(), HeadlessError> {
        if self.fence.is_closed() {
            return Err(HeadlessError::Closed);
        }
        if self.entries.contains_key(terminal) {
            return Err(HeadlessError::IdentityConflict);
        }
        Ok(())
    }
    pub(super) fn require_run(&self, run: &RunBinding) -> Result<(), HeadlessError> {
        if self.fence.is_closed() {
            return Err(HeadlessError::Closed);
        }
        if run.scope() != self.run.scope() {
            return Err(nocterm_cloud_model::authority::AuthorityError::WrongScope.into());
        }
        if run != &self.run {
            return Err(nocterm_cloud_model::authority::AuthorityError::StaleRun.into());
        }
        Ok(())
    }
    pub(super) fn get(
        &self,
        request: &ScopedToolRequest,
    ) -> Result<Arc<Mutex<PhysicalTerminal>>, HeadlessError> {
        self.require_run(request.permit().intent().authority().run())?;
        let id = request
            .call()
            .terminal_id()
            .ok_or(HeadlessError::StaleSession)?;
        let entry = self
            .entries
            .get(id)
            .ok_or(HeadlessError::StaleSession)?
            .clone();
        {
            let mut physical = entry.lock().unwrap_or_else(PoisonError::into_inner);
            physical.check(request.permit().intent().authority())?;
            physical.publish()?;
        }
        Ok(entry)
    }
}
impl Drop for SessionRegistry {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) fn terminal_handle(id: RemoteSessionId) -> String {
    let mut handle = String::from("t-");
    for byte in id.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(handle, "{byte:02x}");
    }
    handle
}
