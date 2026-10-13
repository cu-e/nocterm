//! Deterministic document identity and timestamps, explicitly shared by host fixtures.
use crate::metadata::{ChatIdentity, ChatMetadataSource};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetadataRequests {
    pub identities: u64,
    pub timestamps: u64,
}

pub struct ScriptedChatMetadata {
    state: Mutex<State>,
}
struct State {
    next: u64,
    timestamp: u64,
    requests: MetadataRequests,
}
impl Default for ScriptedChatMetadata {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                next: 1,
                timestamp: 1,
                requests: MetadataRequests::default(),
            }),
        }
    }
}
impl ScriptedChatMetadata {
    pub fn requests(&self) -> MetadataRequests {
        self.state.lock().expect("test metadata").requests
    }
    pub fn timestamp_is(&self, timestamp: u64) {
        self.state.lock().expect("test metadata").timestamp = timestamp;
    }
}
impl ChatMetadataSource for ScriptedChatMetadata {
    fn new_identity(&self) -> ChatIdentity {
        let mut state = self.state.lock().expect("test metadata");
        state.requests.identities += 1;
        let id = format!(
            "{:08x}-0000-4000-8000-{:012x}",
            state.next >> 48,
            state.next & 0xffff_ffff_ffff
        );
        state.next = state.next.checked_add(1).expect("test identity sequence");
        ChatIdentity {
            id,
            created: state.timestamp,
        }
    }
    fn timestamp(&self) -> u64 {
        let mut state = self.state.lock().expect("test metadata");
        state.requests.timestamps += 1;
        state.timestamp
    }
}
