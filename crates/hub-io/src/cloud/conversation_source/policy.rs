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
    destination: RunDestination,
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
        let destination = &self.destination;
        let access = target(destination.target(), destination.authentication())?;
        let existing_session = destination
            .session()
            .map(|value| {
                Ok::<_, ProjectionError>(wire::SessionReference {
                    id: *value.id().as_bytes(),
                    epoch: value.epoch().get(),
                    access: target(value.target(), value.authentication())?,
                })
            })
            .transpose()?;
        let inference = wire::InferenceLimits {
            calls: self.budget.calls(),
            turns: self.budget.turns(),
            request_bytes: self.budget.input_bytes() as u64,
            output_bytes: self.budget.output_bytes() as u64,
            output_tokens: self.budget.output_tokens(),
            spend_micros: self.spend,
            deadline_ms: i64::try_from(
                preparation
                    .issued()
                    .as_millis()
                    .checked_add(self.inference_lifetime_ms)
                    .ok_or(ProjectionError::InvalidRecord)?,
            )
            .map_err(|_| ProjectionError::InvalidRecord)?,
        };
        Ok(wire::CloudCommandPayload::SubmitPrompt(Box::new(
            wire::PromptPayload {
                content: vec![wire::PromptPart::Text(text)],
                model: selection,
                target: Some(access),
                existing_session,
                inference,
            },
        )))
    }
    fn digest(&self) -> Fingerprint {
        let mut hash = Sha256::new();
        hash.update(b"nocterm-native-conversation-policy-v1");
        for value in [
            self.selection.provider().as_str(),
            self.selection.model().as_str(),
            self.selection.pricing().as_str(),
            self.destination.target().principal().as_str(),
        ] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        for value in [
            self.destination.target().host(),
            self.destination.target().route(),
            self.destination.target().workspace(),
            self.destination.authentication().key(),
            self.destination.authentication().credential_revision(),
        ] {
            hash.update(value.as_bytes());
        }
        if let Some(session) = self.destination.session() {
            hash.update([1]);
            hash.update(session.id().as_bytes());
            hash.update(session.epoch().get().to_be_bytes());
        } else {
            hash.update([0]);
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
