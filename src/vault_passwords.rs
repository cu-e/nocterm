//! Passwords of the vault: the connection editor's [`SavedPasswords`] over the vault of a
//! window, this computer's or the account's on a hub.
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_connections::{PasswordNotKept, SavedPassword, SavedPasswords, VaultPlace};
use nocterm_session::{CredentialId, Secret, Target};
use nocterm_vault::{CredentialBinding, CredentialInfo, VaultAccess, VaultError};
use std::sync::Arc;

/// The passwords of one vault.
pub(crate) struct VaultPasswords {
    vault: Arc<dyn VaultAccess>,
    place: VaultPlace,
}

/// The passwords of `vault`, which is the vault `place` names, for the connection editor.
pub(crate) fn passwords_of(
    vault: &Arc<dyn VaultAccess>,
    place: VaultPlace,
) -> Arc<dyn SavedPasswords> {
    Arc::new(VaultPasswords {
        vault: vault.clone(),
        place,
    })
}

impl SavedPasswords for VaultPasswords {
    fn place(&self) -> VaultPlace {
        self.place
    }

    fn list(&self) -> BoxFuture<'static, Result<Vec<SavedPassword>, PasswordNotKept>> {
        let listed = self.vault.list();
        async move {
            Ok(listed
                .await
                .map_err(not_kept)?
                .into_iter()
                .filter_map(|info| {
                    let CredentialBinding::Password { targets } = &info.binding else {
                        return None;
                    };
                    Some(SavedPassword {
                        servers: targets.len(),
                        id: info.id,
                        label: info.label,
                    })
                })
                .collect())
        }
        .boxed()
    }

    fn keep(
        &self,
        replacing: Option<CredentialId>,
        label: String,
        target: Target,
        password: Secret,
    ) -> BoxFuture<'static, Result<CredentialId, PasswordNotKept>> {
        if !self.vault.is_unlocked() {
            return futures::future::ready(Err(PasswordNotKept::Locked)).boxed();
        }
        let vault = self.vault.clone();
        async move {
            // A saved password keeps its name and the servers it signs in to; a new one signs
            // in to this server only.
            let Some(info) = password_record(vault.as_ref(), replacing).await? else {
                let binding = CredentialBinding::password([target]);
                return vault
                    .put(None, label, binding, password)
                    .await
                    .map_err(not_kept);
            };
            let binding = gained(&info.binding, target);
            if binding != info.binding {
                vault
                    .update(info.id, info.label.clone(), binding.clone())
                    .await
                    .map_err(not_kept)?;
            }
            vault
                .put(Some(info.id), info.label, binding, password)
                .await
                .map_err(not_kept)
        }
        .boxed()
    }

    fn attach(
        &self,
        id: CredentialId,
        target: Target,
    ) -> BoxFuture<'static, Result<(), PasswordNotKept>> {
        if !self.vault.is_unlocked() {
            return futures::future::ready(Err(PasswordNotKept::Locked)).boxed();
        }
        let vault = self.vault.clone();
        async move {
            let info = password_record(vault.as_ref(), Some(id))
                .await?
                .ok_or(PasswordNotKept::Missing)?;
            let binding = gained(&info.binding, target);
            if binding == info.binding {
                return Ok(());
            }
            vault
                .update(id, info.label, binding)
                .await
                .map_err(not_kept)
        }
        .boxed()
    }
}

/// The password record `id`, if the vault keeps one.
async fn password_record(
    vault: &dyn VaultAccess,
    id: Option<CredentialId>,
) -> Result<Option<CredentialInfo>, PasswordNotKept> {
    let Some(id) = id else {
        return Ok(None);
    };
    Ok(vault
        .list()
        .await
        .map_err(not_kept)?
        .into_iter()
        .find(|info| info.id == id && matches!(info.binding, CredentialBinding::Password { .. })))
}

/// `binding` signing in to `target` as well.
fn gained(binding: &CredentialBinding, target: Target) -> CredentialBinding {
    CredentialBinding::password(binding.targets().iter().cloned().chain([target]))
}

fn not_kept(error: VaultError) -> PasswordNotKept {
    match error {
        VaultError::Locked => PasswordNotKept::Locked,
        VaultError::NotFound => PasswordNotKept::Missing,
        other => PasswordNotKept::Vault(Box::new(other)),
    }
}

#[cfg(test)]
mod tests;
