//! Signing in with a key kept in the vault, against a real server.
use super::*;
use nocterm_session::CredentialId;
use nocterm_ssh_keys::KeyMaterial;

const CREDENTIAL: CredentialId = CredentialId::from_bytes([9; 16]);

/// The key of `directory` as the vault would hold it.
fn vault_entry(directory: &Path) -> KeyMaterial {
    let text = fs::read_to_string(directory.join("id_ed25519")).unwrap();
    KeyMaterial::import(&Secret::new(text), None).unwrap()
}

fn request_naming(sshd: &Sshd, key: &KeyMaterial) -> ConnectRequest {
    let mut request = sshd.request();
    request.auth = Auth::VaultKey {
        credential: CREDENTIAL,
        public_key: key.public_key().as_str().to_owned(),
    };
    request
}

/// Answers the vault key question with `answer` and records what was asked.
fn from_vault(answer: Option<Secret>, asked: &mut Vec<SecretRequest>) -> impl FnMut(Prompt) + '_ {
    move |prompt| match prompt {
        Prompt::Secret { request, reply, .. } => {
            asked.push(request);
            reply.send(answer.clone());
        }
        other => accept(other),
    }
}

#[tokio::test]
async fn a_key_from_the_vault_signs_in() {
    let Some(sshd) = Sshd::start() else { return };
    let key = vault_entry(&sshd.root().join("plain"));
    let transport = sshd.transport("empty");
    let session = transport.open(request_naming(&sshd, &key));
    let mut asked = Vec::new();
    connect(&session, from_vault(Some(key.to_secret()), &mut asked)).await;
    assert_eq!(asked.len(), 1);
    assert!(matches!(
        &asked[0],
        SecretRequest::PrivateKey { credential, public_key }
            if *credential == CREDENTIAL && public_key == key.public_key().as_str()
    ));
}

#[tokio::test]
async fn a_vault_record_with_another_public_half_is_not_signed_with() {
    let Some(sshd) = Sshd::start() else { return };
    let named = vault_entry(&sshd.root().join("plain"));
    // An authorized key is in the vault under the wrong record: the profile
    // still names `named`, so the answer must not be used.
    keygen(&sshd.root().join("other/id_ed25519"), "");
    let swapped = vault_entry(&sshd.root().join("other"));
    let transport = sshd.transport("empty");
    let session = transport.open(request_naming(&sshd, &named));
    let mut asked = Vec::new();
    let reason = closed(&session, from_vault(Some(swapped.to_secret()), &mut asked)).await;
    assert!(
        matches!(&reason, CloseReason::Failed(SessionError::Other(message)) if message.contains("vault key could not be offered")),
        "{reason:?}"
    );
}

#[tokio::test]
async fn a_missing_vault_key_ends_sign_in_without_other_questions() {
    let Some(sshd) = Sshd::start() else { return };
    let key = vault_entry(&sshd.root().join("plain"));
    let transport = sshd.transport("empty");
    let session = transport.open(request_naming(&sshd, &key));
    let mut asked = Vec::new();
    let reason = closed(&session, from_vault(None, &mut asked)).await;
    assert!(matches!(reason, CloseReason::Failed(_)), "{reason:?}");
    assert_eq!(asked.len(), 1, "{asked:?}");
}
