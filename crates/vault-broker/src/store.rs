//! No filesystem access: keys disappear with their owner connection or expiry.
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
const MAX_ENTRIES: usize = 64;
const MAX_PER_UID: usize = 4;
const LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Owner {
    pub uid: u32,
    pub connection: String,
}
pub(crate) struct Entry {
    pub owner: Owner,
    pub binding: String,
    pub key: Zeroizing<[u8; 32]>,
    pub touched: Instant,
    pub cancel: Arc<AtomicBool>,
    pub busy: bool,
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
    ) -> Result<String, String> {
        self.expire();
        if !Self::valid_binding(&binding) {
            return Err("Invalid vault binding".into());
        }
        if self.entries.len() >= MAX_ENTRIES
            || self
                .entries
                .values()
                .filter(|e| e.owner.uid == owner.uid)
                .count()
                >= MAX_PER_UID
        {
            return Err("Device unlock capacity reached".into());
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| "Randomness unavailable")?;
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
            },
        );
        Ok(token)
    }
    pub(crate) fn entry(
        &mut self,
        owner: &Owner,
        binding: &str,
        token: &str,
    ) -> Result<&mut Entry, String> {
        self.expire();
        if !Self::valid_binding(binding) || !Self::valid_binding(token) {
            return Err("Invalid registration".into());
        }
        self.entries
            .get_mut(token)
            .filter(|e| e.owner == *owner && e.binding == binding)
            .ok_or_else(|| "Registration unavailable; unlock with the master password".into())
    }
    pub(crate) fn remove(
        &mut self,
        owner: &Owner,
        binding: &str,
        token: &str,
    ) -> Result<(), String> {
        if !self.entries.contains_key(token) {
            return Ok(());
        }
        self.entry(owner, binding, token)?
            .cancel
            .store(true, Ordering::SeqCst);
        self.entries.remove(token);
        Ok(())
    }
    pub(crate) fn disconnected(&mut self, name: &str) {
        self.entries.retain(|_, e| {
            if e.owner.connection == name {
                e.cancel.store(true, Ordering::SeqCst);
                false
            } else {
                true
            }
        });
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
    fn another_connection_or_uid_cannot_read_or_delete() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        for b in [owner(1000, ":1.2"), owner(1001, ":1.1")] {
            assert!(store.entry(&b, &id, &token).is_err());
            assert!(store.remove(&b, &id, &token).is_err());
        }
        assert_eq!(
            store.entry(&a, &id, &token).unwrap().key.as_ref(),
            &[42; 32]
        );
        assert!(store.entry(&a, &"b".repeat(64), &token).is_err());
    }
    #[test]
    fn disconnect_and_expiry_cancel_requests_and_erase_registration() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        let cancel = store.entry(&a, &id, &token).unwrap().cancel.clone();
        store.disconnected(":1.1");
        assert!(cancel.load(Ordering::SeqCst));
        assert!(store.entry(&a, &id, &token).is_err());
        let token = store
            .enroll(a.clone(), id.clone(), Zeroizing::new([42; 32]))
            .unwrap();
        store.entry(&a, &id, &token).unwrap().touched =
            Instant::now() - LIFETIME - Duration::from_secs(1);
        assert!(store.entry(&a, &id, &token).is_err());
    }
    #[test]
    fn registration_limits_and_identifiers_are_enforced() {
        let mut store = Store::default();
        let a = owner(1000, ":1.1");
        let id = "a".repeat(64);
        assert!(
            store
                .enroll(a.clone(), "../vault".into(), Zeroizing::new([0; 32]))
                .is_err()
        );
        for _ in 0..MAX_PER_UID {
            store
                .enroll(a.clone(), id.clone(), Zeroizing::new([0; 32]))
                .unwrap();
        }
        assert!(store.enroll(a, id, Zeroizing::new([0; 32])).is_err());
    }

    #[test]
    fn total_capacity_recovers_after_expired_and_disconnected_clients() {
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
        assert!(
            store
                .enroll(newcomer.clone(), id.clone(), Zeroizing::new([0; 32]))
                .is_err()
        );
        let (first, token) = oldest.unwrap();
        let entry = store.entry(&first, &id, &token).unwrap();
        let cancel = entry.cancel.clone();
        entry.busy = true;
        entry.touched = Instant::now() - LIFETIME - Duration::from_secs(1);
        let admitted = store
            .enroll(newcomer.clone(), id.clone(), Zeroizing::new([0; 32]))
            .unwrap();
        assert!(cancel.load(Ordering::SeqCst));
        assert!(store.entry(&first, &id, &token).is_err());
        let cancel = store
            .entry(&newcomer, &id, &admitted)
            .unwrap()
            .cancel
            .clone();
        store.disconnected(&newcomer.connection);
        assert!(cancel.load(Ordering::SeqCst));
        assert!(store.entry(&newcomer, &id, &admitted).is_err());
        assert!(store.enroll(newcomer, id, Zeroizing::new([0; 32])).is_ok());
    }
}
