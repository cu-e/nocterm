//! Native form of a chat's SSH binding offer, shared by its command record and its
//! journal SSH row (ADR 0137). The signed contract bytes are kept as they arrived. Each
//! target's session access (ADR 0139) is absent in records written before it existed.
use nocterm_cloud_model::{
    authority::{AuthenticationBinding, TargetBinding},
    coordinator::{GrantClass, GrantReference},
    identity::{Fingerprint, GrantEpoch, GrantId, NamespaceId, Timestamp},
    journal::JournalError,
    payload::OpaqueBytes,
    ssh_access::{BoundAccess, BoundTarget, SshBindingOffer, SshEndpoint, SshServerKey},
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
    #[serde(default)]
    access: Option<AccessRow>,
}
/// A bound server's session access: route, workspace and credential revision. The host
/// key is the target's, the principal its user and the authentication key the CA key.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessRow {
    route: [u8; 32],
    workspace: [u8; 32],
    credential: [u8; 32],
}
impl AccessRow {
    fn of(target: &BoundTarget, ca: &[u8; 32]) -> Option<Self> {
        let access = target.access.as_ref()?;
        // Only an access whose fixed parts are the target's own is kept.
        (access.target.host() == target.host_key
            && access.target.principal().as_str() == target.endpoint.user
            && access.authentication.key().as_bytes() == ca)
            .then(|| Self {
                route: *access.target.route().as_bytes(),
                workspace: *access.target.workspace().as_bytes(),
                credential: *access.authentication.credential_revision().as_bytes(),
            })
    }
    fn restore(self, target: &TargetRow, ca: &[u8; 32]) -> Result<BoundAccess, JournalError> {
        Ok(BoundAccess {
            target: TargetBinding::new(
                Fingerprint::from_bytes(target.host_key),
                Fingerprint::from_bytes(self.route),
                NamespaceId::new(target.user.clone()).map_err(|_| JournalError::Corrupt)?,
                Fingerprint::from_bytes(self.workspace),
            ),
            authentication: AuthenticationBinding::new(
                Fingerprint::from_bytes(*ca),
                Fingerprint::from_bytes(self.credential),
            ),
        })
    }
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
                    access: AccessRow::of(target, offer.ca_key()),
                })
                .collect(),
            signed: offer.signed().as_bytes().to_vec(),
        }
    }
}
impl OfferRow {
    pub(crate) fn restore(self) -> Result<SshBindingOffer, JournalError> {
        let ca = self.ca;
        let targets = self
            .targets
            .into_iter()
            .map(|mut target| {
                let access = target
                    .access
                    .take()
                    .map(|access| access.restore(&target, &ca))
                    .transpose()?;
                Ok(BoundTarget {
                    server: SshServerKey::from_bytes(target.server),
                    endpoint: SshEndpoint {
                        host: target.host,
                        port: target.port,
                        user: target.user,
                    },
                    host_key: Fingerprint::from_bytes(target.host_key),
                    principal: target.principal,
                    access,
                })
            })
            .collect::<Result<_, JournalError>>()?;
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
