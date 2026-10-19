//! Neutral live attachment identities; none of these values grants execution authority.
use crate::model::{
    json::deferred::{DecodeLedger, JsonDecodeError},
    native::NativeOwned,
};

/// Lossless copy of the original workspace entity's scalar key, not an Item handle.
/// GPUI's EntityId::as_u64 returns slotmap KeyData::as_ffi, including its generation.
/// Resolve only against existing workspace items; never mint authority from this key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocalItemKey(u64);
impl LocalItemKey {
    pub fn from_bits(bits: u64) -> Self {
        Self(bits)
    }
    pub fn bits(self) -> u64 {
        self.0
    }
}

/// Actual live input values. Persisted attachments use their separate stable format.
#[derive(Debug, PartialEq, Eq)]
pub enum LiveAttachment {
    Connection(String),
    Group(String),
    UnavailableLocal(String),
    LocalItem(LocalItemKey),
}
pub fn connection(text: NativeOwned<String>) -> NativeOwned<LiveAttachment> {
    let (text, allocation) = text.into_parts();
    NativeOwned::from_parts(LiveAttachment::Connection(text), allocation)
}
pub fn group(text: NativeOwned<String>) -> NativeOwned<LiveAttachment> {
    let (text, allocation) = text.into_parts();
    NativeOwned::from_parts(LiveAttachment::Group(text), allocation)
}
pub fn unavailable_local(text: NativeOwned<String>) -> NativeOwned<LiveAttachment> {
    let (text, allocation) = text.into_parts();
    NativeOwned::from_parts(LiveAttachment::UnavailableLocal(text), allocation)
}
pub fn local_item(
    key: LocalItemKey,
    ledger: &DecodeLedger,
) -> Result<NativeOwned<LiveAttachment>, JsonDecodeError> {
    let (key, allocation) = NativeOwned::scalar(key, ledger)?.into_parts();
    Ok(NativeOwned::from_parts(
        LiveAttachment::LocalItem(key),
        allocation,
    ))
}

/// Borrowed stable attachment facts selected through the original feature ports.
/// These names carry no terminal or connection execution authority.
pub enum SavedAttachmentRef<'a> {
    Connection(&'a str),
    Group(&'a str),
    LocalTerminal(&'a str),
}
