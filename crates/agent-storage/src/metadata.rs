//! Native entropy and wall-clock facts for newly allocated chat documents.
use nocterm_ai::metadata::{ChatIdentity, ChatMetadataSource};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Default)]
pub struct NativeChatMetadata;
impl ChatMetadataSource for NativeChatMetadata {
    fn new_identity(&self) -> ChatIdentity {
        ChatIdentity {
            id: uuid::Uuid::new_v4().to_string(),
            created: self.timestamp(),
        }
    }
    fn timestamp(&self) -> u64 {
        timestamp_at(SystemTime::now())
    }
}

fn timestamp_at(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn native_chat_allocations_have_distinct_valid_uuid_identities() {
        let source = NativeChatMetadata;
        let first = source.new_identity();
        let second = source.new_identity();
        assert!(uuid::Uuid::parse_str(&first.id).is_ok());
        assert!(uuid::Uuid::parse_str(&second.id).is_ok());
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn native_chat_clock_preserves_epoch_seconds_and_pre_epoch_fallback() {
        assert_eq!(timestamp_at(UNIX_EPOCH), 0);
        assert_eq!(timestamp_at(UNIX_EPOCH + Duration::from_millis(1999)), 1);
        assert_eq!(timestamp_at(UNIX_EPOCH - Duration::from_secs(1)), 0);
    }
}
