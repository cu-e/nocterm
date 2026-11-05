//! A chat's SSH access events (ADR 0137) as the contract's events 14 and 15.
use super::encoding_error;
use nocterm_cloud_model::{
    journal::JournalError,
    ssh_access::{
        SshAccessEvent, SshAccessPhase, SshAccessReport, SshBindingRefusal, SshCertificateRequest,
        SshServerReach,
    },
};
use nocterm_hub_model::cloud::{
    events::CloudEventPayload,
    payload::GrantReference,
    ssh::{self as wire, SignedSshBinding, SshServerId},
};

pub(super) fn with_payload<T>(
    event: &SshAccessEvent,
    emit: impl for<'a> FnOnce(CloudEventPayload<'a>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    match event {
        SshAccessEvent::Requested(request) => emit(CloudEventPayload::SshCertificateRequested(
            Box::new(self::request(request)?),
        )),
        SshAccessEvent::State(report) => {
            emit(CloudEventPayload::SshAccessState(Box::new(update(report)?)))
        }
    }
}

/// Bytes the mapped values own beyond their fixed size: the decoded binding echo.
pub(super) fn mapping_bytes(event: &SshAccessEvent) -> Result<usize, JournalError> {
    Ok(match event {
        SshAccessEvent::Requested(request) => std::mem::size_of::<wire::SshCertificateRequest>()
            .checked_add(request.binding.len().saturating_mul(2))
            .ok_or(JournalError::Capacity)?,
        SshAccessEvent::State(_) => std::mem::size_of::<wire::SshAccessUpdate>(),
    })
}

fn request(value: &SshCertificateRequest) -> Result<wire::SshCertificateRequest, JournalError> {
    let request = wire::SshCertificateRequest {
        request: value.request,
        binding: SignedSshBinding::from_canonical_bytes(value.binding.as_bytes())
            .map_err(encoding_error)?,
        subject_key: value.subject_key,
        source_addresses: value.source_addresses.clone(),
        servers: value
            .servers
            .iter()
            .map(|server| SshServerId::from_bytes(*server.as_bytes()))
            .collect(),
        not_after_ms: millis(value.not_after.as_millis())?,
    };
    request.validate().map_err(encoding_error)?;
    Ok(request)
}

/// A report as the contract's `SshAccessState`, also the input of a summary's servers.
pub(in crate::cloud) fn update(
    report: &SshAccessReport,
) -> Result<wire::SshAccessUpdate, JournalError> {
    let refusal = |refusal| match refusal {
        SshBindingRefusal::NoSourceAddress => wire::SshBindingRefusal::NoSourceAddress,
        SshBindingRefusal::NoReachableServer => wire::SshBindingRefusal::NoReachableServer,
        SshBindingRefusal::Invalid => wire::SshBindingRefusal::Invalid,
    };
    let access = match report.access {
        SshAccessPhase::Pending => wire::SshAccess::Pending,
        SshAccessPhase::Active {
            request,
            valid_before,
        } => wire::SshAccess::Active {
            request,
            valid_before_ms: millis(valid_before.as_millis())?,
        },
        SshAccessPhase::Refused(value) => wire::SshAccess::Refused(refusal(value)),
        SshAccessPhase::Revoked => wire::SshAccess::Revoked,
        SshAccessPhase::Expired => wire::SshAccess::Expired,
    };
    let update = wire::SshAccessUpdate {
        grant: GrantReference {
            id: *report.grant.id().as_bytes(),
            epoch: report.grant.epoch().get(),
        },
        access,
        servers: report
            .servers
            .iter()
            .map(|(server, reach)| wire::SshServerStatus {
                server: SshServerId::from_bytes(*server.as_bytes()),
                reach: match reach {
                    SshServerReach::Admitted => wire::SshServerReach::Admitted,
                    SshServerReach::Unreachable => wire::SshServerReach::Unreachable,
                },
            })
            .collect(),
    };
    update.validate().map_err(encoding_error)?;
    Ok(update)
}

fn millis(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::InvalidBatch)
}
