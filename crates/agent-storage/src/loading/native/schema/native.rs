//! Semantic children carry guarded native origin, never fabricated physical spans.
use super::Schema;
use crate::loading::{json_fields::Key, native::Error};
use nocterm_ai::model::{
    json::deferred::native::{NativeKind, NativeNode},
    native::NativeOwned,
};
use std::cell::RefCell;
pub(in crate::loading) struct NativeScope {
    schema: Schema,
    values: [RefCell<Option<NativeNode>>; 18],
}
impl NativeScope {
    pub(in crate::loading::native) fn read(
        node: NativeNode,
        schema: Schema,
    ) -> std::result::Result<Self, Error> {
        let mut values: [RefCell<Option<NativeNode>>; 18] =
            std::array::from_fn(|_| RefCell::new(None));
        let array = node.kind() == NativeKind::Array;
        if array && schema.map_only {
            return Err(Error::Type);
        }
        if !matches!(node.kind(), NativeKind::Array | NativeKind::Object) {
            return Err(Error::Type);
        }
        let mut children = node.into_children()?;
        let mut count = 0;
        while let Some((key, node)) = children.next_child()? {
            let index = if array {
                if count >= schema.sequence_max {
                    return Err(Error::Positional);
                }
                Some(count)
            } else {
                let key = NativeOwned::text(key.ok_or(Error::Type)?);
                schema
                    .keys
                    .iter()
                    .position(|candidate| candidate.spelling() == key.as_str())
            };
            if let Some(index) = index {
                *values[index].get_mut() = Some(node)
            } else if schema.deny_unknown {
                return Err(Error::UnknownField);
            }
            count += 1;
        }
        let result = Self { schema, values };
        if array {
            if count < schema.positional {
                return Err(Error::Positional);
            }
        } else {
            for key in schema.required {
                if !result.has(*key) {
                    return Err(Error::Missing(*key));
                }
            }
        }
        Ok(result)
    }
    pub(super) fn schema(&self) -> Schema {
        self.schema
    }
    fn index(&self, key: Key) -> Option<usize> {
        self.schema.keys.iter().position(|k| *k == key)
    }
    pub(super) fn has(&self, key: Key) -> bool {
        self.index(key).is_some_and(|i| {
            self.values[i]
                .try_borrow()
                .is_ok_and(|value| value.is_some())
        })
    }
    pub(super) fn kind(&self, key: Key) -> std::result::Result<NativeKind, Error> {
        let index = self.index(key).ok_or(Error::Missing(key))?;
        let value = self.values[index]
            .try_borrow()
            .map_err(|_| Error::Consumed)?;
        Ok(value.as_ref().ok_or(Error::Consumed)?.kind())
    }
    pub(super) fn take(&self, key: Key) -> std::result::Result<NativeNode, Error> {
        let index = self.index(key).ok_or(Error::Missing(key))?;
        self.values[index]
            .try_borrow_mut()
            .map_err(|_| Error::Consumed)?
            .take()
            .ok_or(Error::Consumed)
    }
}
