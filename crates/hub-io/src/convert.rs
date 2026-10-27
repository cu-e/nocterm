//! Records of the hub protocol as the records of the model.

use nocterm_hub_proto::v1;
use nocterm_hubs_model::{
    Account, AccountId, HubInfo, HubSessionId, HubSessionState, HubSessionSummary, NodeId,
    NodeSummary, OidcProvider, OrgId, OrgSummary,
};

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

/// Nodes that were removed are not shown.
pub(crate) fn nodes(list: v1::NodeList) -> Vec<NodeSummary> {
    list.nodes
        .into_iter()
        .filter(|node| !node.removed)
        .map(|node| NodeSummary {
            id: NodeId::new(node.node_id),
            name: node.name,
            online: node.online,
            sessions: node.sessions,
        })
        .collect()
}

pub(crate) fn sessions(list: v1::SessionList) -> Vec<HubSessionSummary> {
    list.sessions
        .into_iter()
        .map(|session| HubSessionSummary {
            state: state(session.state),
            id: HubSessionId::new(session.session_id),
            org: OrgId::new(session.org_id),
            node: NodeId::new(session.node_id),
            node_name: session.node_name,
            owner: AccountId::new(session.owner_account_id),
            title: session.title,
            node_online: session.node_online,
            created_at_ms: session.created_at_ms,
        })
        .collect()
}

/// A state this client does not know is shown as lost, which offers no attach.
fn state(wire: i32) -> HubSessionState {
    match v1::SessionState::try_from(wire) {
        Ok(v1::SessionState::Starting) => HubSessionState::Starting,
        Ok(v1::SessionState::Running) => HubSessionState::Running,
        Ok(v1::SessionState::Exited) => HubSessionState::Exited,
        Ok(v1::SessionState::Lost | v1::SessionState::Unspecified) | Err(_) => {
            HubSessionState::Lost
        }
    }
}

#[cfg(test)]
mod tests;
