//! No filesystem access: keys belong to a desktop user and survive Nocterm
//! restarts, but disappear with the broker (reboot) or after their lifetime.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
pub(crate) const MAX_ATTEMPTS: u8 = 3;
const MAX_ENTRIES: usize = 64;
const MAX_PER_UID: usize = 4;
const LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
/// Registration policy failures retain native entropy errors until the D-Bus boundary.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RegistrationError {
    #[error("Invalid vault binding")]
    InvalidBinding,
    #[error("Invalid registration")]
    InvalidRegistration,
    #[error("Registration unavailable; unlock with the master password")]
    Unavailable,
    #[error("Device unlock capacity reached")]
    Capacity,
    #[error("Randomness unavailable")]
    Entropy {
        #[source]
        source: getrandom::Error,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Owner {
    pub uid: u32,
    pub connection: String,
}
pub(crate) struct Entry {
    /// The user and the connection that last enrolled or released the key.
    pub owner: Owner,
    pub binding: String,
    pub key: Zeroizing<[u8; 32]>,
    pub touched: Instant,
    pub cancel: Arc<AtomicBool>,
    pub busy: bool,
    pub attempts_remaining: u8,
}
#[derive(Default)]
pub(crate) struct Store {
    entries: HashMap<String, Entry>,
}
impl Store {
    pub(crate) fn valid_binding(binding: &str) -> bool {
        binding.len() == 64
            && binding
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }
    pub(crate) fn enroll(
        &mut self,
        owner: Owner,
        binding: String,
        key: Zeroizing<[u8; 32]>,
    ) -> Result<String, RegistrationError> {
        self.enroll_with_entropy(owner, binding, key, getrandom::fill)
    }

    // The fill callback keeps failure coverage deterministic without storing an
    // entropy provider or changing production randomness/registration ordering.
    fn enroll_with_entropy(
        &mut self,
        owner: Owner,
        binding: String,
        key: Zeroizing<[u8; 32]>,
        fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
    ) -> Result<String, RegistrationError> {
        self.expire();
        if !Self::valid_binding(&binding) {
            return Err(RegistrationError::InvalidBinding);
        }
        // One key per user and vault: re-enrolling replaces the previous one.
        self.entries.retain(|_, e| {
            let replaced = e.owner.uid == owner.uid && e.binding == binding;
            if replaced {
                e.cancel.store(true, Ordering::SeqCst);
            }
            !replaced
        });
        if self.entries.len() >= MAX_ENTRIES
            || self
                .entries
                .values()
                .filter(|e| e.owner.uid == owner.uid)
                .count()
                >= MAX_PER_UID
        {
            return Err(RegistrationError::Capacity);
        }
        let mut bytes = [0; 32];
        fill(&mut bytes).map_err(|source| RegistrationError::Entropy { source })?;
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        self.entries.insert(
            token.clone(),
            Entry {
                owner,
                binding,
                key,
                touched: Instant::now(),
                cancel: Arc::new(AtomicBool::new(false)),
                busy: false,
                attempts_remaining: MAX_ATTEMPTS,
            },
        );
        Ok(token)
    }
    pub(crate) fn entry(
        &mut self,
        owner: &Owner,
        binding: &str,
        token: &str,
    ) -> Result<&mut Entry, RegistrationError> {
        self.expire();
        if !Self::valid_binding(binding) || !Self::valid_binding(token) {
            return Err(RegistrationError::InvalidRegistration);
        }
        self.entries
            .get_mut(token)
            .filter(|e| e.owner.uid == owner.uid && e.binding == binding)
            .ok_or(RegistrationError::Unavailable)
    }
    pub(crate) fn remove(
        &mut self,
        owner: &Owner,
        binding: &str,
        token: &str,
    ) -> Result<(), RegistrationError> {
        if !self.entries.contains_key(token) {
            return Ok(());
        }
        self.entry(owner, binding, token)?
            .cancel
            .store(true, Ordering::SeqCst);
        self.entries.remove(token);
        Ok(())
    }
    /// A closed client cancels its authentication; the key stays registered
    /// so the next Nocterm start can unlock with a fingerprint.
    pub(crate) fn disconnected(&mut self, name: &str) {
        for entry in self.entries.values_mut() {
            if entry.owner.connection == name {
                entry.cancel.store(true, Ordering::SeqCst);
            }
        }
    }
    pub(crate) fn expire(&mut self) {
        self.entries.retain(|_, e| {
            if e.touched.elapsed() > LIFETIME {
                e.cancel.store(true, Ordering::SeqCst);
                false
            } else {
                true
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn owner(uid: u32, name: &str) -> Owner {
        Owner {
            uid,
            connection: name.into(),
        }
    }
    #[test]
    fn another_uid_cannot_read_or_delete_but_a_restart_can_read() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        let b = owner(1001, ":1.1");
        assert!(matches!(
            store.entry(&b, &id, &token),
            Err(RegistrationError::Unavailable)
        ));
        assert!(matches!(
            store.remove(&b, &id, &token),
            Err(RegistrationError::Unavailable)
        ));
        let restarted = owner(1000, ":1.2");
        assert_eq!(
            store.entry(&restarted, &id, &token).unwrap().key.as_ref(),
            &[42; 32]
        );
        assert!(matches!(
            store.entry(&a, &"b".repeat(64), &token),
            Err(RegistrationError::Unavailable)
        ));
    }
    #[test]
    fn enrolling_again_replaces_the_key_for_that_vault() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        let first = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([1; 32]))
            .unwrap();
        let second = store
            .enroll(owner(1000, ":1.2"), id.clone(), Zeroizing::new([2; 32]))
            .unwrap();
        assert!(matches!(
            store.entry(&a, &id, &first),
            Err(RegistrationError::Unavailable)
        ));
        assert_eq!(
            store.entry(&a, &id, &second).unwrap().key.as_ref(),
            &[2; 32]
        );
    }
    #[test]
    fn disconnect_cancels_requests_and_expiry_erases_registration() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        let cancel = store.entry(&a, &id, &token).unwrap().cancel.clone();
        store.disconnected(":1.1");
        assert!(cancel.load(Ordering::SeqCst));
        assert!(store.entry(&a, &id, &token).is_ok());
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        store.entry(&a, &id, &token).unwrap().touched =
            Instant::now() - LIFETIME - Duration::from_secs(1);
        assert!(matches!(
            store.entry(&a, &id, &token),
            Err(RegistrationError::Unavailable)
        ));
    }
    #[test]
    fn registration_limits_and_identifiers_are_enforced() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        assert!(matches!(
            store.enroll(a.clone(), "../vault".into(), Zeroizing::new([0; 32])),
            Err(RegistrationError::InvalidBinding)
        ));
        for vault in 0..MAX_PER_UID {
            store
                .enroll(
                    a.clone(),
                    format!("{vault}").repeat(64),
                    Zeroizing::new([0; 32]),
                )
                .unwrap();
        }
        assert!(matches!(
            store.enroll(a, "f".repeat(64), Zeroizing::new([0; 32])),
            Err(RegistrationError::Capacity)
        ));
    }

    #[test]
    fn total_capacity_recovers_after_expired_and_replaced_keys() {
        let mut store = Store::default();
        let id = "a".repeat(64);
        let mut oldest = None;
        for uid in 1000..1000 + MAX_ENTRIES as u32 {
            let connection = format!(":1.{uid}");
            let token = store
                .enroll(owner(uid, &connection), id.clone(), Zeroizing::new([0; 32]))
                .unwrap();
            if oldest.is_none() {
                oldest = Some((owner(uid, &connection), token));
            }
        }
        let newcomer = owner(2000, ":1.2000");
        assert!(matches!(
            store.enroll(newcomer.clone(), id.clone(), Zeroizing::new([0; 32])),
            Err(RegistrationError::Capacity)
        ));
        let (first, token) = oldest.unwrap();
        let entry = store.entry(&first, &id, &token).unwrap();
        let cancel = entry.cancel.clone();
        entry.busy = true;
        entry.touched = Instant::now() - LIFETIME - Duration::from_secs(1);
        let admitted = store
            .enroll(newcomer.clone(), id.clone(), Zeroizing::new([0; 32]))
            .unwrap();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(matches!(
            store.entry(&first, &id, &token),
            Err(RegistrationError::Unavailable)
        ));
        let cancel = store
            .entry(&newcomer, &id, &admitted)
            .unwrap()
            .cancel
            .clone();
        store.disconnected(&newcomer.connection);
        assert!(cancel.load(Ordering::SeqCst));
        assert!(store.entry(&newcomer, &id, &admitted).is_ok());
        assert!(
            store
                .enroll(newcomer.clone(), id.clone(), Zeroizing::new([0; 32]))
                .is_ok()
        );
        assert!(matches!(
            store.entry(&newcomer, &id, &admitted),
            Err(RegistrationError::Unavailable)
        ));
    }

    #[test]
    fn malformed_registrations_do_not_disclose_or_remove_a_matching_key() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let binding = "a".repeat(64);
        let token = store
            .enroll(a.clone(), binding.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        for (binding, token) in [("bad", token.as_str()), (binding.as_str(), "BAD")] {
            assert!(matches!(
                store.entry(&a, binding, token),
                Err(RegistrationError::InvalidRegistration)
            ));
        }
        let error = store.remove(&a, "bad", &token).unwrap_err();
        assert!(matches!(error, RegistrationError::InvalidRegistration));
        assert!(!error.to_string().contains(&token));
        assert_eq!(
            store.entry(&a, &binding, &token).unwrap().key.as_ref(),
            &[42; 32]
        );
    }

    #[test]
    fn deleting_a_known_registration_cancels_it_and_unknown_remove_is_idempotent() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let binding = "a".repeat(64);
        let token = store
            .enroll(a.clone(), binding.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        let cancel = store.entry(&a, &binding, &token).unwrap().cancel.clone();
        store.remove(&a, &binding, &token).unwrap();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(matches!(
            store.entry(&a, &binding, &token),
            Err(RegistrationError::Unavailable)
        ));
        store.remove(&a, &binding, &token).unwrap();
        // Unknown token success precedes registration-shape and authority checks.
        store
            .remove(&owner(1001, ":1.9"), "bad", "unknown")
            .unwrap();
    }

    #[test]
    fn entropy_failure_keeps_the_native_source_and_cancels_the_replaced_registration() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let binding = "a".repeat(64);
        let token = store
            .enroll(a.clone(), binding.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        let cancel = store.entry(&a, &binding, &token).unwrap().cancel.clone();
        let error = store
            .enroll_with_entropy(a.clone(), binding.clone(), Zeroizing::new([7; 32]), |_| {
                Err(getrandom::Error::UNSUPPORTED)
            })
            .unwrap_err();
        assert!(
            matches!(&error, RegistrationError::Entropy { source } if *source == getrandom::Error::UNSUPPORTED)
        );
        assert!(
            std::error::Error::source(&error)
                .unwrap()
                .is::<getrandom::Error>()
        );
        assert_eq!(error.to_string(), "Randomness unavailable");
        assert!(!format!("{error:?}").contains(&token));
        assert!(!format!("{error:?}").contains(&binding));
        assert!(cancel.load(Ordering::SeqCst));
        assert!(matches!(
            store.entry(&a, &binding, &token),
            Err(RegistrationError::Unavailable)
        ));
        // A later successful registration has no inherited authentication budget.
        let next = store
            .enroll(a.clone(), binding.clone(), Zeroizing::new([7; 32]))
            .unwrap();
        assert_eq!(
            store.entry(&a, &binding, &next).unwrap().attempts_remaining,
            MAX_ATTEMPTS
        );
    }

    #[test]
    fn invalid_binding_and_capacity_reject_before_requesting_entropy() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        assert!(matches!(
            store.enroll_with_entropy(
                a.clone(),
                "bad".into(),
                Zeroizing::new([0; 32]),
                |_| panic!("invalid binding requested entropy")
            ),
            Err(RegistrationError::InvalidBinding)
        ));
        for vault in 0..MAX_PER_UID {
            store
                .enroll(
                    a.clone(),
                    format!("{vault}").repeat(64),
                    Zeroizing::new([0; 32]),
                )
                .unwrap();
        }
        assert!(matches!(
            store.enroll_with_entropy(a, "f".repeat(64), Zeroizing::new([0; 32]), |_| panic!(
                "full store requested entropy"
            )),
            Err(RegistrationError::Capacity)
        ));
    }
}
