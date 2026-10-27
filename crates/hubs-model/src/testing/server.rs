//! The data of one fake hub server.

use crate::address::HubAddress;
use crate::records::HubSessionId;
use crate::records::{
    Account, AccountId, HubInfo, HubSessionState, HubSessionSummary, NodeId, NodeSummary,
    OidcProvider, OrgId, OrgSummary,
};
use crate::settings::HubRecord;
use crate::testing::CLIENT_PROTOCOL;
use crate::trust::{Fingerprint, TrustPolicy};

/// A user who can sign in to a fake server.
#[derive(Clone, Debug)]
pub struct FakeUser {
    pub username: String,
    pub password: String,
    pub account: Account,
}

/// A server a [`crate::testing::FakeHub`] can reach. The fields are plain data a test edits.
#[derive(Clone, Debug)]
pub struct FakeServer {
    pub address: HubAddress,
    pub name: String,
    pub certificate: Fingerprint,
    /// A public authority vouches for the certificate.
    pub publicly_trusted: bool,
    /// The server answers; `false` makes every call unreachable.
    pub online: bool,
    /// The protocol version the server speaks.
    pub protocol: u32,
    pub bootstrapped: bool,
    pub password_login: bool,
    pub oidc_providers: Vec<OidcProvider>,
    pub users: Vec<FakeUser>,
    pub nodes: Vec<NodeSummary>,
    pub sessions: Vec<HubSessionSummary>,
}

impl FakeServer {
    /// A self-signed server named "Fake hub" whose certificate is `[seed; 32]`.
    pub fn new(address: &str, seed: u8) -> Self {
        Self {
            address: HubAddress::parse(address).expect("a valid fake address"),
            name: "Fake hub".into(),
            certificate: Fingerprint::from_bytes([seed; 32]),
            publicly_trusted: false,
            online: true,
            protocol: CLIENT_PROTOCOL,
            bootstrapped: true,
            password_login: true,
            oidc_providers: Vec::new(),
            users: Vec::new(),
            nodes: Vec::new(),
            sessions: Vec::new(),
        }
    }

    #[must_use]
    pub fn named(mut self, name: &str) -> Self {
        self.name = name.into();
        self
    }

    /// The settings record of this server as a sign-in leaves it: pinned to its certificate.
    pub fn pinned_record(&self, label: &str) -> HubRecord {
        HubRecord::new(
            self.address.clone(),
            TrustPolicy::Pinned(self.certificate),
            label,
        )
    }

    #[must_use]
    pub fn publicly_trusted(mut self) -> Self {
        self.publicly_trusted = true;
        self
    }

    #[must_use]
    pub fn with_provider(mut self, id: &str, display_name: &str) -> Self {
        self.oidc_providers.push(OidcProvider {
            id: id.into(),
            display_name: display_name.into(),
        });
        self
    }

    /// A user who belongs to the organization `org` (id and name are the same text).
    #[must_use]
    pub fn with_user(mut self, username: &str, password: &str, org: &str) -> Self {
        self.users.push(FakeUser {
            username: username.into(),
            password: password.into(),
            account: Account {
                id: AccountId::new(format!("account-{username}")),
                username: username.into(),
                display_name: username.to_uppercase(),
                orgs: vec![OrgSummary {
                    id: OrgId::new(org),
                    name: org.into(),
                }],
            },
        });
        self
    }

    /// Makes the existing user `username` a member of one more organization `org` (id and name
    /// are the same text); a user that does not exist is left alone.
    #[must_use]
    pub fn with_membership(mut self, username: &str, org: &str) -> Self {
        if let Some(user) = self.users.iter_mut().find(|user| user.username == username) {
            user.account.orgs.push(OrgSummary {
                id: OrgId::new(org),
                name: org.into(),
            });
        }
        self
    }

    #[must_use]
    pub fn with_node(mut self, id: &str, name: &str, online: bool) -> Self {
        self.nodes.push(NodeSummary {
            id: NodeId::new(id),
            name: name.into(),
            online,
            sessions: 0,
        });
        self
    }

    /// A running session of `owner` on node `node` of organization `org`.
    #[must_use]
    pub fn with_session(
        mut self,
        id: &str,
        org: &str,
        node: &str,
        owner: &str,
        title: &str,
    ) -> Self {
        let node_name = self
            .nodes
            .iter()
            .find(|candidate| candidate.id.as_str() == node)
            .map_or_else(|| node.to_owned(), |candidate| candidate.name.clone());
        self.sessions.push(HubSessionSummary {
            id: HubSessionId::new(id),
            org: OrgId::new(org),
            node: NodeId::new(node),
            node_name,
            owner: AccountId::new(format!("account-{owner}")),
            title: title.into(),
            state: HubSessionState::Running,
            node_online: true,
            created_at_ms: 0,
        });
        self
    }

    pub(super) fn info(&self, signed_in: bool) -> HubInfo {
        HubInfo {
            name: self.name.clone(),
            password_login: self.password_login,
            oidc_providers: self.oidc_providers.clone(),
            bootstrapped: self.bootstrapped,
            signed_in,
        }
    }
}
