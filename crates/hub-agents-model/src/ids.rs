//! Identifiers the hub gives its agents, providers and servers.

use std::fmt;

/// The longest identifier this client accepts from a hub.
pub const MAX_ID_BYTES: usize = 128;

/// An identifier a hub sent was empty, too long or not plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a hub identifier holds 1 to 128 visible ASCII characters")]
pub struct IdError;

macro_rules! hub_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Checks the shape only; the hub decides whether it names anything.
            pub fn new(id: impl Into<String>) -> Result<Self, IdError> {
                let id = id.into();
                if id.is_empty()
                    || id.len() > MAX_ID_BYTES
                    || !id.bytes().all(|byte| byte.is_ascii_graphic())
                {
                    return Err(IdError);
                }
                Ok(Self(id))
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

hub_id!(
    /// An agent of the account.
    AgentId
);
hub_id!(
    /// A model provider: one of the hub's or one of the account's own.
    ProviderId
);
hub_id!(
    /// A server connected to the account's agents.
    ServerId
);
