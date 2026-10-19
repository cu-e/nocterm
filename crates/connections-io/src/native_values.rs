//! User/home defaults, UUID entropy and native clock values.
use nocterm_connections_model::{
    draft::EditorDefaults,
    profiles::ProfileId,
    value_ports::{ConnectionClock, ProfileIds},
};
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
pub struct NativeValues;
impl NativeValues {
    /// Discover once during background application preparation.
    pub fn editor_defaults() -> EditorDefaults {
        EditorDefaults {
            user: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|user| !user.is_empty()),
            home: std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from),
        }
    }
}
impl ProfileIds for NativeValues {
    fn next(&self) -> ProfileId {
        ProfileId::from_bytes(*uuid::Uuid::new_v4().as_bytes())
    }
}
impl ConnectionClock for NativeValues {
    fn unix_seconds(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_identifiers_keep_random_uuid_identity_and_canonical_format() {
        let ids = [NativeValues.next(), NativeValues.next()];
        assert_ne!(ids[0], ids[1]);
        for id in ids {
            let encoded = id.to_string();
            let uuid = encoded.parse::<uuid::Uuid>().unwrap();
            assert_eq!(uuid.get_version_num(), 4);
            assert_eq!(encoded.parse::<ProfileId>().unwrap(), id);
        }
    }
}
