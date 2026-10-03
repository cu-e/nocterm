//! Signing in: the agent, key files, then whatever the user can type.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
};

use nocterm_session::{
    Auth, ConnectRequest, Prompt, Secret, SecretRequest, SessionDriver, SessionError,
};
use russh::{
    MethodKind, MethodSet,
    client::{AuthResult, Handle, KeyboardInteractiveAuthResponse},
    keys::{
        Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey, decode_secret_key,
        ssh_key::public::KeyData,
    },
};

use crate::{SshConfig, connection::Client};
use zeroize::Zeroizing;

const MAX_PRIVATE_KEY: usize = 1024 * 1024;
const MAX_PUBLIC_KEY: usize = 64 * 1024;

/// Private keys tried when the connection names none, most modern first.
const DEFAULT_IDENTITIES: [&str; 3] = ["id_ed25519", "id_ecdsa", "id_rsa"];

/// Keys offered before giving up on them. Hosts count every offer as a
/// failed attempt and hang up after a handful (OpenSSH allows six), which
/// must leave room for a password.
const MAX_KEY_OFFERS: usize = 4;

/// Times the user may mistype one secret.
const MAX_SECRET_ATTEMPTS: usize = 3;

pub(crate) async fn authenticate(
    handle: &mut Handle<Client>,
    request: &ConnectRequest,
    config: &SshConfig,
    driver: &SessionDriver,
) -> Result<(), SessionError> {
    let mut authenticator = Authenticator {
        handle,
        request,
        driver,
        methods: MethodSet::empty(),
        offered: Vec::new(),
    };
    authenticator.run(config).await
}

struct Authenticator<'a> {
    handle: &'a mut Handle<Client>,
    request: &'a ConnectRequest,
    driver: &'a SessionDriver,
    /// The methods the host will still accept.
    methods: MethodSet,
    /// Public keys the host has already turned down.
    offered: Vec<KeyData>,
}

impl<'a> Authenticator<'a> {
    async fn run(&mut self, config: &SshConfig) -> Result<(), SessionError> {
        // Asking for no authentication is how a client learns what the host
        // accepts; a few hosts really do let the user in right here.
        let probe = self.handle.authenticate_none(self.user()).await;
        if self.succeeded(probe)? {
            return Ok(());
        }

        let signed_in = match &self.request.auth {
            Auth::Auto => self.try_agent(config).await? || self.try_default_keys(config).await?,
            Auth::Key { path } => self.try_key_file(path, true).await?,
            Auth::Password => false,
        };
        if signed_in || self.try_typed_secrets().await? {
            return Ok(());
        }

        Err(SessionError::AuthenticationFailed {
            user: self.user().to_owned(),
            host: self.request.target.host.clone(),
        })
    }

    fn user(&self) -> &'a str {
        let request: &'a ConnectRequest = self.request;
        &request.target.user
    }

    fn offers(&self, method: MethodKind) -> bool {
        self.methods.contains(&method)
    }

    fn may_offer_key(&self) -> bool {
        self.offers(MethodKind::PublicKey) && self.offered.len() < MAX_KEY_OFFERS
    }

    /// Whether an attempt signed the user in; a failed one tells which
    /// methods are left.
    fn succeeded(
        &mut self,
        result: Result<AuthResult, russh::Error>,
    ) -> Result<bool, SessionError> {
        match result.map_err(lost)? {
            AuthResult::Success => Ok(true),
            AuthResult::Failure {
                remaining_methods, ..
            } => {
                self.methods = remaining_methods;
                Ok(false)
            }
        }
    }

    /// The hash an RSA key signs with: the strongest one the host supports.
    async fn rsa_hash(&self, algorithm: &Algorithm) -> Option<HashAlg> {
        if !matches!(algorithm, Algorithm::Rsa { .. }) {
            return None;
        }
        self.handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten()
    }

    #[cfg(unix)]
    async fn try_agent(&mut self, config: &SshConfig) -> Result<bool, SessionError> {
        use russh::keys::agent::{AgentIdentity, client::AgentClient};

        if !config.use_agent || !self.offers(MethodKind::PublicKey) {
            return Ok(false);
        }
        // No agent is the common case, not an error.
        let Ok(mut agent) = AgentClient::connect_env().await else {
            return Ok(false);
        };
        let identities = match agent.request_identities().await {
            Ok(identities) => identities,
            Err(error) => {
                tracing::debug!(%error, "the SSH agent did not list its keys");
                return Ok(false);
            }
        };

        for identity in identities {
            let AgentIdentity::PublicKey { key, .. } = identity else {
                continue;
            };
            if !self.may_offer_key() {
                break;
            }

            let hash = self.rsa_hash(&key.algorithm()).await;
            self.offered.push(key.key_data().clone());
            let user = self.user().to_owned();
            match self
                .handle
                .authenticate_publickey_with(user, key, hash, &mut agent)
                .await
            {
                Ok(AuthResult::Success) => return Ok(true),
                Ok(AuthResult::Failure {
                    remaining_methods, ..
                }) => self.methods = remaining_methods,
                // The agent may refuse to sign (a locked or removed key).
                Err(error) => tracing::debug!(%error, "the SSH agent could not sign"),
            }
        }
        Ok(false)
    }

    #[cfg(not(unix))]
    async fn try_agent(&mut self, _config: &SshConfig) -> Result<bool, SessionError> {
        Ok(false)
    }

    async fn try_default_keys(&mut self, config: &SshConfig) -> Result<bool, SessionError> {
        let Some(directory) = &config.identity_dir else {
            return Ok(false);
        };
        for name in DEFAULT_IDENTITIES {
            if self.try_key_file(&directory.join(name), false).await? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Offers one private key file. `explicit` is set for a key the user
    /// chose, whose absence is worth a warning.
    async fn try_key_file(&mut self, path: &Path, explicit: bool) -> Result<bool, SessionError> {
        if !self.may_offer_key() {
            return Ok(false);
        }
        let text = match read_key(path.to_owned(), MAX_PRIVATE_KEY).await {
            Ok(text) => text,
            Err(error) => {
                if explicit {
                    tracing::warn!(key = %path.display(), %error, "cannot read the private key");
                }
                return Ok(false);
            }
        };

        // The public half, when it lies next to the key, tells whether the
        // host has already refused it, before a passphrase is asked for.
        if let Some(public) = public_half(path).await
            && self.offered.contains(public.key_data())
        {
            return Ok(false);
        }
        let Some(key) = self.unlock(path, text).await else {
            return Ok(false);
        };
        let public = key.public_key().key_data().clone();
        if self.offered.contains(&public) {
            return Ok(false);
        }

        let hash = self.rsa_hash(&key.algorithm()).await;
        self.offered.push(public);
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        let result = self.handle.authenticate_publickey(self.user(), key).await;
        self.succeeded(result)
    }

    /// Decodes a private key, asking for its passphrase if it has one.
    /// `None` means the key cannot be used.
    async fn unlock(&self, path: &Path, text: Arc<Zeroizing<Vec<u8>>>) -> Option<PrivateKey> {
        match decode(text.clone(), None).await {
            Ok(key) => return Some(key),
            Err(russh::keys::Error::KeyIsEncrypted) => {}
            Err(error) => {
                tracing::warn!(key = %path.display(), %error, "unusable private key");
                return None;
            }
        }

        for attempt in 0..MAX_SECRET_ATTEMPTS {
            // Dismissing the prompt skips this key, not the whole sign-in.
            let passphrase = self
                .ask(SecretRequest::KeyPassphrase {
                    path: path.to_owned(),
                    retry: attempt > 0,
                })
                .await?;
            if let Ok(key) = decode(text.clone(), Some(passphrase)).await {
                return Some(key);
            }
        }
        None
    }

    /// The methods that need the user at the keyboard.
    async fn try_typed_secrets(&mut self) -> Result<bool, SessionError> {
        let mut passwords = 0;
        let mut conversations = 0;

        loop {
            if self.offers(MethodKind::Password) && passwords < MAX_SECRET_ATTEMPTS {
                let request = SecretRequest::Password {
                    target: self.request.target.clone(),
                    retry: passwords > 0,
                };
                passwords += 1;
                let password = self.ask(request).await.ok_or(SessionError::Cancelled)?;
                let result = self
                    .handle
                    .authenticate_password(self.user(), password.expose())
                    .await;
                if self.succeeded(result)? {
                    return Ok(true);
                }
            } else if self.offers(MethodKind::KeyboardInteractive)
                && conversations < MAX_SECRET_ATTEMPTS
            {
                conversations += 1;
                if self.converse().await? {
                    return Ok(true);
                }
            } else {
                return Ok(false);
            }
        }
    }

    /// Keyboard-interactive: the host asks questions in its own words until
    /// it is satisfied (one-time codes, PAM password prompts).
    async fn converse(&mut self) -> Result<bool, SessionError> {
        let mut response = self
            .handle
            .authenticate_keyboard_interactive_start(self.user(), None)
            .await
            .map_err(lost)?;

        loop {
            match response {
                KeyboardInteractiveAuthResponse::Success => return Ok(true),
                KeyboardInteractiveAuthResponse::Failure {
                    remaining_methods, ..
                } => {
                    self.methods = remaining_methods;
                    return Ok(false);
                }
                KeyboardInteractiveAuthResponse::InfoRequest {
                    instructions,
                    prompts,
                    ..
                } => {
                    let mut answers = Vec::with_capacity(prompts.len());
                    for prompt in prompts {
                        let request = SecretRequest::Interactive {
                            prompt: if prompt.prompt.trim().is_empty() {
                                instructions.clone()
                            } else {
                                prompt.prompt
                            },
                            echo: prompt.echo,
                        };
                        let answer = self.ask(request).await.ok_or(SessionError::Cancelled)?;
                        answers.push(answer.expose().to_owned());
                    }
                    response = self
                        .handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(lost)?;
                }
            }
        }
    }

    async fn ask(&self, request: SecretRequest) -> Option<Secret> {
        self.driver
            .ask(|reply| Prompt::Secret { request, reply })
            .await
            .flatten()
    }
}

/// Decoding runs a deliberately slow key derivation for encrypted keys, so
/// it stays off the threads that serve the network.
async fn decode(
    text: Arc<Zeroizing<Vec<u8>>>,
    passphrase: Option<Secret>,
) -> Result<PrivateKey, russh::keys::Error> {
    tokio::task::spawn_blocking(move || {
        let text = std::str::from_utf8(&text).map_err(|_| russh::keys::Error::CouldNotReadKey)?;
        decode_secret_key(text, passphrase.as_ref().map(Secret::expose))
    })
    .await
    .unwrap_or(Err(russh::keys::Error::CouldNotReadKey))
}

async fn read_key(path: PathBuf, limit: usize) -> std::io::Result<Arc<Zeroizing<Vec<u8>>>> {
    tokio::task::spawn_blocking(move || crate::file::read(&path, limit).map(Arc::new))
        .await
        .map_err(std::io::Error::other)?
}

/// The public key stored next to a private key as `<name>.pub`.
async fn public_half(private: &Path) -> Option<PublicKey> {
    let mut path = OsString::from(private);
    path.push(".pub");
    let bytes = read_key(PathBuf::from(path), MAX_PUBLIC_KEY).await.ok()?;
    let text = std::str::from_utf8(&bytes).ok()?;
    PublicKey::from_openssh(text).ok()
}

fn lost(error: russh::Error) -> SessionError {
    SessionError::ConnectionLost(error.to_string())
}

#[cfg(test)]
mod material_tests {
    use super::*;
    #[tokio::test]
    async fn private_and_public_material_limits_are_enforced_before_decode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("key");
        std::fs::write(&path, vec![b'x'; MAX_PRIVATE_KEY + 1]).unwrap();
        assert!(read_key(path.clone(), MAX_PRIVATE_KEY).await.is_err());
        std::fs::write(&path, vec![b'x'; MAX_PUBLIC_KEY + 1]).unwrap();
        assert!(read_key(path.clone(), MAX_PUBLIC_KEY).await.is_err());
        std::fs::write(&path, b"not a private key").unwrap();
        let bytes = read_key(path, MAX_PRIVATE_KEY).await.unwrap();
        assert!(decode(bytes.clone(), None).await.is_err());
        assert_eq!(Arc::strong_count(&bytes), 1);
    }
}
