//! Typed semantic intake after an injected authenticated command producer.
use super::CoordinatorError;
use crate::{
    authority::{
        AuthenticationBinding, CommandIssuer, ExecutionAuthority, RunBinding, SessionReceipt,
        StreamBinding, TargetBinding,
    },
    engine::{EnginePrompt, ModelBudget},
    grant::{GrantPolicy, GrantWindow},
    identity::*,
    inference::{InferenceGrant, ModelSelection},
    journal::{CommandAcceptance, CommandKind},
};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct RunDestination {
    target: TargetBinding,
    authentication: AuthenticationBinding,
    session: Option<SessionReceipt>,
}
impl RunDestination {
    pub fn new(
        target: TargetBinding,
        authentication: AuthenticationBinding,
        session: Option<SessionReceipt>,
    ) -> Result<Self, CoordinatorError> {
        if session.as_ref().is_some_and(|receipt| {
            receipt.target() != &target || receipt.authentication() != &authentication
        }) {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            target,
            authentication,
            session,
        })
    }
    pub fn target(&self) -> &TargetBinding {
        &self.target
    }
    pub fn authentication(&self) -> &AuthenticationBinding {
        &self.authentication
    }
    pub fn session(&self) -> Option<&SessionReceipt> {
        self.session.as_ref()
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PromptAllowance {
    budget: ModelBudget,
    spend: u64,
    window: GrantWindow,
}
impl PromptAllowance {
    pub fn new(
        budget: ModelBudget,
        spend: u64,
        window: GrantWindow,
    ) -> Result<Self, CoordinatorError> {
        if spend == 0 {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            budget,
            spend,
            window,
        })
    }
    pub fn budget(self) -> ModelBudget {
        self.budget
    }
    pub fn spend_micros(self) -> u64 {
        self.spend
    }
    pub fn window(self) -> GrantWindow {
        self.window
    }
}
#[derive(Clone, Debug)]
pub struct PromptRequest {
    prompt: EnginePrompt,
    selection: ModelSelection,
    destination: RunDestination,
    allowance: PromptAllowance,
}
impl PromptRequest {
    pub fn new(
        prompt: EnginePrompt,
        selection: ModelSelection,
        destination: RunDestination,
        allowance: PromptAllowance,
    ) -> Self {
        Self {
            prompt,
            selection,
            destination,
            allowance,
        }
    }
    pub fn prompt(&self) -> &EnginePrompt {
        &self.prompt
    }
    pub fn selection(&self) -> &ModelSelection {
        &self.selection
    }
    pub fn destination(&self) -> &RunDestination {
        &self.destination
    }
    pub fn allowance(&self) -> PromptAllowance {
        self.allowance
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalChoice {
    Approve,
    Deny,
    ReconcileKnown(Fingerprint),
}
#[derive(Clone, Debug)]
pub struct ApprovalAnswer {
    intent: IntentId,
    authority: ExecutionAuthority,
    arguments: Fingerprint,
    choice: ApprovalChoice,
}
impl ApprovalAnswer {
    pub fn new(
        intent: IntentId,
        authority: ExecutionAuthority,
        arguments: Fingerprint,
        choice: ApprovalChoice,
    ) -> Self {
        Self {
            intent,
            authority,
            arguments,
            choice,
        }
    }
    pub fn intent(&self) -> IntentId {
        self.intent
    }
    pub fn authority(&self) -> &ExecutionAuthority {
        &self.authority
    }
    pub fn arguments(&self) -> Fingerprint {
        self.arguments
    }
    pub fn choice(&self) -> ApprovalChoice {
        self.choice
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantClass {
    Inference,
    Tools,
    Provider,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GrantReference {
    class: GrantClass,
    id: GrantId,
    epoch: GrantEpoch,
}
impl GrantReference {
    pub fn new(class: GrantClass, id: GrantId, epoch: GrantEpoch) -> Self {
        Self { class, id, epoch }
    }
    pub fn class(self) -> GrantClass {
        self.class
    }
    pub fn id(self) -> GrantId {
        self.id
    }
    pub fn epoch(self) -> GrantEpoch {
        self.epoch
    }
}
/// The source stages the actual boot-bound capsule by original CommandId. This
/// value never contains or reconstructs a provider secret/ciphertext envelope.
#[derive(Clone, Debug)]
pub struct ProviderOffer {
    reference: GrantReference,
    provider: NamespaceId,
    models: Vec<NamespaceId>,
    boot: Fingerprint,
    context: Fingerprint,
    expires: Timestamp,
}
impl ProviderOffer {
    pub fn new(
        reference: GrantReference,
        provider: NamespaceId,
        models: Vec<NamespaceId>,
        boot: Fingerprint,
        context: Fingerprint,
        expires: Timestamp,
    ) -> Result<Self, CoordinatorError> {
        if reference.class() != GrantClass::Provider
            || models.is_empty()
            || models.len() > 128
            || models.iter().collect::<BTreeSet<_>>().len() != models.len()
            || expires.as_millis() == 0
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            reference,
            provider,
            models,
            boot,
            context,
            expires,
        })
    }
    pub fn reference(&self) -> GrantReference {
        self.reference
    }
    pub fn provider(&self) -> &NamespaceId {
        &self.provider
    }
    pub fn models(&self) -> &[NamespaceId] {
        &self.models
    }
    pub fn boot(&self) -> Fingerprint {
        self.boot
    }
    pub fn context(&self) -> Fingerprint {
        self.context
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}
/// Complete non-secret original attempt coordinates; the separately staged
/// purpose-specific SSH proof is consumed only by the injected consent owner.
#[derive(Clone, Debug)]
pub struct SessionConsent {
    attempt: AttemptId,
    nonce: Fingerprint,
    run: RunBinding,
    selection: RunDestination,
    algorithm: NamespaceId,
    approved: bool,
    expires: Timestamp,
}
impl SessionConsent {
    pub fn new(
        attempt: AttemptId,
        nonce: Fingerprint,
        run: RunBinding,
        selection: RunDestination,
        algorithm: NamespaceId,
        approved: bool,
        expires: Timestamp,
    ) -> Self {
        Self {
            attempt,
            nonce,
            run,
            selection,
            algorithm,
            approved,
            expires,
        }
    }
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub fn nonce(&self) -> Fingerprint {
        self.nonce
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn target(&self) -> &TargetBinding {
        self.selection.target()
    }
    pub fn authentication(&self) -> &AuthenticationBinding {
        self.selection.authentication()
    }
    pub fn session(&self) -> Option<&SessionReceipt> {
        self.selection.session()
    }
    pub fn algorithm(&self) -> &NamespaceId {
        &self.algorithm
    }
    pub fn approved(&self) -> bool {
        self.approved
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
/// Original device plus exact Ed25519 public key bytes; no hashing is implied.
pub struct IssuerSubject {
    device: NamespaceId,
    key: Fingerprint,
}
impl IssuerSubject {
    pub fn new(device: NamespaceId, key: Fingerprint) -> Self {
        Self { device, key }
    }
    pub fn from_issuer(issuer: &CommandIssuer) -> Self {
        Self::new(issuer.device().clone(), issuer.key())
    }
    pub fn device(&self) -> &NamespaceId {
        &self.device
    }
    pub fn key(&self) -> Fingerprint {
        self.key
    }
}
#[derive(Clone, Debug)]
pub struct IssuerRevocation {
    issuer: IssuerSubject,
    cutoff: ChatControlRevision,
    grants: Vec<GrantReference>,
}
impl IssuerRevocation {
    pub fn new(
        issuer: IssuerSubject,
        cutoff: ChatControlRevision,
        grants: Vec<GrantReference>,
    ) -> Result<Self, CoordinatorError> {
        if grants.len() > 256 {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            issuer,
            cutoff,
            grants,
        })
    }
    pub fn issuer(&self) -> &IssuerSubject {
        &self.issuer
    }
    pub fn cutoff(&self) -> ChatControlRevision {
        self.cutoff
    }
    pub fn grants(&self) -> &[GrantReference] {
        &self.grants
    }
}
#[derive(Clone, Debug)]
pub enum CommandBody {
    Submit(PromptRequest),
    Stop,
    Approval(ApprovalAnswer),
    InferenceGrant(InferenceGrant),
    ToolGrant(GrantPolicy),
    ProviderGrant(ProviderOffer),
    RevokeGrant(GrantReference),
    SessionConsent(SessionConsent),
    DeleteChat,
    RevokeIssuer(IssuerRevocation),
}
impl CommandBody {
    pub fn kind(&self) -> CommandKind {
        match self {
            Self::Submit(_) => CommandKind::SubmitPrompt,
            Self::Stop => CommandKind::Stop,
            Self::Approval(_) => CommandKind::ResolveApproval,
            Self::InferenceGrant(_) | Self::ToolGrant(_) | Self::ProviderGrant(_) => {
                CommandKind::IssueGrant
            }
            Self::RevokeGrant(_) => CommandKind::RevokeGrant,
            Self::SessionConsent(_) => CommandKind::AuthenticationConsent,
            Self::DeleteChat => CommandKind::DeleteChat,
            Self::RevokeIssuer(_) => CommandKind::RevokeIssuer,
        }
    }
    /// Finite control responses must reach an admitted waiting run even when
    /// ordinary prompt admission is full. They do not create another prompt.
    pub fn high_priority(&self) -> bool {
        self.kind().high_priority()
    }
}
/// Construction checks shape and repeated coordinates only. Authentication and
/// original digest(M) come solely from the injected producer; no wire re-encode.
#[derive(Clone, Debug)]
pub struct ReceivedCommand {
    stream: StreamBinding,
    acceptance: CommandAcceptance,
    window: GrantWindow,
    run: Option<RunBinding>,
    body: CommandBody,
    hard_expires: Option<Timestamp>,
}
impl ReceivedCommand {
    pub fn new(
        stream: StreamBinding,
        acceptance: CommandAcceptance,
        window: GrantWindow,
        run: Option<RunBinding>,
        body: CommandBody,
        hard_expires: Option<Timestamp>,
    ) -> Result<Self, CoordinatorError> {
        if acceptance.kind() != body.kind()
            || run.as_ref().is_some_and(|run| run.stream() != stream)
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        let expected = match &body {
            CommandBody::Approval(answer) => Some(answer.authority().run()),
            CommandBody::InferenceGrant(grant) => Some(grant.allowance().run()),
            CommandBody::ToolGrant(grant) => Some(grant.authority().run()),
            CommandBody::SessionConsent(consent) => Some(consent.run()),
            _ => None,
        };
        if expected.is_some_and(|expected| run.as_ref() != Some(expected))
            || (matches!(body, CommandBody::Stop | CommandBody::RevokeGrant(_)) && run.is_none())
            || (matches!(body, CommandBody::Submit(_)) && run.is_some())
        {
            return Err(CoordinatorError::InvalidCommand);
        }
        if let CommandBody::RevokeIssuer(revoke) = &body {
            if revoke.issuer().device() != acceptance.issuer().device()
                || revoke.issuer().key() != acceptance.issuer().key()
                || revoke.cutoff() > acceptance.expected_control()
            {
                return Err(CoordinatorError::InvalidCommand);
            }
        }
        let origin = match &body {
            CommandBody::InferenceGrant(grant) => Some(grant.origin()),
            CommandBody::ToolGrant(grant) => Some(grant.origin()),
            _ => None,
        };
        if origin.is_some_and(|origin| {
            origin.issuer() != acceptance.issuer()
                || origin.command() != acceptance.id()
                || acceptance.expected_control().next().ok() != Some(origin.control())
        }) {
            return Err(CoordinatorError::InvalidCommand);
        }
        Ok(Self {
            stream,
            acceptance,
            window,
            run,
            body,
            hard_expires,
        })
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn acceptance(&self) -> &CommandAcceptance {
        &self.acceptance
    }
    pub fn window(&self) -> GrantWindow {
        self.window
    }
    pub fn run(&self) -> Option<&RunBinding> {
        self.run.as_ref()
    }
    pub fn body(&self) -> &CommandBody {
        &self.body
    }
    pub fn hard_expires(&self) -> Option<Timestamp> {
        self.hard_expires
    }
    /// Bounded reservation for a trusted, already materialized convenience
    /// caller. Native receive must reserve before decoding instead. Variable
    /// prompt bytes are measured; the fixed reserve covers all bounded native
    /// coordinates, provider model lists and issuer revocation metadata.
    pub fn reservation_bytes(&self) -> usize {
        let prompt = match &self.body {
            CommandBody::Submit(request) => {
                request.prompt().encoded_bytes() + std::mem::size_of_val(request.prompt().content())
            }
            _ => 0,
        };
        std::mem::size_of::<Self>() + 64 * 1024 + prompt
    }
    pub fn acknowledgement(&self) -> CommandAcknowledgement {
        CommandAcknowledgement {
            stream: self.stream.clone(),
            acceptance: self.acceptance.clone(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct CommandAcknowledgement {
    stream: StreamBinding,
    acceptance: CommandAcceptance,
}
impl CommandAcknowledgement {
    /// Descriptive verified-header coordinates; this constructor authenticates
    /// nothing and cannot authorize execution or a final transport ACK.
    pub fn new(stream: StreamBinding, acceptance: CommandAcceptance) -> Self {
        Self { stream, acceptance }
    }
    pub fn stream(&self) -> &StreamBinding {
        &self.stream
    }
    pub fn acceptance(&self) -> &CommandAcceptance {
        &self.acceptance
    }
}
