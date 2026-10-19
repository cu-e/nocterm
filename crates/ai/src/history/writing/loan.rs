//! Unique loans return their original capabilities after physical payload Drop.
use super::{ConstructionRole, RetainedRole, SnapshotOwner, WriterCapabilityError, WriterProducer};
pub(super) struct LatestLoan {
    role: Option<RetainedRole>,
    producer: WriterProducer,
}
impl LatestLoan {
    pub(super) fn new(role: RetainedRole, producer: WriterProducer) -> Self {
        Self {
            role: Some(role),
            producer,
        }
    }
    pub(super) fn role(&self) -> Result<&RetainedRole, WriterCapabilityError> {
        self.role.as_ref().ok_or(WriterCapabilityError::Unavailable)
    }
    pub(super) fn role_mut(&mut self) -> Result<&mut RetainedRole, WriterCapabilityError> {
        self.role.as_mut().ok_or(WriterCapabilityError::Unavailable)
    }
}
impl Drop for LatestLoan {
    fn drop(&mut self) {
        let Some(mut role) = self.role.take() else {
            return;
        };
        role.clear();
        if let Ok(mut available) = self.producer.available()
            && available.latest.is_none()
        {
            available.latest = Some(role)
        }
    }
}
pub(super) struct WriterLoan {
    writer: Option<ConstructionRole>,
    active: Option<RetainedRole>,
    producer: WriterProducer,
}
impl WriterLoan {
    pub(super) fn new(
        active: RetainedRole,
        writer: ConstructionRole,
        producer: WriterProducer,
    ) -> Self {
        Self {
            writer: Some(writer),
            active: Some(active),
            producer,
        }
    }
    pub(super) fn start(&mut self, source: &RetainedRole) -> Result<(), WriterCapabilityError> {
        let active = self
            .active
            .as_mut()
            .ok_or(WriterCapabilityError::Unavailable)?;
        active.clone_from_role(source)?;
        let writer = self
            .writer
            .as_mut()
            .ok_or(WriterCapabilityError::Unavailable)?;
        writer.begin_phase()?;
        writer.retain_source(active)?;
        Ok(())
    }
    pub(super) fn writer(&self) -> Result<&ConstructionRole, WriterCapabilityError> {
        self.writer
            .as_ref()
            .ok_or(WriterCapabilityError::Unavailable)
    }
    pub(super) fn snapshot(&self) -> Result<&SnapshotOwner, WriterCapabilityError> {
        self.active
            .as_ref()
            .and_then(RetainedRole::snapshot)
            .ok_or(WriterCapabilityError::Unavailable)
    }
}
impl Drop for WriterLoan {
    fn drop(&mut self) {
        let (Some(mut writer), Some(mut active)) = (self.writer.take(), self.active.take()) else {
            return;
        };
        writer.close_phase();
        active.clear();
        if let Ok(mut available) = self.producer.available()
            && available.active.is_none()
            && available.writer.is_none()
        {
            available.active = Some(active);
            available.writer = Some(writer);
        }
    }
}
