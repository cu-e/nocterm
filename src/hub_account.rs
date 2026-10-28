//! Keeps this device's profiles and vault in step with the hub account the user chose.
//!
//! The choice is the `[hubs] account` setting. While a server is chosen, the saved profiles
//! replicate to and from its account and the vault is the account's; without one, profiles stay
//! local and the vault is the file of this device. Nothing restarts when the choice changes:
//! this entity follows the registry of servers and moves the two consumers.
//!
//! Stopping (unchecking the account, removing the server) never takes a profile off the device.
//! Only choosing another server removes what the leaving account agreed to and the user did not
//! change since, so it is not offered to the other account; an edited profile stays and goes to
//! the next account's personal space.
use gpui_kit::{AppContext as _, Context, Entity, Subscription};
use nocterm_connections_model::{
    profiles::Profile,
    replica::{ProfileReplica, ReplicaScope, ReplicaState},
};
use nocterm_connections_store::{ConnectionStore, Departure, ProfileReplication, ReplicationEvent};
use nocterm_core::Paths;
use nocterm_hubs::Hubs;
use nocterm_hubs_model::{HubError, HubId};
use nocterm_store::{ShutdownCoordinator, SnapshotWriter, StoreError};
use nocterm_vault::VaultAccess;
use std::sync::Arc;

use crate::vault_selection::SelectedVault;

/// The encrypted data of one signed-in hub account.
pub(crate) struct AccountAccess {
    pub(crate) replica: Arc<dyn ProfileReplica>,
    pub(crate) vault: Arc<dyn VaultAccess>,
}

/// What the application needs to reach a hub's account.
pub(crate) trait HubAccounts: Send + Sync {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError>;
}

impl HubAccounts for nocterm_hub_io::HubLinks {
    fn open(&self, hub: &HubId) -> Result<AccountAccess, HubError> {
        let account = self.open_account(hub)?;
        Ok(AccountAccess {
            replica: account.replica(),
            vault: account.vault(),
        })
    }
}

/// Follows the chosen account server.
pub(crate) struct HubAccountSync {
    hubs: Entity<Hubs>,
    accounts: Arc<dyn HubAccounts>,
    vault: Option<SelectedVault>,
    replication: Entity<ProfileReplication>,
    /// The server whose account is in use now.
    attached: Option<HubId>,
    _hubs: Subscription,
    _refusals: Subscription,
}

impl HubAccountSync {
    pub(crate) fn install(
        hubs: Entity<Hubs>,
        accounts: Arc<dyn HubAccounts>,
        vault: Option<SelectedVault>,
        replication: Entity<ProfileReplication>,
        cx: &mut gpui_kit::App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(hubs, accounts, vault, replication, cx))
    }

    fn new(
        hubs: Entity<Hubs>,
        accounts: Arc<dyn HubAccounts>,
        vault: Option<SelectedVault>,
        replication: Entity<ProfileReplication>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observed = cx.observe(&hubs, |this, _, cx| this.follow(cx));
        let refusals = cx.subscribe(&replication, |_, replication, event, cx| {
            if let ReplicationEvent::Refused {
                profile,
                scope,
                restored,
            } = event
            {
                let space = space_name(&replication.read(cx).spaces(), scope);
                let message = refusal_message(profile, &space, scope, *restored);
                gpui_kit::App::defer(cx, move |cx| notify(message, cx));
            }
        });
        let mut sync = Self {
            hubs,
            accounts,
            vault,
            replication,
            attached: None,
            _hubs: observed,
            _refusals: refusals,
        };
        sync.follow(cx);
        sync
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        // The setting only names a server that is still listed.
        let wanted = self.hubs.read(cx).account_hub(cx);
        if wanted == self.attached {
            return;
        }
        let next = wanted.and_then(|hub| match self.accounts.open(&hub) {
            Ok(account) => Some((hub, account)),
            Err(error) => {
                tracing::error!(%error, "the account of the chosen server cannot be opened");
                None
            }
        });
        if self.attached.take().is_some() {
            // Profiles leave the device only for the sake of another account that is
            // really taking over; stopping, removing the server or failing to reach the
            // next one leaves everything here.
            let departure = if next.is_some() {
                Departure::Switched
            } else {
                Departure::Stopped
            };
            self.leave(departure, cx);
        }
        let Some((hub, account)) = next else {
            return;
        };
        if let Some(vault) = &self.vault {
            vault.select(account.vault);
        }
        self.replication.update(cx, |replication, cx| {
            replication.attach(account.replica, cx)
        });
        self.attached = Some(hub);
    }

    /// Back to this device's own profiles and vault.
    fn leave(&mut self, departure: Departure, cx: &mut Context<Self>) {
        self.replication
            .update(cx, |replication, cx| replication.detach(departure, cx));
        if let Some(vault) = &self.vault {
            vault.select_home();
        }
    }
}

/// The agreed state of the profile replication, kept in a private file of `paths` when the
/// application keeps files, and registered for the orderly shutdown.
pub(crate) fn replication(
    store: Entity<ConnectionStore>,
    stored: Option<&Paths>,
    shutdown: &ShutdownCoordinator,
    cx: &mut gpui_kit::App,
) -> Entity<ProfileReplication> {
    let file = stored.map(|paths| {
        Arc::new(nocterm_connections_io::FileReplicaState::new(
            paths.state_dir().join("hub-replica.json"),
        ))
    });
    let initial = file.as_ref().map(|file| file.load()).unwrap_or_default();
    let writer: SnapshotWriter<ReplicaState> = Arc::new(move |state| {
        let file = file.clone();
        Box::pin(async move {
            if let Some(file) = file {
                file.save(&state).map_err(StoreError::backend)?;
            }
            Ok(())
        })
    });
    let document = cx.new(|_| ProfileReplication::state_document(initial, writer));
    if let Err(error) = shutdown.register(&document) {
        tracing::error!(%error, "shutdown registration failed");
    }
    cx.new(|cx| ProfileReplication::new(store, document, cx))
}

/// What the user calls the space a refusal came from.
fn space_name(
    spaces: &[nocterm_connections_model::replica::ReplicaSpace],
    scope: &ReplicaScope,
) -> String {
    spaces
        .iter()
        .find(|space| &space.scope == scope)
        .map_or_else(|| "the organization".to_owned(), |space| space.name.clone())
}

/// Tells the user, once for the refusal, that the hub did not let them change a connection and
/// where it is now.
pub(crate) fn refusal_message(
    profile: &Profile,
    space: &str,
    scope: &ReplicaScope,
    restored: bool,
) -> String {
    let name = &profile.name;
    match (restored, scope) {
        (true, _) => format!(
            "You may not change \u{201c}{name}\u{201d} in {space}. The shared version was restored."
        ),
        (false, ReplicaScope::Organization(_)) => format!(
            "You may not add \u{201c}{name}\u{201d} to {space}. It stays in your personal space."
        ),
        (false, ReplicaScope::Personal) => {
            format!("The server refused \u{201c}{name}\u{201d}. It stays on this device only.")
        }
    }
}

/// Shows `message` in the window the user is working in; with no window there is no one to tell.
fn notify(message: String, cx: &mut gpui_kit::App) {
    tracing::warn!(%message, "the hub refused a change to a connection");
    let Some(handle) = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next())
    else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        nocterm_ui::notice::warning(
            window,
            cx,
            "connections-refused",
            "Connection not changed",
            message,
        );
    });
}
