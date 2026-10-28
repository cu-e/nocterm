//! The environment a window belongs to.

use crate::address::HubId;

/// What a window shows: this computer, or the account of one hub.
///
/// It is chosen when the window is composed and never changes inside a window.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Environment {
    Local,
    Hub(HubId),
}

impl Environment {
    pub const fn is_local(&self) -> bool {
        matches!(self, Self::Local)
    }

    /// The hub of a hub window.
    pub const fn hub(&self) -> Option<&HubId> {
        match self {
            Self::Local => None,
            Self::Hub(hub) => Some(hub),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::HubAddress;

    #[test]
    fn a_local_environment_has_no_hub_and_a_hub_environment_names_its_hub() {
        let hub = HubId::from(HubAddress::parse("hub.home").unwrap());
        assert!(Environment::Local.is_local());
        assert_eq!(Environment::Local.hub(), None);
        let environment = Environment::Hub(hub.clone());
        assert!(!environment.is_local());
        assert_eq!(environment.hub(), Some(&hub));
        assert_ne!(environment, Environment::Local);
    }
}
