//! ACP provider permissions are independent of Nocterm terminal grants.
use futures::channel::oneshot;
use gpui_kit::Context;
use nocterm_ai::acp;
use nocterm_settings::ApprovalPolicy;
use nocterm_ui::{ActiveAi as _, ActiveSettings as _};

use super::{AgentThread, PendingPermission};

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

impl AgentThread {
    fn accepts_permission(
        &self,
        request: &acp::RequestPermissionRequest,
        cx: &Context<Self>,
    ) -> bool {
        cx.ai_enabled()
            && self.accept_updates
            && !self.stopped
            && self.session.as_ref() == Some(&request.session_id)
    }

    pub(crate) fn permission(
        &mut self,
        request: acp::RequestPermissionRequest,
        respond: oneshot::Sender<acp::RequestPermissionOutcome>,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_permission(&request, cx) {
            let _ = respond.send(acp::RequestPermissionOutcome::Cancelled);
            return;
        }
        let automatic = cx.settings().ai.approval.agent_permissions == ApprovalPolicy::Allow;
        if automatic && let Some(id) = allow_once(&request) {
            let _ = respond.send(selected(id));
            return;
        }
        self.approval_generation = self.approval_generation.wrapping_add(1);
        self.permissions.push(PendingPermission {
            request,
            respond,
            generation: self.approval_generation,
            explanation: automatic.then_some(ONE_TIME_REQUIRED),
        });
        cx.notify();
    }

    /// Applies changes to queued requests without granting persistent provider permissions.
    pub(crate) fn apply_permission_policy(&mut self, cx: &mut Context<Self>) {
        let automatic = cx.settings().ai.approval.agent_permissions == ApprovalPolicy::Allow;
        let mut changed = false;
        for mut permission in std::mem::take(&mut self.permissions) {
            if !self.accepts_permission(&permission.request, cx) {
                let _ = permission
                    .respond
                    .send(acp::RequestPermissionOutcome::Cancelled);
                changed = true;
            } else if automatic && let Some(id) = allow_once(&permission.request) {
                let _ = permission.respond.send(selected(id));
                changed = true;
            } else {
                let explanation = automatic.then_some(ONE_TIME_REQUIRED);
                changed |= permission.explanation != explanation;
                permission.explanation = explanation;
                self.permissions.push(permission);
            }
        }
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn choose_permission(
        &mut self,
        index: usize,
        option: Option<acp::PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        if index >= self.permissions.len() {
            return;
        }
        let permission = self.permissions.remove(index);
        let outcome = option
            .filter(|id| {
                self.accepts_permission(&permission.request, cx)
                    && unique_option(&permission.request, id)
            })
            .map(selected)
            .unwrap_or(acp::RequestPermissionOutcome::Cancelled);
        let _ = permission.respond.send(outcome);
        cx.notify();
    }

    /// A rendered card can outlive a queue drain or a newly arrived request.
    pub(crate) fn choose_permission_request(
        &mut self,
        generation: u64,
        option: Option<acp::PermissionOptionId>,
        cx: &mut Context<Self>,
    ) {
        if let Some(index) = self
            .permissions
            .iter()
            .position(|request| request.generation == generation)
        {
            self.choose_permission(index, option, cx);
        }
    }
}

#[cfg(test)]
mod tests;
