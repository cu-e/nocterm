//! Supplied descriptive identity and authority-source partition coordinates.
pub use nocterm_hubs_model::source_identity::{
    AccountIdentity, AccountScope, Fingerprint, HubIdentity, MAX_NAMESPACE_BYTES, NamespaceId,
    SourceIdentityError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("an authority epoch must be nonzero")]
    ZeroEpoch,
    #[error("the revision or epoch is exhausted")]
    Exhausted,
}

macro_rules! opaque_ids {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 16]);
        impl $name {
            /// The host supplies identity bytes; this function grants no authority.
            pub const fn from_bytes(bytes: [u8; 16]) -> Self { Self(bytes) }
            pub const fn as_bytes(&self) -> &[u8; 16] { &self.0 }
        }
    )+};
}
opaque_ids!(
    CloudChatId,
    RunId,
    RunnerId,
    CommandId,
    IntentId,
    StepId,
    AttemptId,
    GrantId,
    RemoteSessionId,
    SchedulerId,
    EventId
);

macro_rules! epochs {
    ($($name:ident),+ $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u64);
        impl $name {
            pub fn new(value: u64) -> Result<Self, IdentityError> {
                if value == 0 { Err(IdentityError::ZeroEpoch) } else { Ok(Self(value)) }
            }
            pub const fn get(self) -> u64 { self.0 }
            pub fn next(self) -> Result<Self, IdentityError> {
                self.0.checked_add(1).map(Self).ok_or(IdentityError::Exhausted)
            }
        }
    )+};
}
epochs!(
    RunEpoch,
    WriterEpoch,
    SessionEpoch,
    GrantEpoch,
    RunnerKeyEpoch,
    ChatKeyEpoch
);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(u64);
impl Revision {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::Exhausted)
    }
}

/// Event replay position, distinct from the transaction's semantic revision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventSequence(u64);
impl EventSequence {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn advance(self, count: usize) -> Result<Self, IdentityError> {
        let count = u64::try_from(count).map_err(|_| IdentityError::Exhausted)?;
        self.0
            .checked_add(count)
            .map(Self)
            .ok_or(IdentityError::Exhausted)
    }
}

/// Atomic chat-journal transaction revision, not a per-run FSM revision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamRevision(u64);
impl StreamRevision {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::Exhausted)
    }
}

/// Local durable record cursor, independent of the externally delivered event cursor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalSequence(u64);
impl JournalSequence {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn advance(self, count: usize) -> Result<Self, IdentityError> {
        let count = u64::try_from(count).map_err(|_| IdentityError::Exhausted)?;
        self.0
            .checked_add(count)
            .map(Self)
            .ok_or(IdentityError::Exhausted)
    }
}

/// A supplied millisecond observation. It performs no wall-clock read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);
impl Timestamp {
    pub const fn from_millis(value: u64) -> Self {
        Self(value)
    }
    pub const fn as_millis(self) -> u64 {
        self.0
    }
    pub fn elapsed_since(self, earlier: Self) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }
}

/// User-facing command/control admission revision; streaming never advances it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChatControlRevision(u64);
impl ChatControlRevision {
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
    pub const fn get(self) -> u64 {
        self.0
    }
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::Exhausted)
    }
}
