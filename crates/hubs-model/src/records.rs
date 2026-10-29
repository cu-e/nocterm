//! Plain data the hub access ports exchange. None of it carries a secret.

use std::fmt;

use crate::address::HubAddress;
use crate::trust::TrustPolicy;

macro_rules! text_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

text_id!(
    /// An account on a hub.
    AccountId
);
text_id!(
    /// An organization of a hub.
    OrgId
);

/// A hub and the trust decided for it; the adapter refuses a certificate that does not match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubEndpointSpec {
    pub address: HubAddress,
    pub trust: TrustPolicy,
}

/// An identity provider the hub offers for single sign-on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OidcProvider {
    pub id: String,
    pub display_name: String,
}

/// What a hub says of itself, and whether this device is signed in to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubInfo {
    pub name: String,
    /// The hub accepts a user name and password.
    pub password_login: bool,
    pub oidc_providers: Vec<OidcProvider>,
    /// The hub has an owner. A hub without one cannot be signed in to from this client.
    pub bootstrapped: bool,
    /// This device holds working credentials for the hub.
    pub signed_in: bool,
}

/// An organization an account belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgSummary {
    pub id: OrgId,
    pub name: String,
}

/// Who this device is signed in as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: AccountId,
    pub username: String,
    pub display_name: String,
    pub orgs: Vec<OrgSummary>,
}

/// What a sign-in leaves to do about the account's keys.
///
/// The keys protect what the account keeps on the hub (profiles, the vault); the hub never
/// holds them. They are opened with the login password, or with a vault passphrase for an
/// account that signs in through a provider.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyStatus {
    /// The keys are open: the account's data on the hub can be read.
    Open,
    /// The keys are open, but the recovery key made with them was never confirmed as saved
    /// (the dialog was closed or the client stopped before): a new one can be issued, because
    /// the open keys may replace the recovery envelope.
    RecoveryKeyUnconfirmed(crate::RecoveryKeyReissue),
    /// The account had no keys; they were made now. The recovery key is the only way back
    /// after the password is lost, so it must be shown once and kept nowhere by the client.
    Created(crate::RecoveryKeyOffer),
    /// A browser sign-in cannot open the keys: the user types the vault passphrase.
    PassphraseNeeded,
    /// The keys exist but the password does not open them (it was reset): the recovery key
    /// does.
    RecoveryNeeded,
}

/// A finished sign-in.
#[derive(Clone, Debug, PartialEq)]
pub struct SignedIn {
    pub account: Account,
    pub keys: KeyStatus,
    /// Original login authority for delayed publication or conditional cleanup.
    pub login: crate::LoginReceipt,
}
