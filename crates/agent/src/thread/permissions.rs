//! ACP provider permissions are independent of Nocterm terminal grants.
use super::approval_inbox::ApprovalToken;
use nocterm_ai::model::{
    auth::{PermissionEffect, PermissionOutcome, ProviderPermission},
    ids::{PermissionOptionId, SessionId},
};

const ONE_TIME_REQUIRED: &str = "Automatic approval needs a valid Allow once option. This agent did not provide one. Review its choices or cancel this request.";

fn unique_option(request: &ProviderPermission, id: &PermissionOptionId) -> bool {
    !id.as_str().trim().is_empty()
        && request
            .choices
            .iter()
            .filter(|option| option.id == *id)
            .count()
            == 1
}

fn allow_once(request: &ProviderPermission) -> Option<PermissionOptionId> {
    request
        .choices
        .iter()
        .find(|option| {
            option.effect == PermissionEffect::AllowOnce && unique_option(request, &option.id)
        })
        .map(|option| option.id.clone())
}

fn selected(id: PermissionOptionId) -> PermissionOutcome {
    PermissionOutcome::Selected(id)
}

/// Fresh provider authority, sampled by the coordinator before every transition.
pub(super) struct PermissionGate {
    enabled: bool,
    accepts_updates: bool,
    stopped: bool,
    session: Option<SessionId>,
}
impl PermissionGate {
    pub(super) fn new(
        enabled: bool,
        accepts_updates: bool,
        stopped: bool,
        session: Option<SessionId>,
    ) -> Self {
        Self {
            enabled,
            accepts_updates,
            stopped,
            session,
        }
    }
    fn accepts(&self, request: &ProviderPermission) -> bool {
        self.enabled
            && self.accepts_updates
            && !self.stopped
            && self.session.as_ref() == Some(&request.session)
    }
}

struct PendingPermission {
    request: ProviderPermission,
    respond: nocterm_ai::PermissionResponder,
    token: ApprovalToken,
    explanation: Option<&'static str>,
}

/// An immutable card; responders and mutable queue state never leave their owner.
pub(crate) struct PermissionCard {
    request: ProviderPermission,
    token: ApprovalToken,
    explanation: Option<&'static str>,
}
impl PermissionCard {
    pub(crate) fn token(&self) -> ApprovalToken {
        self.token
    }
    pub(crate) fn request(&self) -> &ProviderPermission {
        &self.request
    }
    pub(crate) fn explanation(&self) -> Option<&'static str> {
        self.explanation
    }
}

#[derive(Default)]
pub(super) struct ProviderRequests {
    pending: Vec<PendingPermission>,
}
impl ProviderRequests {
    pub(super) fn receive(
        &mut self,
        request: ProviderPermission,
        respond: nocterm_ai::PermissionResponder,
        token: ApprovalToken,
        gate: &PermissionGate,
        automatic: bool,
    ) -> bool {
        if !gate.accepts(&request) {
            respond.respond(PermissionOutcome::Cancelled);
            return false;
        }
        if automatic && let Some(id) = allow_once(&request) {
            respond.respond(selected(id));
            return false;
        }
        self.pending.push(PendingPermission {
            request,
            respond,
            token,
            explanation: automatic.then_some(ONE_TIME_REQUIRED),
        });
        true
    }
    pub(super) fn apply_policy(&mut self, gate: &PermissionGate, automatic: bool) -> bool {
        let mut changed = false;
        for mut permission in std::mem::take(&mut self.pending) {
            if !gate.accepts(&permission.request) {
                permission.respond.respond(PermissionOutcome::Cancelled);
                changed = true;
            } else if automatic && let Some(id) = allow_once(&permission.request) {
                permission.respond.respond(selected(id));
                changed = true;
            } else {
                let explanation = automatic.then_some(ONE_TIME_REQUIRED);
                changed |= permission.explanation != explanation;
                permission.explanation = explanation;
                self.pending.push(permission);
            }
        }
        changed
    }
    pub(super) fn choose(
        &mut self,
        token: ApprovalToken,
        option: Option<PermissionOptionId>,
        gate: &PermissionGate,
    ) -> bool {
        let Some(index) = self
            .pending
            .iter()
            .position(|request| request.token == token)
        else {
            return false;
        };
        let permission = self.pending.remove(index);
        let outcome = option
            .filter(|id| {
                gate.accepts(&permission.request) && unique_option(&permission.request, id)
            })
            .map(selected)
            .unwrap_or(PermissionOutcome::Cancelled);
        permission.respond.respond(outcome);
        true
    }
    pub(super) fn clear(&mut self) {
        self.pending.clear();
    }
    pub(super) fn len(&self) -> usize {
        self.pending.len()
    }
    pub(super) fn cards(&self) -> Vec<PermissionCard> {
        self.pending
            .iter()
            .map(|permission| PermissionCard {
                request: permission.request.clone(),
                token: permission.token,
                explanation: permission.explanation,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
