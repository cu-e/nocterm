//! Chat SSH bindings and certificates (ADR 0137) as native command bodies.
//!
//! The contract decoder verified the binding's CA signature and that it names the routed
//! namespace, runner and chat; this adds the chat key epoch and derives each target's
//! per-server principal, so the runner never needs the contract to compare certificates.
use super::*;
use nocterm_cloud_model::ssh_access::{
    BoundTarget, SshBindingOffer, SshCertificateOffer, SshEndpoint, SshServerKey,
};
use nocterm_hub_model::cloud::ssh::SignedSshBinding;
use nocterm_keys::agents::ssh_ca::principal_for;

/// A chat-scoped SSH grant: a run coordinate is refused.
pub(in super::super) fn grant(
    grant: wire::GrantPayload<'_>,
    has_run: bool,
    stream: &StreamBinding,
    key_epoch: Option<ChatKeyEpoch>,
) -> Result<CommandBody, CoordinatorError> {
    match grant {
        _ if has_run => Err(CoordinatorError::InvalidCommand),
        wire::GrantPayload::SshBinding { binding } => self::binding(&binding, stream, key_epoch),
        wire::GrantPayload::SshCertificate {
            grant,
            request,
            certificate,
        } => self::certificate(grant, request, certificate),
        _ => Err(CoordinatorError::InvalidCommand),
    }
}

pub(in super::super) fn binding(
    signed: &SignedSshBinding,
    stream: &StreamBinding,
    key_epoch: Option<ChatKeyEpoch>,
) -> Result<CommandBody, CoordinatorError> {
    let binding = signed.binding();
    if binding.chat.chat_id() != *stream.chat().as_bytes()
        || key_epoch.map(ChatKeyEpoch::get) != Some(binding.chat.key_epoch())
    {
        return Err(CoordinatorError::InvalidCommand);
    }
    let targets = binding
        .targets
        .iter()
        .map(|target| BoundTarget {
            server: SshServerKey::from_bytes(*target.server.as_bytes()),
            endpoint: SshEndpoint {
                host: target.host.as_str().to_owned(),
                port: target.port,
                user: target.user.as_str().to_owned(),
            },
            host_key: Fingerprint::from_bytes(target.host_key_sha256),
            principal: principal_for(&binding.ca_key, &target.server),
        })
        .collect();
    let offer = SshBindingOffer::new(
        reference(binding.grant)?,
        timestamp(binding.window.issued_at_ms)?,
        timestamp(binding.window.expires_at_ms)?,
        binding.ca_key,
        targets,
        nocterm_cloud_model::payload::OpaqueBytes::new(signed.canonical_bytes().map_err(backend)?)
            .map_err(backend)?,
    )
    .map_err(backend)?;
    Ok(CommandBody::SshBinding(Box::new(offer)))
}

pub(in super::super) fn certificate(
    grant: wire::GrantReference,
    request: [u8; 16],
    certificate: &[u8],
) -> Result<CommandBody, CoordinatorError> {
    let offer = SshCertificateOffer::new(
        reference(grant)?,
        request,
        nocterm_cloud_model::payload::OpaqueBytes::new(certificate.to_vec()).map_err(backend)?,
    )
    .map_err(backend)?;
    Ok(CommandBody::SshCertificate(Box::new(offer)))
}

fn reference(grant: wire::GrantReference) -> Result<GrantReference, CoordinatorError> {
    Ok(GrantReference::new(
        GrantClass::SshBinding,
        GrantId::from_bytes(grant.id),
        GrantEpoch::new(grant.epoch)?,
    ))
}
