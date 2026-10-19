//! Named native archive edits preserve original graph ownership and draft identity.
use crate::{
    history::{HistoryCodecError, SavedDocument},
    model::native::NativeOwned,
};

pub fn patch_archived_draft(
    owned: &mut NativeOwned<SavedDocument>,
    draft: Option<&str>,
    updated: u64,
) -> Result<bool, HistoryCodecError> {
    if owned.get().composer.draft.as_deref() == draft {
        return Ok(false);
    }
    let bytes = draft.map_or(0, str::len);
    let mut allocation = owned.ledger().reserve(bytes)?;
    let value = draft
        .map(|text| {
            let mut value = String::new();
            value
                .try_reserve_exact(text.len())
                .map_err(crate::model::json::deferred::JsonDecodeError::from)?;
            if value.capacity() != text.len() {
                return Err(crate::model::json::deferred::JsonDecodeError::CapacityProfile);
            }
            value.push_str(text);
            Ok(value)
        })
        .transpose()?;
    owned.replace_archived_draft(value, updated, &mut allocation)?;
    Ok(true)
}
