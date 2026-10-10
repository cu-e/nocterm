//! ACP provider permissions are independent of Nocterm terminal grants.
use super::approval_inbox::ApprovalToken;
use nocterm_ai::acp;

const ONE_TIME_REQUIRED: &str = "Automatic approval needs a valid Allow once option. This agent did not provide one. Review its choices or cancel this request.";

fn unique_option(request: &acp::RequestPermissionRequest, id: &acp::PermissionOptionId) -> bool {
    !id.0.trim().is_empty()
        && request
            .options
            .iter()
            .filter(|option| option.option_id == *id)
            .count()
            == 1
}

fn allow_once(request: &acp::RequestPermissionRequest) -> Option<acp::PermissionOptionId> {
    request
        .options
        .iter()
        .find(|option| {
            option.kind == acp::PermissionOptionKind::AllowOnce
                && unique_option(request, &option.option_id)
        })
        .map(|option| option.option_id.clone())
}

fn selected(id: acp::PermissionOptionId) -> acp::RequestPermissionOutcome {
    acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(id))
}

/// Fresh provider authority, sampled by the coordinator before every transition.
pub(super) struct PermissionGate {
    enabled: bool,
    accepts_updates: bool,
    stopped: bool,
    session: Option<acp::SessionId>,
}
impl PermissionGate {
    pub(super) fn new(
        enabled: bool,
        accepts_updates: bool,
        stopped: bool,
        session: Option<acp::SessionId>,
    ) -> Self {
        Self {
            enabled,
            accepts_updates,
            stopped,
            session,
        }
    }
    fn accepts(&self, request: &acp::RequestPermissionRequest) -> bool {
        self.enabled
            && self.accepts_updates
            && !self.stopped
            && self.session.as_ref() == Some(&request.session_id)
    }
}

struct PendingPermission {
    request: acp::RequestPermissionRequest,
    respond: nocterm_ai::PermissionResponder,
    token: ApprovalToken,
    explanation: Option<&'static str>,
}

/// An immutable card; responders and mutable queue state never leave their owner.
pub(crate) struct PermissionCard {
    request: acp::RequestPermissionRequest,
    token: ApprovalToken,
    explanation: Option<&'static str>,
}
impl PermissionCard {
    pub(crate) fn token(&self) -> ApprovalToken {
        self.token
    }
    pub(crate) fn request(&self) -> &acp::RequestPermissionRequest {
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
        request: acp::RequestPermissionRequest,
        respond: nocterm_ai::PermissionResponder,
        token: ApprovalToken,
        gate: &PermissionGate,
        automatic: bool,
    ) -> bool {
        if !gate.accepts(&request) {
            respond.respond(acp::RequestPermissionOutcome::Cancelled);
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
                permission
                    .respond
                    .respond(acp::RequestPermissionOutcome::Cancelled);
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
        option: Option<acp::PermissionOptionId>,
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
            .unwrap_or(acp::RequestPermissionOutcome::Cancelled);
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
