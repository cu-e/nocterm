//! Narrow physical prompt and request framing owned by the original dispatch role.
use super::{
    PromptSlot,
    bundle::{self, Body, BundleProjection, BundleSlot},
    slots::{
        IdSlot, NativeSourcePool, PayloadGuard, PhysicalError, TextGuard, TextSlot, optional_text,
    },
};
use crate::model::{
    ids::SessionId,
    retention::{
        CensusPermit, RetentionError,
        document::{ConstructionRole, DocumentRetention, Publication},
        graph::{Count, OwnerGraph, sealed},
    },
};

/// A non-cloneable physical request; only its original producer can admit it.
pub struct DispatchPayload {
    body: BundleSlot<DispatchBody>,
    policy: super::policy::DispatchPolicy,
}
struct DispatchBody {
    prompt: PromptSlot,
    framing: BundleSlot<RequestFraming>,
    attachments: Option<BundleSlot<super::inputs::LiveAttachments>>,
}
struct RequestFraming {
    session: IdSlot<SessionId>,
    context: Option<TextSlot>,
    history: Option<TextSlot>,
    command: bool,
}
#[derive(Clone)]
pub(crate) struct DispatchProjection {
    body: BundleProjection<DispatchBody>,
    policy: usize,
}
pub struct DispatchStaging<'a> {
    payload: DispatchPayload,
    scope: &'a DocumentRetention,
}
impl DispatchPayload {
    pub(super) fn prepare<'a>(
        role: &'a ConstructionRole,
        prompt: &PromptSlot,
        inputs: Option<&super::LiveInputs>,
        framing: crate::history::dispatch::DispatchFraming,
    ) -> Result<DispatchStaging<'a>, PhysicalError> {
        let policy = super::policy::DispatchPolicy::prepare(prompt)?;
        let (source, command) = framing.into_source()?;
        if !source.ledger().belongs_to_document(role.scope()) {
            return Err(RetentionError::ForeignAuthority.into());
        }
        let header = super::slots::NativeCredit::reserve_header(role)?;
        let (value, allocation) = source.into_parts();
        let mut source = FramingSource {
            value: Some(value),
            pool: NativeSourcePool::new(role, allocation, header),
        };
        let original = source.pool.original().clone();
        let ((session, context), history) = source.value.take().expect("fresh dispatch framing");
        let framing = RequestFraming {
            session: IdSlot::session(session),
            context: optional_text(context, &mut source.pool)?,
            history: optional_text(history, &mut source.pool)?,
            command,
        };
        let framing = bundle::part(role, &original, framing)?;
        let body = bundle::part(
            role,
            &original,
            DispatchBody {
                prompt: prompt.alias(),
                framing,
                attachments: inputs.and_then(|inputs| inputs.prompt_attachments(prompt)),
            },
        )?;
        Ok(DispatchStaging {
            payload: Self { body, policy },
            scope: role.scope(),
        })
    }
    pub fn prompt(&self) -> &PromptSlot {
        &self.body.body().prompt
    }
    pub fn live_attachments(&self) -> Option<&[super::LiveAttachmentSlot]> {
        self.body
            .body()
            .attachments
            .as_ref()
            .map(|attachments| attachments.body().values())
    }
    pub fn session(&self) -> PayloadGuard<'_, SessionId> {
        self.body.body().framing.body().session.read()
    }
    pub fn context(&self) -> Option<TextGuard<'_>> {
        self.body
            .body()
            .framing
            .body()
            .context
            .as_ref()
            .map(TextSlot::read)
    }
    pub fn history(&self) -> Option<TextGuard<'_>> {
        self.body
            .body()
            .framing
            .body()
            .history
            .as_ref()
            .map(TextSlot::read)
    }
    pub fn is_command(&self) -> bool {
        self.body.body().framing.body().command
    }
    pub(crate) fn projection(&self) -> DispatchProjection {
        DispatchProjection {
            body: self.body.projection(),
            policy: self.policy.bytes(),
        }
    }
}
impl DispatchStaging<'_> {
    pub(in crate::history) fn for_dispatch(
        payload: DispatchPayload,
        scope: &DocumentRetention,
    ) -> DispatchStaging<'_> {
        DispatchStaging { payload, scope }
    }
    pub(crate) fn check_scope(&self, scope: &DocumentRetention) -> Result<(), RetentionError> {
        if self.scope.same_document(scope) {
            Ok(())
        } else {
            Err(RetentionError::ForeignAuthority)
        }
    }
    pub(crate) fn projection(&self) -> DispatchProjection {
        self.payload.projection()
    }
    pub(crate) fn into_payload(self) -> DispatchPayload {
        self.payload
    }
}
struct FramingSource {
    value: Option<((SessionId, Option<String>), Option<String>)>,
    pool: NativeSourcePool,
}
impl Body for DispatchBody {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.prompt.count(count)?;
        self.framing.count(count)?;
        if let Some(attachments) = &self.attachments {
            attachments.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.prompt.cleanup(permit);
        self.framing.cleanup(permit);
        if let Some(attachments) = &self.attachments {
            attachments.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.prompt.adopt(publication);
        self.framing.adopt(publication);
        if let Some(attachments) = &self.attachments {
            attachments.adopt(publication);
        }
    }
    fn retire(&self) {
        self.prompt.retire();
        self.framing.retire();
        if let Some(attachments) = &self.attachments {
            attachments.retire();
        }
    }
}
impl Body for RequestFraming {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        self.session.count(count)?;
        if let Some(value) = &self.context {
            value.count(count)?;
        }
        if let Some(value) = &self.history {
            value.count(count)?;
        }
        Ok(())
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.session.cleanup(permit);
        if let Some(value) = &self.context {
            value.cleanup(permit);
        }
        if let Some(value) = &self.history {
            value.cleanup(permit);
        }
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.session.adopt(publication);
        if let Some(value) = &self.context {
            value.adopt(publication);
        }
        if let Some(value) = &self.history {
            value.adopt(publication);
        }
    }
    fn retire(&self) {
        self.session.retire();
        if let Some(value) = &self.context {
            value.retire();
        }
        if let Some(value) = &self.history {
            value.retire();
        }
    }
}
impl sealed::Source for DispatchProjection {}
impl OwnerGraph for DispatchProjection {
    fn count(&self, count: &mut Count<'_, '_>) -> Result<(), RetentionError> {
        count.record_policy(Some(if self.body.payload_retired() {
            0
        } else {
            self.policy
        }))?;
        self.body.count(count)
    }
    fn cleanup(&self, permit: &CensusPermit<'_>) {
        self.body.cleanup(permit);
    }
    fn adopt(&self, publication: &mut Publication<'_>) {
        self.body.adopt(publication);
    }
}
