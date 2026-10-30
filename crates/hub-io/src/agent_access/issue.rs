//! Turns a contract SSH grant into an OpenSSH user certificate signed by the personal CA.

use nocterm_keys::agents::SshCaGrant;
use nocterm_ssh_keys::ca::{CertificateError, CertificateSpec, IssuedCertificate, UserCa};

/// Why a grant cannot be certified.
#[derive(Debug, thiserror::Error)]
pub enum IssueError {
    /// The grant names another CA than this account's.
    #[error("the grant was made for another CA")]
    OtherCa,
    /// The grant is not within what a certificate may carry.
    #[error("the grant is not valid")]
    Grant(#[source] nocterm_keys::agents::GrantError),
    /// The certificate could not be built.
    #[error("the certificate could not be issued")]
    Certificate(#[from] CertificateError),
}

/// A certificate for the subject key of `grant`, valid only as the grant says.
pub fn certify(
    ca: &UserCa,
    grant: &SshCaGrant,
    nonce: [u8; 32],
) -> Result<IssuedCertificate, IssueError> {
    if grant.ca_key != ca.public_key() {
        return Err(IssueError::OtherCa);
    }
    grant.validate().map_err(IssueError::Grant)?;
    Ok(ca.issue(
        &CertificateSpec {
            principal: grant.principal.clone(),
            subject_key: grant.subject_key,
            serial: grant.serial,
            valid_after_ms: grant.valid_after_ms,
            valid_before_ms: grant.valid_before_ms,
            source_addresses: grant.source_addresses.clone(),
        },
        nonce,
    )?)
}
