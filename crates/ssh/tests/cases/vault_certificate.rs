//! Signing in with a vault key and the user certificate that a user CA issued for it.
use super::*;
use nocterm_session::CredentialId;
use nocterm_ssh_keys::{
    KeyMaterial,
    ca::{CertificateSpec, UserCa},
};
use russh::keys::ssh_key::{PublicKey, public::KeyData};

const CREDENTIAL: CredentialId = CredentialId::from_bytes([7; 16]);
const PRINCIPAL: &str = "nocterm-agent-0123456789abcdef";

/// A server that trusts `ca` for the principal of every agent certificate.
fn trusting(ca: &UserCa) -> Option<Sshd> {
    let mut sshd = Sshd::start()?;
    let root = sshd.root().to_owned();
    fs::write(root.join("ca.pub"), ca.public_text().as_str()).unwrap();
    fs::write(root.join("principals"), format!("{PRINCIPAL}\n")).unwrap();
    // Only the certificate can sign in: no key of the fixture is authorized any more.
    fs::write(root.join("authorized_keys"), "").unwrap();
    let mut config = fs::read_to_string(root.join("sshd_config")).unwrap();
    config.push_str(&format!(
        "TrustedUserCAKeys {root}/ca.pub\nAuthorizedPrincipalsFile {root}/principals\n",
        root = root.display()
    ));
    fs::write(root.join("sshd_config"), config).unwrap();
    sshd.restart().unwrap();
    Some(sshd)
}

fn subject(sshd: &Sshd) -> KeyMaterial {
    let text = fs::read_to_string(sshd.root().join("plain/id_ed25519")).unwrap();
    KeyMaterial::import(&Secret::new(text), None).unwrap()
}

fn raw(key: &KeyMaterial) -> [u8; 32] {
    match PublicKey::from_openssh(key.public_key().as_str())
        .unwrap()
        .key_data()
    {
        KeyData::Ed25519(key) => key.0,
        other => panic!("not an Ed25519 key: {other:?}"),
    }
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

fn certificate(ca: &UserCa, key: &KeyMaterial, sources: &str) -> String {
    let spec = CertificateSpec {
        principal: PRINCIPAL.to_owned(),
        subject_key: raw(key),
        serial: 1,
        valid_after_ms: now_ms(),
        valid_before_ms: now_ms() + 3_600_000,
        source_addresses: vec![sources.to_owned()],
    };
    ca.issue(&spec, [3; 32]).unwrap().openssh().to_owned()
}

fn request(sshd: &Sshd, key: &KeyMaterial, certificate: String) -> ConnectRequest {
    let mut request = sshd.request();
    request.auth = Auth::VaultCertificate {
        credential: CREDENTIAL,
        public_key: key.public_key().as_str().to_owned(),
        certificate,
    };
    request
}

fn from_vault(answer: Secret) -> impl FnMut(Prompt) {
    move |prompt| match prompt {
        Prompt::Secret { reply, .. } => reply.send(Some(answer.clone())),
        other => accept(other),
    }
}

#[tokio::test]
async fn a_certificate_from_a_trusted_ca_signs_in() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = trusting(&ca) else { return };
    let key = subject(&sshd);
    let transport = sshd.transport("empty");
    let session = transport.open(request(&sshd, &key, certificate(&ca, &key, "127.0.0.1")));
    connect(&session, from_vault(key.to_secret())).await;
}

#[tokio::test]
async fn a_certificate_from_another_ca_is_refused() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = trusting(&ca) else { return };
    let key = subject(&sshd);
    let stranger = UserCa::from_seed([6; 32]).unwrap();
    let transport = sshd.transport("empty");
    let session = transport.open(request(
        &sshd,
        &key,
        certificate(&stranger, &key, "127.0.0.1"),
    ));
    let reason = closed(&session, from_vault(key.to_secret())).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}

#[tokio::test]
async fn a_certificate_limited_to_another_source_address_is_refused() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = trusting(&ca) else { return };
    let key = subject(&sshd);
    let transport = sshd.transport("empty");
    let session = transport.open(request(&sshd, &key, certificate(&ca, &key, "203.0.113.9")));
    let reason = closed(&session, from_vault(key.to_secret())).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}

#[tokio::test]
async fn a_certificate_for_another_key_is_never_offered() {
    let ca = UserCa::from_seed([5; 32]).unwrap();
    let Some(sshd) = trusting(&ca) else { return };
    let key = subject(&sshd);
    keygen(&sshd.root().join("other/id_ed25519"), "");
    let other = KeyMaterial::import(
        &Secret::new(fs::read_to_string(sshd.root().join("other/id_ed25519")).unwrap()),
        None,
    )
    .unwrap();
    // The certificate names `other`, the profile names `key`.
    let transport = sshd.transport("empty");
    let session = transport.open(request(&sshd, &key, certificate(&ca, &other, "127.0.0.1")));
    let reason = closed(&session, from_vault(key.to_secret())).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
}
