//! Records of the hub protocol as the records of the model.

use nocterm_hub_proto::v2;
use nocterm_hubs_model::{Account, AccountId, HubInfo, OidcProvider};

pub(crate) fn hub_info(info: &v2::HubInfo, signed_in: bool) -> HubInfo {
    HubInfo {
        name: info.name.clone(),
        password_login: info.password_login,
        oidc_providers: info
            .oidc_providers
            .iter()
            .map(|provider| OidcProvider {
                id: provider.id.clone(),
                display_name: provider.display_name.clone(),
            })
            .collect(),
        bootstrapped: !info.needs_setup,
        signed_in,
    }
}

pub(crate) fn account(me: v2::Me) -> Account {
    Account {
        id: AccountId::new(me.account_id),
        username: me.username,
        display_name: me.display_name,
        admin: me.admin,
    }
}

#[cfg(test)]
mod tests;
