//! The credential vault of this device and the credential provider a terminal installation
//! binds to a vault and the connections of its source.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::App;
use nocterm_vault::VaultAccess;
use nocterm_vault_store::VaultService;

use super::VaultSetup;

pub(super) struct OpenedVault {
    pub(super) installed: Rc<nocterm_vault_ui::Installed>,
    pub(super) credentials: nocterm_terminal::CredentialProvider,
}

/// Opens the vault service of this device, or none when it cannot be initialised.
pub(super) fn open_vault(
    setup: VaultSetup,
    ready: &nocterm_ui::UiReady,
    cx: &mut App,
) -> Option<Arc<dyn VaultAccess>> {
    let device = setup.device_unlock.then(nocterm_device_unlock::provider);
    ready
        .settings()
        .register::<nocterm_vault_ui::VaultSettings>(cx);
    let duration = std::time::Duration::from_secs(
        u64::from(
            ready
                .settings()
                .setting::<nocterm_vault_ui::VaultSettings>(cx)
                .auto_lock_minutes,
        ) * 60,
    );
    match VaultService::new_with_device_unlock(setup.file, duration, device) {
        Ok(vault) => Some(Arc::new(vault)),
        Err(error) => {
            tracing::error!(%error, "could not initialize the credential vault");
            None
        }
    }
}

/// Lets terminals keep credentials in the vault and shows it in Settings.
pub(super) fn install_vault(
    vault: Arc<dyn VaultAccess>,
    ready: &nocterm_ui::UiReady,
    connections: Option<gpui_kit::Entity<nocterm_connections::Connections>>,
    cx: &mut App,
) -> OpenedVault {
    let credentials = credential_provider(vault.clone(), connections);
    OpenedVault {
        installed: Rc::new(nocterm_vault_ui::install(
            nocterm_vault_ui::Deps::new(vault),
            ready,
            cx,
        )),
        credentials,
    }
}

/// The credential candidates of a terminal session and the association of a typed secret
/// with the connection it was typed for, both through `vault` and the connections of one
/// source.
pub(crate) fn credential_provider(
    vault: Arc<dyn VaultAccess>,
    connections: Option<gpui_kit::Entity<nocterm_connections::Connections>>,
) -> nocterm_terminal::CredentialProvider {
    nocterm_terminal::CredentialProvider::new(
        Arc::new(nocterm_vault_ui::VaultCredentials::new(vault)),
        move |spec, id, cx| {
            let Some(connections) = connections.as_ref() else {
                return gpui_kit::Task::ready(Err(
                    nocterm_terminal::CredentialAssociationError::Unavailable,
                ));
            };
            // A secret remembered for a machine behind jump hosts binds only to
            // the connection it was typed for, never to one that merely shares its address.
            let profile = spec
                .profile
                .as_deref()
                .and_then(|profile| profile.parse().ok());
            let Some(scope) =
                nocterm_connections_model::credential_scope::CredentialScope::of_session(
                    spec.target.clone(),
                    spec.auth.clone(),
                    profile,
                    !spec.jumps.is_empty(),
                )
            else {
                return gpui_kit::Task::ready(Ok(()));
            };
            let association = connections.update(cx, |connections, cx| {
                connections.associate_credential(scope, id, cx)
            });
            cx.spawn(async move |_| {
                association
                    .await
                    .map_err(nocterm_terminal::CredentialAssociationError::binding)
            })
        },
    )
}
