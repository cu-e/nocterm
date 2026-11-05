//! Identifiers of a conversation and of what it holds. Plain values; none confers authority.
use core::fmt;

fn hex(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    bytes.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
}

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);
        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }
            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                hex(&self.0, f)
            }
        }
    };
}

id!(
    /// A command this device sent.
    CommandKey
);
id!(
    /// A run a prompt started.
    RunKey
);
id!(
    /// A tool call of a run.
    IntentKey
);

/// What a history row and a window name one chat by: the chat's reference as text. Stable for
/// the same chat across listings and restarts.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConversationKey(String);
impl ConversationKey {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for ConversationKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_print_as_lowercase_hex() {
        let mut bytes = [0u8; 16];
        bytes[0] = 0xab;
        bytes[15] = 0x01;
        assert_eq!(
            CommandKey::from_bytes(bytes).to_string(),
            "ab000000000000000000000000000001"
        );
        assert_eq!(ConversationKey::new("a-b-1").as_str(), "a-b-1");
    }
}
