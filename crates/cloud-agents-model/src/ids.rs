//! Opaque identifiers the hub gives agents, pools and seats.

use std::fmt;

/// An identifier is empty, too long or holds a control character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the identifier is empty, longer than {MAX_ID_BYTES} bytes or holds a control character")]
pub struct IdError;

/// Longest identifier the client accepts from a hub.
pub const MAX_ID_BYTES: usize = 64;

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Wraps the text the hub sent.
            pub fn new(text: impl Into<String>) -> Result<Self, IdError> {
                let text = text.into();
                if text.is_empty() || text.len() > MAX_ID_BYTES || text.chars().any(char::is_control) {
                    return Err(IdError);
                }
                Ok(Self(text))
            }

            /// The text as the hub wrote it.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    };
}

id!(
    /// An agent of a hub.
    AgentId
);
id!(
    /// A pool of hosts that runs agents.
    PoolId
);
id!(
    /// One seat: an account's runner of one agent.
    InstanceId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_is_bounded_and_printable() {
        assert!(AgentId::new("a1").is_ok());
        assert_eq!(AgentId::new(""), Err(IdError));
        assert_eq!(PoolId::new("x".repeat(MAX_ID_BYTES + 1)), Err(IdError));
        assert_eq!(InstanceId::new("a\nb"), Err(IdError));
    }
}
