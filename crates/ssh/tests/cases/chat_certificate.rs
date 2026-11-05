//! A runner signs in with a chat's in-memory key and the certificate a device issued for
//! one bound server's own principal (ADR 0137).
use super::*;
use nocterm_ssh::{CertificateLogin, CertificateLoginError};
use nocterm_ssh_keys::{
    PublicKeyText, ed25519_public_key,
    testing::ca::{CertificateSpec, UserCa},
};

const SEED: [u8; 32] = [11; 32];
/// `nocterm-agent-<ca>-<server>` of the server under test, and of another one.
const PRINCIPAL: &str = "nocterm-agent-0123456789abcdef-a1a1a1a1a1a1a1a1";
const OTHER_SERVER: &str = "nocterm-agent-0123456789abcdef-b2b2b2b2b2b2b2b2";

/// A server whose only way in is a certificate of `ca` naming this server's principal,
/// granted by a per-server `cert-authority` line in `authorized_keys`.
fn per_server(ca: &UserCa) -> Option<Sshd> {
    let mut sshd = Sshd::start()?;
    let line = format!(
        "cert-authority,principals=\"{PRINCIPAL}\" {}\n",
        ca.public_text().as_str()
    );
    fs::write(sshd.root().join("authorized_keys"), line).unwrap();
    sshd.restart().unwrap();
    Some(sshd)
}
fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}
fn certificate(ca: &UserCa, principal: &str, subject: [u8; 32]) -> Vec<u8> {
    let spec = CertificateSpec {
        principal: principal.to_owned(),
        subject_key: subject,
        serial: 2,
        valid_after_ms: now_ms() - 60_000,
        valid_before_ms: now_ms() + 3_600_000,
        source_addresses: vec!["127.0.0.1".to_owned()],
    };
    let text = ca.issue(&spec, [4; 32]).unwrap().openssh().to_owned();
    russh::keys::Certificate::from_openssh(&text)
        .unwrap()
        .to_bytes()
        .unwrap()
}
fn host(sshd: &Sshd) -> PublicKeyText {
    PublicKeyText::parse(&fs::read_to_string(sshd.root().join("host_key.pub")).unwrap()).unwrap()
}
fn login(sshd: &Sshd, blob: &[u8]) -> CertificateLogin {
    CertificateLogin::new(sshd.request().target, &host(sshd), &SEED, blob).unwrap()
}

#[tokio::test]
async fn a_chat_key_certified_for_this_servers_principal_signs_in_without_prompts() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = per_server(&ca) else { return };
    let blob = certificate(&ca, PRINCIPAL, ed25519_public_key(&SEED));
    let transport = sshd.transport("empty");
    let session = transport.open_certificate(
        sshd.request(),
        nocterm_session::ConnectionOpenOptions::default(),
        login(&sshd, &blob),
    );
    connect(&session, no_prompts).await;
    session.close();
}

#[tokio::test]
async fn a_certificate_for_another_bound_server_is_refused_by_this_server() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = per_server(&ca) else { return };
    let blob = certificate(&ca, OTHER_SERVER, ed25519_public_key(&SEED));
    let transport = sshd.transport("empty");
    let session = transport.open_certificate(
        sshd.request(),
        nocterm_session::ConnectionOpenOptions::default(),
        login(&sshd, &blob),
    );
    let reason = closed(&session, no_prompts).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}

#[tokio::test]
async fn a_host_that_is_not_the_pinned_one_is_never_signed_in_to() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = per_server(&ca) else { return };
    let blob = certificate(&ca, PRINCIPAL, ed25519_public_key(&SEED));
    let stranger = nocterm_ssh_keys::KeyMaterial::ed25519_from_seed(&[9; 32], "other").unwrap();
    let pinned =
        CertificateLogin::new(sshd.request().target, &stranger.public_key(), &SEED, &blob).unwrap();
    let transport = sshd.transport("empty");
    let session = transport.open_certificate(
        sshd.request(),
        nocterm_session::ConnectionOpenOptions::default(),
        pinned,
    );
    let reason = closed(&session, no_prompts).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}

#[test]
fn a_certificate_for_another_key_is_refused_before_connecting() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let blob = certificate(&ca, PRINCIPAL, ed25519_public_key(&[12; 32]));
    let host = nocterm_ssh_keys::KeyMaterial::ed25519_from_seed(&[9; 32], "host").unwrap();
    let target = Target::new("deploy", "127.0.0.1", 22);
    assert!(matches!(
        CertificateLogin::new(target, &host.public_key(), &SEED, &blob),
        Err(CertificateLoginError::OtherKey)
    ));
}
