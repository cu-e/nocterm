//! Search uses the original contextual Unicode lowercase operation. Its pinned
//! Rust allocation growth is prepaid while the original guarded text remains live.
use crate::model::{
    json::deferred::{DecodeLedger, JsonDecodeError},
    retention::document::ConstructionRole,
};
pub(super) struct SearchPattern<'a> {
    query: &'a str,
    ledger: DecodeLedger,
}
impl<'a> SearchPattern<'a> {
    pub(super) fn new(role: &ConstructionRole, query: &'a str) -> Result<Self, JsonDecodeError> {
        Ok(Self {
            query,
            ledger: DecodeLedger::for_operation(role, crate::history::MAX_RESIDENT_BYTES)?,
        })
    }
    pub(super) fn contains(&self, text: &str) -> Result<bool, JsonDecodeError> {
        if self.query.is_empty() {
            return Ok(true);
        }
        let (capacity, peak) = lowercase_allocation(text)?;
        let allocation = self.ledger.reserve(peak)?;
        let lowercase = text.to_lowercase();
        if lowercase.capacity() != capacity {
            return Err(JsonDecodeError::CapacityProfile);
        }
        let found = lowercase.contains(self.query);
        drop(lowercase);
        drop(allocation);
        Ok(found)
    }
}
pub(crate) fn matches(
    role: &ConstructionRole,
    snapshot: &crate::history::physical::SnapshotOwner,
    query: &str,
) -> Result<bool, JsonDecodeError> {
    use crate::history::physical::{ContentSlot, RowBody};
    let pattern = SearchPattern::new(role, query)?;
    if let Some(title) = snapshot.metadata().title()
        && pattern.contains(&title)?
    {
        return Ok(true);
    }
    for row in snapshot.rows() {
        match row.view() {
            RowBody::Agent(text) => {
                if pattern.contains(&text.read())? {
                    return Ok(true);
                }
            }
            RowBody::User(parts) => {
                for content in parts.contents() {
                    if let ContentSlot::Text(text) = content
                        && pattern.contains(&text.text())?
                    {
                        return Ok(true);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(false)
}
/// Rust1.98 alloc::str starts with exact input byte capacity. Each output char
/// uses Vec<u8>'s amortized growth. Contextual sigma changes no UTF-8 byte length.
/// Include simultaneous old/new buffers even when the allocator grows in place.
fn lowercase_allocation(text: &str) -> Result<(usize, usize), JsonDecodeError> {
    let mut capacity = text.len();
    let mut peak = capacity;
    let mut len = 0usize;
    for ch in text.chars().flat_map(char::to_lowercase) {
        let required = len
            .checked_add(ch.len_utf8())
            .ok_or(JsonDecodeError::Arithmetic)?;
        if required > capacity {
            let next = capacity
                .checked_mul(2)
                .ok_or(JsonDecodeError::Arithmetic)?
                .max(required)
                .max(8);
            peak = peak.max(
                capacity
                    .checked_add(next)
                    .ok_or(JsonDecodeError::Arithmetic)?,
            );
            capacity = next;
        }
        len = required;
    }
    Ok((capacity, peak))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lowercase_capacity_profile_covers_contextual_sigma_and_expanding_text() {
        for text in [
            "",
            "ASCII",
            "ΟΣ",
            "ΣΟΣ",
            "ΟΣ'",
            "ΟΣ'Α",
            "İ",
            "İİİİİİ",
            "ﬃ",
            "Москва",
            "aİΣ",
        ] {
            let (capacity, peak) = lowercase_allocation(text).unwrap();
            let value = text.to_lowercase();
            assert_eq!(value.capacity(), capacity, "{text:?}");
            assert!(peak >= value.capacity());
        }
    }
}
