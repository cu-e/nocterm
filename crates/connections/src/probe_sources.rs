//! Weak original-source receipts, rotating cursors and bounded capacity retry.
use crate::Connections;
use gpui_kit::{App, Entity, EntityId, WeakEntity};
use nocterm_connections_model::profiles::ProfileId;
use nocterm_connections_store::{
    FactsAdmissionError, FactsService, ProbeOutcome, ProbeRequest, ProbeSourceAuthority,
    ProbeSourceIdentity, ProbeSourceLease,
};
use nocterm_session::{RemoteFs, Target};
use nocterm_workspace::{ConnectionFactSession, ExpectedSession, Workspace};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{Arc, Weak},
};
struct OriginalSource {
    original: Rc<dyn ConnectionFactSession>,
    expected: ExpectedSession,
    fs: Weak<dyn RemoteFs>,
    target: Target,
    association: gpui_kit::SharedString,
}
impl ProbeSourceAuthority for OriginalSource {
    fn same_session_alive(&self, cx: &App) -> bool {
        self.original.current(cx).is_some_and(|snapshot| {
            snapshot.expected == self.expected
                && snapshot.session.connected
                && snapshot.session.fs.as_ref().is_some_and(|fs| {
                    self.fs
                        .upgrade()
                        .is_some_and(|original| Arc::ptr_eq(&original, fs))
                })
        })
    }
    fn is_current(&self, cx: &App) -> bool {
        let Some(snapshot) = self.original.current(cx) else {
            return false;
        };
        snapshot.expected == self.expected
            && snapshot
                .profile
                .as_ref()
                .is_some_and(|id| *id == self.association)
            && snapshot.session.connected
            && snapshot.session.target == self.target
            && snapshot.session.fs.as_ref().is_some_and(|fs| {
                self.fs
                    .upgrade()
                    .is_some_and(|original| Arc::ptr_eq(&original, fs))
            })
    }
}
struct SourceSlot {
    expected: ExpectedSession,
    profile: ProfileId,
    target: Target,
    fs: Weak<dyn RemoteFs>,
    lease: ProbeSourceLease,
}
struct ProbeSource {
    workspace: WeakEntity<Workspace>,
    cursor: usize,
    leases: HashMap<EntityId, SourceSlot>,
}
/// One foreground projection, never retained as a pending native request.
struct ProjectedSource {
    association: gpui_kit::SharedString,
    original: Rc<dyn ConnectionFactSession>,
    expected: ExpectedSession,
    profile: ProfileId,
    target: Target,
    fs: Arc<dyn RemoteFs>,
}
impl ProjectedSource {
    fn read(
        item: &dyn nocterm_workspace::ItemHandle,
        connections: &Entity<Connections>,
        cx: &App,
    ) -> Option<Self> {
        let capabilities = item.terminal_capabilities(cx)?;
        let original = capabilities.fact_session()?.clone();
        let snapshot = original.current(cx)?;
        if snapshot.expected.item() != item.item_id() || !snapshot.session.connected {
            return None;
        }
        let association = snapshot.profile?;
        let profile = connections
            .read(cx)
            .profiles(cx)
            .iter()
            .find(|row| row.id.to_string() == *association)?
            .id;
        Some(Self {
            association,
            original,
            expected: snapshot.expected,
            profile,
            target: snapshot.session.target,
            fs: snapshot.session.fs?,
        })
    }
    fn fresh_slot(&self) -> SourceSlot {
        let authority = OriginalSource {
            original: self.original.clone(),
            expected: self.expected,
            fs: Arc::downgrade(&self.fs),
            target: self.target.clone(),
            association: self.association.clone(),
        };
        SourceSlot {
            expected: self.expected,
            profile: self.profile,
            target: self.target.clone(),
            fs: Arc::downgrade(&self.fs),
            lease: ProbeSourceLease::new(
                ProbeSourceIdentity::new(self.expected.item(), self.expected.epoch()),
                Rc::new(authority),
            ),
        }
    }
    fn request(self, lease: ProbeSourceLease) -> ProbeRequest {
        ProbeRequest::new(self.profile, self.target, self.fs, lease)
    }
}
impl SourceSlot {
    fn matches(&self, projection: &ProjectedSource, cx: &App) -> bool {
        self.expected == projection.expected
            && self.profile == projection.profile
            && self.target == projection.target
            && self.lease.is_current(cx)
            && self
                .fs
                .upgrade()
                .is_some_and(|original| Arc::ptr_eq(&original, &projection.fs))
    }
}
impl ProbeSource {
    fn reproject(&mut self, connections: &Entity<Connections>, cx: &App) -> Vec<ProbeRequest> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let projected: Vec<_> = workspace
            .read(cx)
            .items()
            .filter_map(|item| ProjectedSource::read(item, connections, cx))
            .collect();
        let present: HashSet<_> = projected.iter().map(|row| row.expected.item()).collect();
        self.leases.retain(|id, slot| {
            let present = present.contains(id);
            if !present && !slot.lease.is_current(cx) {
                slot.lease.revoke();
            }
            present
        });
        projected
            .into_iter()
            .filter_map(|projection| {
                let id = projection.expected.item();
                if self
                    .leases
                    .get(&id)
                    .is_none_or(|slot| !slot.matches(&projection, cx))
                {
                    if let Some(old) = self.leases.remove(&id)
                        && !old.lease.is_current(cx)
                    {
                        old.lease.revoke();
                    }
                    self.leases.insert(id, projection.fresh_slot());
                }
                self.leases
                    .get(&id)
                    .map(|slot| projection.request(slot.lease.clone()))
            })
            .collect()
    }
}
#[derive(Default)]
pub(crate) struct ProbeSources {
    sources: RefCell<Vec<ProbeSource>>,
    rotation: Cell<usize>,
}
impl ProbeSources {
    pub(crate) fn register(&self, workspace: WeakEntity<Workspace>) {
        let mut sources = self.sources.borrow_mut();
        if !sources
            .iter()
            .any(|source| source.workspace.entity_id() == workspace.entity_id())
        {
            sources.push(ProbeSource {
                workspace,
                cursor: 0,
                leases: HashMap::new(),
            });
        }
    }
    pub(crate) fn scan(
        &self,
        connections: &Entity<Connections>,
        facts: &Entity<FactsService>,
        cx: &mut App,
    ) {
        let mut sources = self.sources.borrow_mut();
        sources.retain(|source| source.workspace.upgrade().is_some());
        let count = sources.len();
        let start = self.rotation.get();
        for offset in 0..count {
            let index = (start + offset) % count;
            let source = &mut sources[index];
            let requests = source.reproject(connections, cx);
            let length = requests.len();
            if length == 0 {
                source.cursor = 0;
                continue;
            }
            let first = source.cursor % length;
            let mut indexed: Vec<_> = requests.into_iter().enumerate().collect();
            indexed.rotate_left(first);
            for (position, request) in indexed {
                match facts.update(cx, |facts, cx| facts.request_probe(request, cx)) {
                    Err(FactsAdmissionError::Capacity { .. }) => {
                        source.cursor = position;
                        self.rotation.set(index);
                        return;
                    }
                    Err(FactsAdmissionError::Closing) => return,
                    Ok(ProbeOutcome::Admitted { .. }) => {
                        source.cursor = (position + 1) % length;
                        self.rotation.set((index + 1) % count);
                        break;
                    }
                    Ok(ProbeOutcome::Settled)
                    | Err(
                        FactsAdmissionError::MissingProfile
                        | FactsAdmissionError::EndpointMismatch
                        | FactsAdmissionError::StaleSource,
                    ) => source.cursor = (position + 1) % length,
                }
            }
        }
        facts.update(cx, |facts, cx| facts.yield_probe_turn(cx));
    }
}

#[cfg(test)]
mod tests;
