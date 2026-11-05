//! Reads a device's OpenSSH user certificate for a chat's SSH binding (ADR 0145).
use nocterm_cloud_model::{
    identity::Timestamp,
    ssh_access::{CertificateFacts, SshAccessError, SshCertificateReader},
};
use nocterm_keys::agents::ssh_ca;

/// Checks that the binding's CA signed the blob as an agent user certificate with only the
/// allowed restrictions, and returns what it states. The runner compares the facts.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunnerSshCertificates;
impl SshCertificateReader for RunnerSshCertificates {
    fn read(
        &self,
        certificate: &[u8],
        ca_key: &[u8; 32],
    ) -> Result<CertificateFacts, SshAccessError> {
        let facts = ssh_ca::CertificateFacts::read(certificate, ca_key).map_err(|error| {
            tracing::debug!(%error, "a chat SSH certificate was refused");
            SshAccessError::Invalid
        })?;
        let millis = |seconds: u64| {
            seconds
                .checked_mul(1000)
                .map(Timestamp::from_millis)
                .ok_or(SshAccessError::Validity)
        };
        Ok(CertificateFacts {
            subject_key: facts.subject_key,
            principals: facts.principals,
            valid_after: millis(facts.valid_after)?,
            valid_before: millis(facts.valid_before)?,
        })
    }
}
