//! Borrowed wire access to a genuine controlled native document awaiting publication.
use crate::{
    history::{HistoryCodecError, SavedDocument, wire::Wire},
    model::native::NativeOwned,
};
use serde::{Serialize, Serializer};
pub struct NativeDocumentWire<'a>(Wire<'a>);
impl<'a> NativeDocumentWire<'a> {
    pub fn new(document: &'a NativeOwned<SavedDocument>) -> Result<Self, HistoryCodecError> {
        Ok(Self(Wire::new(document.get().view()?)))
    }
}
impl Serialize for NativeDocumentWire<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}
