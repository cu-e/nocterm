//! Records of the hub protocol as the records of the model.

use nocterm_hub_proto::v1;
use nocterm_hubs_model::{Account, AccountId, HubInfo, OidcProvider, OrgId, OrgSummary};

pub(crate) fn hub_info(info: &v1::HubInfo, signed_in: bool) -> HubInfo {
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
        bootstrapped: info.bootstrapped,
        signed_in,
    }
}

pub(crate) fn account(me: v1::Me) -> Account {
    Account {
        id: AccountId::new(me.account_id),
        username: me.username,
        display_name: me.display_name,
        orgs: me
            .memberships
            .into_iter()
            .filter_map(|membership| membership.org)
            .map(|org| OrgSummary {
                id: OrgId::new(org.org_id),
                name: org.name,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
