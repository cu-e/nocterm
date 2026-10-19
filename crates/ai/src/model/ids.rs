use serde::{Deserialize, Serialize};
use std::fmt;
macro_rules! identifiers {
    ($($name:ident),+ $(,)?) => { $(
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Self { Self(value.into()) }
            pub fn as_str(&self) -> &str { &self.0 }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Display::fmt(&self.0, f) }
        }
        impl From<&str> for $name {
            fn from(value: &str) -> Self { Self::new(value) }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self { Self(value) }
        }
        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str { &self.0 }
        }
    )+ };
}
identifiers!(
    SessionId,
    ToolId,
    ModeId,
    ConfigId,
    ConfigChoiceId,
    ConfigGroupId,
    AuthMethodId,
    PermissionOptionId,
    ProviderTerminalId
);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplied_identifiers_preserve_bytes_without_inferring_authority() {
        assert_eq!(SessionId::new(" 001 ").as_str(), " 001 ");
        assert_ne!(ToolId::new("001"), ToolId::new("1"));
        let id = ModeId::new("");
        assert_eq!(
            serde_json::from_str::<ModeId>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
    }
}
