//! Bounded native semantic frames, never public signed command/event bytes.
use crate::codec::{
    identity::{Authority, Run, Stream, fp, namespace},
    record::{command_kind, command_tag},
};
use nocterm_ai::model::content::Content;
use nocterm_cloud_model::{
    authority::*, coordinator::*, engine::*, grant::*, identity::*, inference::*, journal::*,
    payload::OpaqueBytes,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

pub(super) fn encoded_len<T: Serialize>(value: &T) -> Result<usize, CoordinatorError> {
    let mut counter = CountedJson { bytes: 0 };
    serde_json::to_writer(&mut counter, &(1u8, value)).map_err(|_| JournalError::Capacity)?;
    Ok(counter.bytes)
}
pub(super) fn encode<T: Serialize>(value: &T) -> Result<OpaqueBytes, CoordinatorError> {
    encode_counted(value, encoded_len(value)?)
}
pub(super) fn encode_counted<T: Serialize>(
    value: &T,
    bytes: usize,
) -> Result<OpaqueBytes, CoordinatorError> {
    let mut output = JsonOutput {
        bytes: Vec::with_capacity(bytes),
        maximum: bytes,
    };
    serde_json::to_writer(&mut output, &(1u8, value)).map_err(|_| JournalError::Capacity)?;
    if output.bytes.len() != bytes {
        return Err(JournalError::Corrupt.into());
    }
    OpaqueBytes::new(output.bytes).map_err(CoordinatorError::from)
}
struct CountedJson {
    bytes: usize,
}
impl std::io::Write for CountedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|bytes| *bytes <= nocterm_cloud_model::payload::MAX_LOGICAL_PAYLOAD_BYTES)
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::OutOfMemory))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct JsonOutput {
    bytes: Vec<u8>,
    maximum: usize,
}
impl std::io::Write for JsonOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::from(std::io::ErrorKind::OutOfMemory));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn decode<T: DeserializeOwned>(bytes: &OpaqueBytes) -> Result<T, CoordinatorError> {
    nocterm_ai::history::structural_bytes(bytes.as_bytes()).map_err(|_| JournalError::Corrupt)?;
    let (version, value): (u8, T) =
        serde_json::from_slice(bytes.as_bytes()).map_err(|_| JournalError::Corrupt)?;
    if version != 1 {
        return Err(JournalError::UnsupportedFormat.into());
    }
    Ok(value)
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Window(u64, u64);
impl From<GrantWindow> for Window {
    fn from(value: GrantWindow) -> Self {
        Self(value.issued().as_millis(), value.expires().as_millis())
    }
}
impl Window {
    pub(super) fn restore(self) -> Result<GrantWindow, CoordinatorError> {
        Ok(GrantWindow::new(
            Timestamp::from_millis(self.0),
            Timestamp::from_millis(self.1),
        )?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Selection(String, String, String);
impl From<&ModelSelection> for Selection {
    fn from(value: &ModelSelection) -> Self {
        Self(
            value.provider().as_str().into(),
            value.model().as_str().into(),
            value.pricing().as_str().into(),
        )
    }
}
impl Selection {
    pub(super) fn restore(self) -> Result<ModelSelection, CoordinatorError> {
        Ok(ModelSelection::new(
            namespace(self.0)?,
            namespace(self.1)?,
            namespace(self.2)?,
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(super) enum Budget {
    Current {
        version: u8,
        calls: u32,
        turns: u32,
        input_bytes: usize,
        output_bytes: usize,
        output_tokens: u64,
    },
    Legacy(u32, usize, usize, u64),
}
impl From<ModelBudget> for Budget {
    fn from(value: ModelBudget) -> Self {
        Self::Current {
            version: 2,
            calls: value.calls(),
            turns: value.turns(),
            input_bytes: value.input_bytes(),
            output_bytes: value.output_bytes(),
            output_tokens: value.output_tokens(),
        }
    }
}
impl Budget {
    pub(super) fn restore(self) -> Result<ModelBudget, CoordinatorError> {
        match self {
            Self::Legacy(turns, input, output, tokens) => {
                Ok(ModelBudget::new(turns, input, output, tokens)?)
            }
            Self::Current {
                version: 2,
                calls,
                turns,
                input_bytes,
                output_bytes,
                output_tokens,
            } => Ok(
                ModelBudget::new(turns, input_bytes, output_bytes, output_tokens)?
                    .with_calls(calls)?,
            ),
            Self::Current { .. } => Err(JournalError::UnsupportedFormat.into()),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Destination {
    host: [u8; 32],
    route: [u8; 32],
    principal: String,
    workspace: [u8; 32],
    key: [u8; 32],
    credential: [u8; 32],
    session: Option<([u8; 16], u64)>,
}
impl From<&RunDestination> for Destination {
    fn from(value: &RunDestination) -> Self {
        let target = value.target();
        Self {
            host: *target.host().as_bytes(),
            route: *target.route().as_bytes(),
            principal: target.principal().as_str().into(),
            workspace: *target.workspace().as_bytes(),
            key: *value.authentication().key().as_bytes(),
            credential: *value.authentication().credential_revision().as_bytes(),
            session: value
                .session()
                .map(|session| (*session.id().as_bytes(), session.epoch().get())),
        }
    }
}
impl Destination {
    pub(super) fn selection(
        target: &TargetBinding,
        authentication: &AuthenticationBinding,
        session: Option<&SessionReceipt>,
    ) -> Self {
        Self {
            host: *target.host().as_bytes(),
            route: *target.route().as_bytes(),
            principal: target.principal().as_str().into(),
            workspace: *target.workspace().as_bytes(),
            key: *authentication.key().as_bytes(),
            credential: *authentication.credential_revision().as_bytes(),
            session: session.map(|session| (*session.id().as_bytes(), session.epoch().get())),
        }
    }
    pub(super) fn restore(self) -> Result<RunDestination, CoordinatorError> {
        let target = TargetBinding::new(
            fp(self.host),
            fp(self.route),
            namespace(self.principal)?,
            fp(self.workspace),
        );
        let authentication = AuthenticationBinding::new(fp(self.key), fp(self.credential));
        let session = self
            .session
            .map(|(id, epoch)| {
                Ok::<_, CoordinatorError>(SessionReceipt::new(
                    RemoteSessionId::from_bytes(id),
                    SessionEpoch::new(epoch)?,
                    target.clone(),
                    authentication.clone(),
                ))
            })
            .transpose()?;
        RunDestination::new(target, authentication, session)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Prompt {
    content: Vec<Content>,
    selection: Selection,
    destination: Destination,
    budget: Budget,
    spend: u64,
    window: Window,
}
impl From<&PromptRequest> for Prompt {
    fn from(value: &PromptRequest) -> Self {
        Self {
            content: value.prompt().content().to_vec(),
            selection: value.selection().into(),
            destination: value.destination().into(),
            budget: value.allowance().budget().into(),
            spend: value.allowance().spend_micros(),
            window: value.allowance().window().into(),
        }
    }
}
impl Prompt {
    pub(super) fn restore(self) -> Result<PromptRequest, CoordinatorError> {
        Ok(PromptRequest::new(
            EnginePrompt::new(self.content)?,
            self.selection.restore()?,
            self.destination.restore()?,
            PromptAllowance::new(self.budget.restore()?, self.spend, self.window.restore()?)?,
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Origin {
    device: String,
    key: [u8; 32],
    invocation: [u8; 32],
    command: [u8; 16],
    control: u64,
}
impl From<&GrantOrigin> for Origin {
    fn from(value: &GrantOrigin) -> Self {
        Self {
            device: value.issuer().device().as_str().into(),
            key: *value.issuer().key().as_bytes(),
            invocation: *value.issuer().invocation().as_bytes(),
            command: *value.command().as_bytes(),
            control: value.control().get(),
        }
    }
}
impl Origin {
    pub(super) fn restore(self) -> Result<GrantOrigin, CoordinatorError> {
        Ok(GrantOrigin::new(
            CommandIssuer::new(namespace(self.device)?, fp(self.key), fp(self.invocation)),
            CommandId::from_bytes(self.command),
            ChatControlRevision::from_u64(self.control),
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Task {
    run: Run,
    selection: Selection,
    budget: Budget,
    spend: u64,
    window: Window,
}
impl From<&InferenceTask> for Task {
    fn from(value: &InferenceTask) -> Self {
        Self {
            run: value.run().into(),
            selection: value.selection().into(),
            budget: value.budget().into(),
            spend: value.spend_micros(),
            window: value.window().into(),
        }
    }
}
impl Task {
    pub(super) fn restore(self) -> Result<InferenceTask, CoordinatorError> {
        Ok(InferenceTask::new(
            self.run.restore()?,
            self.selection.restore()?,
            self.budget.restore()?,
            self.spend,
            self.window.restore()?,
        )?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Inference {
    id: [u8; 16],
    epoch: u64,
    origin: Origin,
    task: Task,
}
impl From<&InferenceGrant> for Inference {
    fn from(value: &InferenceGrant) -> Self {
        Self {
            id: *value.id().as_bytes(),
            epoch: value.epoch().get(),
            origin: value.origin().into(),
            task: value.allowance().into(),
        }
    }
}
impl Inference {
    pub(super) fn restore(self) -> Result<InferenceGrant, CoordinatorError> {
        Ok(InferenceGrant::new(
            GrantId::from_bytes(self.id),
            GrantEpoch::new(self.epoch)?,
            self.origin.restore()?,
            self.task.restore()?,
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolGrant {
    authority: Authority,
    origin: Origin,
    capabilities: u8,
    window: Window,
    operations: u32,
}
impl From<&GrantPolicy> for ToolGrant {
    fn from(value: &GrantPolicy) -> Self {
        Self {
            authority: value.authority().into(),
            origin: value.origin().into(),
            capabilities: value.capabilities().bits(),
            window: value.window().into(),
            operations: value.operations(),
        }
    }
}
impl ToolGrant {
    pub(super) fn restore(self) -> Result<GrantPolicy, CoordinatorError> {
        Ok(GrantPolicy::new(
            self.authority.restore()?,
            self.origin.restore()?,
            Capabilities::from_bits(self.capabilities)?,
            self.window.restore()?,
            self.operations,
        )?)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference(u8, [u8; 16], u64);
impl From<GrantReference> for Reference {
    fn from(value: GrantReference) -> Self {
        Self(
            match value.class() {
                GrantClass::Inference => 1,
                GrantClass::Tools => 2,
                GrantClass::Provider => 3,
            },
            *value.id().as_bytes(),
            value.epoch().get(),
        )
    }
}
impl Reference {
    fn restore(self) -> Result<GrantReference, CoordinatorError> {
        let class = match self.0 {
            1 => GrantClass::Inference,
            2 => GrantClass::Tools,
            3 => GrantClass::Provider,
            _ => return Err(JournalError::Corrupt.into()),
        };
        Ok(GrantReference::new(
            class,
            GrantId::from_bytes(self.1),
            GrantEpoch::new(self.2)?,
        ))
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Body {
    Submit(Prompt),
    Stop,
    Approval([u8; 16], Authority, [u8; 32], Option<Option<[u8; 32]>>),
    Inference(Inference),
    Tools(ToolGrant),
    Provider(Reference, String, Vec<String>, [u8; 32], [u8; 32], u64),
    Revoke(Reference),
    Consent([u8; 16], [u8; 32], Run, Destination, String, bool, u64),
    Delete,
    Issuer(String, [u8; 32], u64, Vec<Reference>),
    Begin(super::session_request::SessionStart),
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Command {
    stream: Stream,
    id: [u8; 16],
    kind: u8,
    digest: [u8; 32],
    device: String,
    key: [u8; 32],
    invocation: [u8; 32],
    control: u64,
    window: Window,
    run: Option<Run>,
    body: Body,
    hard: Option<u64>,
}
impl From<&ReceivedCommand> for Command {
    fn from(value: &ReceivedCommand) -> Self {
        let accepted = value.acceptance();
        let body = match value.body() {
            CommandBody::Submit(prompt) => Body::Submit(prompt.into()),
            CommandBody::Stop => Body::Stop,
            CommandBody::Approval(answer) => Body::Approval(
                *answer.intent().as_bytes(),
                answer.authority().into(),
                *answer.arguments().as_bytes(),
                match answer.choice() {
                    ApprovalChoice::Approve => None,
                    ApprovalChoice::Deny => Some(None),
                    ApprovalChoice::ReconcileKnown(digest) => Some(Some(*digest.as_bytes())),
                },
            ),
            CommandBody::InferenceGrant(grant) => Body::Inference(grant.into()),
            CommandBody::ToolGrant(grant) => Body::Tools(grant.into()),
            CommandBody::ProviderGrant(offer) => Body::Provider(
                offer.reference().into(),
                offer.provider().as_str().into(),
                offer.models().iter().map(|id| id.as_str().into()).collect(),
                *offer.boot().as_bytes(),
                *offer.context().as_bytes(),
                offer.expires().as_millis(),
            ),
            CommandBody::RevokeGrant(reference) => Body::Revoke((*reference).into()),
            CommandBody::SessionConsent(consent) => Body::Consent(
                *consent.attempt().as_bytes(),
                *consent.nonce().as_bytes(),
                consent.run().into(),
                Destination::selection(
                    consent.target(),
                    consent.authentication(),
                    consent.session(),
                ),
                consent.algorithm().as_str().into(),
                consent.approved(),
                consent.expires().as_millis(),
            ),
            CommandBody::BeginSession(request) => Body::Begin(request.as_ref().into()),
            CommandBody::DeleteChat => Body::Delete,
            CommandBody::RevokeIssuer(revoke) => Body::Issuer(
                revoke.issuer().device().as_str().into(),
                *revoke.issuer().key().as_bytes(),
                revoke.cutoff().get(),
                revoke
                    .grants()
                    .iter()
                    .copied()
                    .map(Reference::from)
                    .collect(),
            ),
        };
        Self {
            stream: value.stream().into(),
            id: *accepted.id().as_bytes(),
            kind: command_tag(accepted.kind()),
            digest: *accepted.digest().as_bytes(),
            device: accepted.issuer().device().as_str().into(),
            key: *accepted.issuer().key().as_bytes(),
            invocation: *accepted.issuer().invocation().as_bytes(),
            control: accepted.expected_control().get(),
            window: value.window().into(),
            run: value.run().map(Run::from),
            body,
            hard: value.hard_expires().map(Timestamp::as_millis),
        }
    }
}
impl Command {
    pub(super) fn restore(self) -> Result<ReceivedCommand, CoordinatorError> {
        let body = match self.body {
            Body::Submit(prompt) => CommandBody::Submit(prompt.restore()?),
            Body::Stop => CommandBody::Stop,
            Body::Approval(id, authority, digest, choice) => {
                CommandBody::Approval(ApprovalAnswer::new(
                    IntentId::from_bytes(id),
                    authority.restore()?,
                    fp(digest),
                    match choice {
                        None => ApprovalChoice::Approve,
                        Some(None) => ApprovalChoice::Deny,
                        Some(Some(digest)) => ApprovalChoice::ReconcileKnown(fp(digest)),
                    },
                ))
            }
            Body::Inference(grant) => CommandBody::InferenceGrant(grant.restore()?),
            Body::Tools(grant) => CommandBody::ToolGrant(grant.restore()?),
            Body::Provider(reference, provider, models, boot, context, expires) => {
                CommandBody::ProviderGrant(ProviderOffer::new(
                    reference.restore()?,
                    namespace(provider)?,
                    models
                        .into_iter()
                        .map(namespace)
                        .collect::<Result<_, _>>()?,
                    fp(boot),
                    fp(context),
                    Timestamp::from_millis(expires),
                )?)
            }
            Body::Revoke(reference) => CommandBody::RevokeGrant(reference.restore()?),
            Body::Consent(attempt, nonce, run, destination, algorithm, approved, expires) => {
                CommandBody::SessionConsent(SessionConsent::new(
                    AttemptId::from_bytes(attempt),
                    fp(nonce),
                    run.restore()?,
                    destination.restore()?,
                    namespace(algorithm)?,
                    approved,
                    Timestamp::from_millis(expires),
                ))
            }
            Body::Delete => CommandBody::DeleteChat,
            Body::Begin(request) => CommandBody::BeginSession(Box::new(request.restore()?)),
            Body::Issuer(device, key, cutoff, grants) => {
                CommandBody::RevokeIssuer(IssuerRevocation::new(
                    IssuerSubject::new(namespace(device)?, fp(key)),
                    ChatControlRevision::from_u64(cutoff),
                    grants
                        .into_iter()
                        .map(Reference::restore)
                        .collect::<Result<_, _>>()?,
                )?)
            }
        };
        ReceivedCommand::new(
            self.stream.restore()?,
            CommandAcceptance::new(
                CommandId::from_bytes(self.id),
                command_kind(self.kind)?,
                fp(self.digest),
                CommandIssuer::new(namespace(self.device)?, fp(self.key), fp(self.invocation)),
                ChatControlRevision::from_u64(self.control),
            ),
            self.window.restore()?,
            self.run.map(Run::restore).transpose()?,
            body,
            self.hard.map(Timestamp::from_millis),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counted_encoding_matches_the_direct_native_serializer_without_an_intermediate_value() {
        let value = ("escaped \"text\"", [1u8, 2, 3]);
        let expected = serde_json::to_vec(&(1u8, &value)).unwrap();
        assert_eq!(encoded_len(&value).unwrap(), expected.len());
        assert_eq!(encode(&value).unwrap().as_bytes(), expected);
    }
    #[test]
    fn an_oversized_frame_stops_in_the_counting_pass_before_an_output_generation_exists() {
        struct Oversized(std::sync::atomic::AtomicUsize);
        impl Serialize for Oversized {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut sequence = serializer.serialize_seq(None)?;
                let chunk = "x".repeat(1024);
                for _ in 0..=(nocterm_cloud_model::payload::MAX_LOGICAL_PAYLOAD_BYTES / 1024) {
                    sequence.serialize_element(&chunk)?;
                }
                sequence.end()
            }
        }
        let value = Oversized(std::sync::atomic::AtomicUsize::new(0));
        assert!(matches!(
            encode(&value),
            Err(CoordinatorError::Journal(JournalError::Capacity))
        ));
        assert_eq!(value.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    #[test]
    fn legacy_budget_tuple_keeps_the_original_coupled_meaning() {
        let budget: Budget = serde_json::from_str("[4,4096,1024,100]").unwrap();
        let restored = budget.restore().unwrap();
        assert_eq!((restored.calls(), restored.turns()), (4, 4));
        assert!(serde_json::from_str::<Budget>("[1,4,4096,1024,100]").is_err());
    }
    #[test]
    fn versioned_budget_roundtrip_preserves_independent_calls_and_refuses_unknown_versions() {
        let original = ModelBudget::new(4, 4096, 1024, 100)
            .unwrap()
            .with_calls(1)
            .unwrap();
        let bytes = serde_json::to_vec(&Budget::from(original)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["version"], 2);
        assert_eq!(value["calls"], 1);
        let restored = serde_json::from_slice::<Budget>(&bytes)
            .unwrap()
            .restore()
            .unwrap();
        assert_eq!((restored.calls(), restored.turns()), (1, 4));
        let mut unknown = value;
        unknown["version"] = serde_json::json!(3);
        assert!(matches!(
            serde_json::from_value::<Budget>(unknown).unwrap().restore(),
            Err(CoordinatorError::Journal(JournalError::UnsupportedFormat))
        ));
    }
}
