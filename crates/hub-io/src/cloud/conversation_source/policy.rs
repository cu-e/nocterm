//! Explicit immutable product setup; no inferred provider, target or allowance.
use nocterm_cloud_model::{
    coordinator::RunDestination,
    engine::ModelBudget,
    identity::Fingerprint,
    inference::ModelSelection,
    projection::{ProjectionError, ports::ConversationPreparation},
};
use nocterm_hub_model::cloud::{CloudTextId, payload as wire};
use sha2::{Digest, Sha256};

pub struct ConversationPromptPolicy {
    selection: ModelSelection,
    /// `None` lets the runner apply its configured default destination.
    destination: Option<RunDestination>,
    budget: ModelBudget,
    spend: u64,
    lifetime_ms: u64,
    inference_lifetime_ms: u64,
    fingerprint: Fingerprint,
}
impl ConversationPromptPolicy {
    pub fn new(
        selection: ModelSelection,
        destination: RunDestination,
        budget: ModelBudget,
        spend: u64,
        lifetime_ms: u64,
        inference_lifetime_ms: u64,
    ) -> Result<Self, ProjectionError> {
        Self::build(
            selection,
            Some(destination),
            budget,
            spend,
            lifetime_ms,
            inference_lifetime_ms,
        )
    }
    /// Prompts name no target: the runner runs them at its configured default destination
    /// (for a managed instance, its workspace) and never resumes a session.
    pub fn runner_default(
        selection: ModelSelection,
        budget: ModelBudget,
        spend: u64,
        lifetime_ms: u64,
        inference_lifetime_ms: u64,
    ) -> Result<Self, ProjectionError> {
        Self::build(
            selection,
            None,
            budget,
            spend,
            lifetime_ms,
            inference_lifetime_ms,
        )
    }
    fn build(
        selection: ModelSelection,
        destination: Option<RunDestination>,
        budget: ModelBudget,
        spend: u64,
        lifetime_ms: u64,
        inference_lifetime_ms: u64,
    ) -> Result<Self, ProjectionError> {
        if spend == 0
            || inference_lifetime_ms == 0
            || inference_lifetime_ms > nocterm_hub_model::cloud::MAX_COMMAND_LIFETIME_MS as u64
            || lifetime_ms == 0
            || lifetime_ms > nocterm_hub_model::cloud::MAX_COMMAND_LIFETIME_MS as u64
        {
            return Err(ProjectionError::InvalidRecord);
        }
        let mut result = Self {
            selection,
            destination,
            budget,
            spend,
            lifetime_ms,
            inference_lifetime_ms,
            fingerprint: Fingerprint::from_bytes([0; 32]),
        };
        result.fingerprint = result.digest();
        Ok(result)
    }
    pub(super) fn prompt<'a>(
        &self,
        text: &'a str,
        preparation: ConversationPreparation,
    ) -> Result<wire::CloudCommandPayload<'a>, ProjectionError> {
        let selection = wire::ModelSelection {
            provider: id(self.selection.provider().as_str())?,
            model: id(self.selection.model().as_str())?,
            pricing_policy: id(self.selection.pricing().as_str())?,
        };
        let (access, existing_session) = match &self.destination {
            Some(destination) => (
                Some(target(destination.target(), destination.authentication())?),
                destination
                    .session()
                    .map(|value| {
                        Ok::<_, ProjectionError>(wire::SessionReference {
                            id: *value.id().as_bytes(),
                            epoch: value.epoch().get(),
                            access: target(value.target(), value.authentication())?,
                        })
                    })
                    .transpose()?,
            ),
            None => (None, None),
        };
        let inference = self.limits(
            i64::try_from(
                preparation
                    .issued()
                    .as_millis()
                    .checked_add(self.inference_lifetime_ms)
                    .ok_or(ProjectionError::InvalidRecord)?,
            )
            .map_err(|_| ProjectionError::InvalidRecord)?,
        );
        Ok(wire::CloudCommandPayload::SubmitPrompt(Box::new(
            wire::PromptPayload {
                content: vec![wire::PromptPart::Text(text)],
                model: selection,
                target: access,
                existing_session,
                inference,
            },
        )))
    }
    /// The run's model allowance: the prompt's own bounds, issued now for half the
    /// inference lifetime, so the person's permission stays inside the prompt's task
    /// window and what the chat's setup configured (ADR 0129).
    pub(super) fn inference_grant(
        &self,
        preparation: ConversationPreparation,
    ) -> Result<wire::CloudCommandPayload<'static>, ProjectionError> {
        // The runner requires the grant's window inside the prompt's task window, which
        // started earlier: half the lifetime leaves room for the grant to follow the prompt
        // by up to the other half.
        let deadline = i64::try_from(
            preparation
                .issued()
                .as_millis()
                .checked_add(self.inference_lifetime_ms / 2)
                .ok_or(ProjectionError::InvalidRecord)?,
        )
        .map_err(|_| ProjectionError::InvalidRecord)?;
        let issued = i64::try_from(preparation.issued().as_millis())
            .map_err(|_| ProjectionError::InvalidRecord)?;
        Ok(wire::CloudCommandPayload::IssueGrant(Box::new(
            wire::GrantPayload::Inference {
                grant: wire::GrantReference {
                    id: nocterm_keys::random_bytes().map_err(|_| ProjectionError::Unavailable)?,
                    epoch: 1,
                },
                model: wire::ModelSelection {
                    provider: id(self.selection.provider().as_str())?,
                    model: id(self.selection.model().as_str())?,
                    pricing_policy: id(self.selection.pricing().as_str())?,
                },
                window: wire::GrantWindow {
                    issued_at_ms: issued,
                    expires_at_ms: deadline,
                },
                limits: self.limits(deadline),
            },
        )))
    }
    fn limits(&self, deadline_ms: i64) -> wire::InferenceLimits {
        wire::InferenceLimits {
            calls: self.budget.calls(),
            turns: self.budget.turns(),
            request_bytes: self.budget.input_bytes() as u64,
            output_bytes: self.budget.output_bytes() as u64,
            output_tokens: self.budget.output_tokens(),
            spend_micros: self.spend,
            deadline_ms,
        }
    }
    fn digest_destination(hash: &mut Sha256, destination: &RunDestination) {
        let principal = destination.target().principal().as_str();
        hash.update((principal.len() as u64).to_be_bytes());
        hash.update(principal.as_bytes());
        for value in [
            destination.target().host(),
            destination.target().route(),
            destination.target().workspace(),
            destination.authentication().key(),
            destination.authentication().credential_revision(),
        ] {
            hash.update(value.as_bytes());
        }
        if let Some(session) = destination.session() {
            hash.update([1]);
            hash.update(session.id().as_bytes());
            hash.update(session.epoch().get().to_be_bytes());
        } else {
            hash.update([0]);
        }
    }
    fn digest(&self) -> Fingerprint {
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-conversation-policy-v1");
        for value in [
            self.selection.provider().as_str(),
            self.selection.model().as_str(),
            self.selection.pricing().as_str(),
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        match &self.destination {
            Some(destination) => {
                hash.update([1]);
                Self::digest_destination(&mut hash, destination);
            }
            None => hash.update([0]),
        }
        for value in [
            self.budget.calls() as u64,
            self.budget.turns() as u64,
            self.budget.input_bytes() as u64,
            self.budget.output_bytes() as u64,
            self.budget.output_tokens(),
            self.spend,
            self.lifetime_ms,
            self.inference_lifetime_ms,
        ] {
            hash.update(value.to_be_bytes());
        }
        Fingerprint::from_bytes(hash.finalize().into())
    }
}
fn id(value: &str) -> Result<CloudTextId, ProjectionError> {
    CloudTextId::new(value).map_err(|_| ProjectionError::InvalidRecord)
}
fn target(
    target: &nocterm_cloud_model::authority::TargetBinding,
    auth: &nocterm_cloud_model::authority::AuthenticationBinding,
) -> Result<wire::TargetAccess, ProjectionError> {
    Ok(wire::TargetAccess {
        target: wire::TargetSelection {
            host_key: *target.host().as_bytes(),
            route: *target.route().as_bytes(),
            principal: id(target.principal().as_str())?,
            workspace: *target.workspace().as_bytes(),
        },
        authentication: wire::AuthenticationSelection {
            public_key: *auth.key().as_bytes(),
            credential_revision: *auth.credential_revision().as_bytes(),
        },
    })
}

/// Independent control timing never requires inference/provider configuration.
pub(super) struct ConversationPolicy {
    prompt: Option<ConversationPromptPolicy>,
    lifetime_ms: u64,
    fingerprint: Fingerprint,
}
impl ConversationPolicy {
    pub(super) fn for_prompt(prompt: ConversationPromptPolicy) -> Self {
        Self {
            lifetime_ms: prompt.lifetime_ms,
            fingerprint: prompt.fingerprint,
            prompt: Some(prompt),
        }
    }
    pub(super) fn controls(lifetime_ms: u64) -> Result<Self, ProjectionError> {
        if lifetime_ms == 0
            || lifetime_ms > nocterm_hub_model::cloud::MAX_COMMAND_LIFETIME_MS as u64
        {
            return Err(ProjectionError::InvalidRecord);
        }
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-cloud-control-timing-v1");
        hash.update(lifetime_ms.to_be_bytes());
        Ok(Self {
            prompt: None,
            lifetime_ms,
            fingerprint: Fingerprint::from_bytes(hash.finalize().into()),
        })
    }
    pub(super) fn bind_capture(&mut self) -> Result<(), ProjectionError> {
        let nonce = nocterm_keys::random_bytes::<32>().map_err(|_| ProjectionError::Unavailable)?;
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-conversation-preparation-generation-v1");
        hash.update(self.fingerprint.as_bytes());
        hash.update(nonce);
        self.fingerprint = Fingerprint::from_bytes(hash.finalize().into());
        Ok(())
    }
    pub(super) fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }
    pub(super) fn allows_prompt(&self) -> bool {
        self.prompt.is_some()
    }
    pub(super) fn prompt<'a>(
        &self,
        text: &'a str,
        preparation: ConversationPreparation,
    ) -> Result<wire::CloudCommandPayload<'a>, ProjectionError> {
        if preparation.policy() != self.fingerprint {
            return Err(ProjectionError::WrongSource);
        }
        self.prompt
            .as_ref()
            .ok_or(ProjectionError::Unavailable)?
            .prompt(text, preparation)
    }
    pub(super) fn inference_grant(
        &self,
        preparation: ConversationPreparation,
    ) -> Result<wire::CloudCommandPayload<'static>, ProjectionError> {
        if preparation.policy() != self.fingerprint {
            return Err(ProjectionError::WrongSource);
        }
        self.prompt
            .as_ref()
            .ok_or(ProjectionError::Unavailable)?
            .inference_grant(preparation)
    }
}

impl ConversationPolicy {
    pub(super) fn preparation(
        &self,
        issued: nocterm_cloud_model::identity::Timestamp,
        hard_deadline: i64,
        action: &nocterm_cloud_model::projection::ConversationIntent,
    ) -> Result<ConversationPreparation, ProjectionError> {
        use nocterm_cloud_model::projection::ConversationIntent;
        if matches!(action, ConversationIntent::Prompt(_)) && !self.allows_prompt() {
            return Err(ProjectionError::Unavailable);
        }
        let hard = u64::try_from(hard_deadline).map_err(|_| ProjectionError::Stale)?;
        let expires = issued
            .as_millis()
            .checked_add(self.lifetime_ms)
            .ok_or(ProjectionError::Capacity)?
            .min(hard);
        let expires = match action {
            ConversationIntent::AnswerApproval(request) => {
                expires.min(request.expires().as_millis())
            }
            ConversationIntent::GrantProvider(access) => expires.min(access.expires().as_millis()),
            _ => expires,
        };
        if expires <= issued.as_millis() {
            return Err(ProjectionError::Unavailable);
        }
        ConversationPreparation::new(
            issued,
            nocterm_cloud_model::identity::Timestamp::from_millis(expires),
            self.fingerprint,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::{
        authority::{AuthenticationBinding, TargetBinding},
        identity::{NamespaceId, Timestamp},
        inference::MANAGED_PRICING,
    };

    fn fingerprint(byte: u8) -> Fingerprint {
        Fingerprint::from_bytes([byte; 32])
    }
    fn selection() -> ModelSelection {
        ModelSelection::managed("model-a").unwrap()
    }
    fn budget() -> ModelBudget {
        ModelBudget::new(8, 1024 * 1024, 1024 * 1024, 4096).unwrap()
    }
    fn destination() -> RunDestination {
        let target = TargetBinding::new(
            fingerprint(5),
            fingerprint(6),
            NamespaceId::new("user").unwrap(),
            fingerprint(7),
        );
        RunDestination::new(
            target,
            AuthenticationBinding::new(fingerprint(8), fingerprint(9)),
            None,
        )
        .unwrap()
    }
    fn targeted() -> ConversationPromptPolicy {
        ConversationPromptPolicy::new(selection(), destination(), budget(), 10, 60_000, 60_000)
            .unwrap()
    }
    fn default_destination() -> ConversationPromptPolicy {
        ConversationPromptPolicy::runner_default(selection(), budget(), 10, 60_000, 60_000).unwrap()
    }
    fn preparation(policy: &ConversationPromptPolicy) -> ConversationPreparation {
        ConversationPreparation::new(
            Timestamp::from_millis(1),
            Timestamp::from_millis(10_000),
            policy.fingerprint,
        )
        .unwrap()
    }
    fn submitted(policy: &ConversationPromptPolicy) -> wire::PromptPayload<'_> {
        let payload = policy.prompt("hello", preparation(policy)).unwrap();
        match payload {
            wire::CloudCommandPayload::SubmitPrompt(prompt) => *prompt,
            _ => panic!("expected a prompt"),
        }
    }

    #[test]
    fn a_runner_default_prompt_names_no_target_or_session() {
        let policy = default_destination();
        let prompt = submitted(&policy);
        assert!(prompt.target.is_none());
        assert!(prompt.existing_session.is_none());
        assert_eq!(prompt.model.pricing_policy.as_str(), MANAGED_PRICING);
    }

    #[test]
    fn a_targeted_prompt_still_names_its_destination() {
        let policy = targeted();
        assert!(submitted(&policy).target.is_some());
    }

    #[test]
    fn a_default_and_a_targeted_policy_never_share_a_fingerprint() {
        assert_ne!(targeted().fingerprint, default_destination().fingerprint);
    }

    #[test]
    fn a_runner_default_policy_refuses_the_same_invalid_limits_as_a_targeted_one() {
        for (spend, lifetime) in [(0, 60_000), (10, 0)] {
            assert!(
                ConversationPromptPolicy::runner_default(
                    selection(),
                    budget(),
                    spend,
                    lifetime,
                    60_000
                )
                .is_err()
            );
        }
    }

    fn grant_of(
        payload: wire::CloudCommandPayload<'_>,
    ) -> (
        wire::GrantReference,
        wire::ModelSelection,
        wire::InferenceLimits,
    ) {
        let wire::CloudCommandPayload::IssueGrant(grant) = payload else {
            panic!("an inference grant is an IssueGrant command");
        };
        let wire::GrantPayload::Inference {
            grant,
            model,
            limits,
            ..
        } = *grant
        else {
            panic!("expected an inference grant");
        };
        (grant, model, limits)
    }

    #[test]
    fn an_inference_grant_carries_exactly_the_prompt_bounds() {
        let policy = default_destination();
        let prompt = submitted(&policy);
        let (grant, model, limits) =
            grant_of(policy.inference_grant(preparation(&policy)).unwrap());
        assert_eq!(grant.epoch, 1);
        assert_eq!(model, prompt.model);
        assert_eq!(
            limits,
            wire::InferenceLimits {
                deadline_ms: 1 + 30_000,
                ..prompt.inference
            }
        );
    }

    #[test]
    fn an_inference_grant_window_stays_inside_the_prompt_window() {
        let policy = default_destination();
        let prompt = submitted(&policy);
        let (_, _, limits) = grant_of(policy.inference_grant(preparation(&policy)).unwrap());
        assert!(limits.deadline_ms <= prompt.inference.deadline_ms);
    }

    #[test]
    fn each_inference_grant_has_its_own_identity() {
        let policy = default_destination();
        let first = grant_of(policy.inference_grant(preparation(&policy)).unwrap()).0;
        let second = grant_of(policy.inference_grant(preparation(&policy)).unwrap()).0;
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn an_inference_grant_needs_a_prompting_policy_and_its_own_preparation() {
        let controls = ConversationPolicy::controls(60_000).unwrap();
        let prepared = ConversationPreparation::new(
            Timestamp::from_millis(1),
            Timestamp::from_millis(10_000),
            controls.fingerprint(),
        )
        .unwrap();
        assert!(matches!(
            controls.inference_grant(prepared),
            Err(ProjectionError::Unavailable)
        ));
        let prompting = ConversationPolicy::for_prompt(default_destination());
        let foreign = ConversationPreparation::new(
            Timestamp::from_millis(1),
            Timestamp::from_millis(10_000),
            Fingerprint::from_bytes([9; 32]),
        )
        .unwrap();
        assert!(matches!(
            prompting.inference_grant(foreign),
            Err(ProjectionError::WrongSource)
        ));
    }
}
