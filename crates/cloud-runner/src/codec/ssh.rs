//! Native form of a chat's SSH binding offer, shared by its command record and its
//! journal v8 row (ADR 0137). The signed contract bytes are kept as they arrived.
use nocterm_cloud_model::{
    coordinator::{GrantClass, GrantReference},
    identity::{Fingerprint, GrantEpoch, GrantId, Timestamp},
    journal::JournalError,
    payload::OpaqueBytes,
    ssh_access::{BoundTarget, SshBindingOffer, SshEndpoint, SshServerKey},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OfferRow {
    grant: ([u8; 16], u64),
    window: (u64, u64),
    ca: [u8; 32],
    targets: Vec<TargetRow>,
    signed: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetRow {
    server: [u8; 16],
    host: String,
    port: u16,
    user: String,
    host_key: [u8; 32],
    principal: String,
}
impl From<&SshBindingOffer> for OfferRow {
    fn from(offer: &SshBindingOffer) -> Self {
        Self {
            grant: (*offer.grant().id().as_bytes(), offer.grant().epoch().get()),
            window: (offer.issued().as_millis(), offer.expires().as_millis()),
            ca: *offer.ca_key(),
            targets: offer
                .targets()
                .iter()
                .map(|target| TargetRow {
                    server: *target.server.as_bytes(),
                    host: target.endpoint.host.clone(),
                    port: target.endpoint.port,
                    user: target.endpoint.user.clone(),
                    host_key: *target.host_key.as_bytes(),
                    principal: target.principal.clone(),
                })
                .collect(),
            signed: offer.signed().as_bytes().to_vec(),
        }
    }
}
impl OfferRow {
    pub(crate) fn restore(self) -> Result<SshBindingOffer, JournalError> {
        let targets = self
            .targets
            .into_iter()
            .map(|target| BoundTarget {
                server: SshServerKey::from_bytes(target.server),
                endpoint: SshEndpoint {
                    host: target.host,
                    port: target.port,
                    user: target.user,
                },
                host_key: Fingerprint::from_bytes(target.host_key),
                principal: target.principal,
            })
            .collect();
        SshBindingOffer::new(
            ssh_grant(self.grant.0, self.grant.1)?,
            Timestamp::from_millis(self.window.0),
            Timestamp::from_millis(self.window.1),
            self.ca,
            targets,
            OpaqueBytes::new(self.signed)?,
        )
        .map_err(|_| JournalError::Corrupt)
    }
}

/// The reference of a chat's SSH binding grant.
pub(crate) fn ssh_grant(id: [u8; 16], epoch: u64) -> Result<GrantReference, JournalError> {
    Ok(GrantReference::new(
        GrantClass::SshBinding,
        GrantId::from_bytes(id),
        GrantEpoch::new(epoch).map_err(|_| JournalError::Corrupt)?,
    ))
}
