//! Exact span and typed vector backing is prepaid before either collection grows.
use super::{
    Error,
    schema::{Schema, Scope},
};
use crate::loading::{
    held_file::{HeldFile, plan::AuthenticatedPlan},
    json_fields::Key,
    json_frames::{Element, RootFrames},
};
use nocterm_ai::model::{
    json::deferred::{DecodeLedger, NativeScratch},
    native::{NativeList, NativeOwned},
};
use std::mem::size_of;
struct Elements {
    values: Vec<Element>,
    _allocation: NativeScratch,
}
impl Elements {
    fn scan(
        reader: &mut dyn std::io::Read,
        start: usize,
        capacity: usize,
        ledger: &DecodeLedger,
    ) -> std::result::Result<Self, Error> {
        let allocation = NativeScratch::reserve(
            ledger,
            capacity
                .checked_mul(size_of::<Element>())
                .ok_or(nocterm_ai::model::json::deferred::JsonDecodeError::Arithmetic)?,
        )?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if values.capacity() != capacity {
            return Err(nocterm_ai::model::json::deferred::JsonDecodeError::CapacityProfile.into());
        }
        let mut frames = RootFrames::new_literal_managed(reader, start)?;
        let RootFrames::Array(array) = &mut frames else {
            return Err(Error::Type);
        };
        while let Some(element) = array.next_element()? {
            if values.len() == capacity {
                return Err(Error::Collection);
            }
            values.push(element)
        }
        frames.finish()?;
        if values.len() != capacity {
            return Err(Error::Collection);
        }
        Ok(Self {
            values,
            _allocation: allocation,
        })
    }
}
fn count(
    reader: &mut dyn std::io::Read,
    start: usize,
    limit: usize,
) -> std::result::Result<usize, Error> {
    let mut frames = RootFrames::new_literal_managed(reader, start)?;
    let RootFrames::Array(array) = &mut frames else {
        return Err(Error::Type);
    };
    let mut count = 0;
    while array.next_element()?.is_some() {
        if count == limit {
            return Err(Error::Collection);
        }
        count += 1;
    }
    frames.finish()?;
    Ok(count)
}
pub(super) fn list<T>(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    limit: usize,
    schema: Schema,
    mut decode: impl FnMut(
        &AuthenticatedPlan<Scope>,
        &mut HeldFile,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Vec<T>>, Error> {
    if parent.payload().is_native() && !parent.payload().has(key) {
        return Ok(NativeList::new(ledger, 0)?.finish());
    }
    if parent.payload().native_kind(key).is_some() {
        let node = super::fields::node(parent, held, key, ledger)?;
        if node.kind() != nocterm_ai::model::json::deferred::native::NativeKind::Array {
            return Err(Error::Type);
        }
        let count = node.container_len().ok_or(Error::Type)?;
        if count > limit {
            return Err(Error::Collection);
        }
        let mut values = NativeList::new(ledger, count)?;
        let mut children = node.into_children()?;
        while let Some((_, child)) = children.next_child()? {
            let scope = parent.native_scope(super::schema::NativeScope::read(child, schema)?);
            values.push(decode(&scope, held, ledger)?)?;
        }
        return Ok(values.finish());
    }
    let Some(field) = parent.payload().field(key) else {
        return Ok(NativeList::new(ledger, 0)?.finish());
    };
    let start = field.start();
    let count = parent.replay(held, |_| field.span(), |reader| count(reader, start, limit))??;
    let elements = parent
        .derive(
            held,
            |_| field.span(),
            |reader| Elements::scan(reader, start, count, ledger),
        )?
        .transpose()?;
    let mut values = NativeList::new(ledger, count)?;
    for element in &elements.payload().values {
        let start = element.value().start();
        let scope = elements
            .derive(
                held,
                |_| element.native_span(),
                |reader| Scope::scan_with_ledger(reader, start, schema, Some(ledger)),
            )?
            .transpose()?;
        values.push(decode(&scope, held, ledger)?)?;
    }
    Ok(values.finish())
}
pub(super) fn primitive<T>(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    limit: usize,
    mut decode: impl FnMut(
        nocterm_ai::model::json::deferred::native::NativeNode,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Vec<T>>, Error> {
    if parent.payload().is_native() && !parent.payload().has(key) {
        return Ok(NativeList::new(ledger, 0)?.finish());
    }
    if parent.payload().native_kind(key).is_some() {
        let node = super::fields::node(parent, held, key, ledger)?;
        if node.kind() != nocterm_ai::model::json::deferred::native::NativeKind::Array {
            return Err(Error::Type);
        }
        let count = node.container_len().ok_or(Error::Type)?;
        if count > limit {
            return Err(Error::Collection);
        }
        let mut values = NativeList::new(ledger, count)?;
        let mut children = node.into_children()?;
        while let Some((_, child)) = children.next_child()? {
            values.push(decode(child, ledger)?)?;
        }
        return Ok(values.finish());
    }
    let Some(field) = parent.payload().field(key) else {
        return Ok(NativeList::new(ledger, 0)?.finish());
    };
    let start = field.start();
    let count = parent.replay(held, |_| field.span(), |reader| count(reader, start, limit))??;
    let elements = parent
        .derive(
            held,
            |_| field.span(),
            |reader| Elements::scan(reader, start, count, ledger),
        )?
        .transpose()?;
    let mut values = NativeList::new(ledger, count)?;
    for element in &elements.payload().values {
        let value = crate::loading::literal_reader::decode_authenticated_native(
            &elements,
            held,
            |_| element.native_span(),
            ledger,
        )??;
        values.push(decode(value, ledger)?)?;
    }
    Ok(values.finish())
}

pub(super) fn skipping<T>(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    schema: Schema,
    mut decode: impl FnMut(
        &AuthenticatedPlan<Scope>,
        &mut HeldFile,
        &DecodeLedger,
    ) -> std::result::Result<NativeOwned<T>, Error>,
) -> std::result::Result<NativeOwned<Vec<T>>, Error> {
    if parent.payload().is_native() && !parent.payload().has(key) {
        return Ok(NativeList::new(ledger, 0)?.finish());
    }
    if parent.payload().native_kind(key).is_some() {
        let node = super::fields::node(parent, held, key, ledger)?;
        if node.kind() != nocterm_ai::model::json::deferred::native::NativeKind::Array {
            return Ok(NativeList::new(ledger, 0)?.finish());
        }
        let count = node.container_len().ok_or(Error::Type)?;
        let mut values = NativeList::new(ledger, count)?;
        let mut children = node.into_children()?;
        while let Some((_, child)) = children.next_child()? {
            let decoded = super::schema::NativeScope::read(child, schema)
                .and_then(|scope| decode(&parent.native_scope(scope), held, ledger));
            match decoded {
                Ok(value) => values.push(value)?,
                Err(error) if error.defaultable() => {}
                Err(error) => return Err(error),
            }
        }
        return Ok(values.finish());
    }
    let Some(field) = parent.payload().field(key) else {
        return Ok(NativeList::new(ledger, 0)?.finish());
    };
    let start = field.start();
    if !super::fields::starts(parent, held, key, b'[')? {
        return Ok(NativeList::new(ledger, 0)?.finish());
    }
    let count = parent.replay(
        held,
        |_| field.span(),
        |reader| count(reader, start, usize::MAX),
    )??;
    let elements = parent
        .derive(
            held,
            |_| field.span(),
            |reader| Elements::scan(reader, start, count, ledger),
        )?
        .transpose()?;
    let mut values = NativeList::new(ledger, count)?;
    for element in &elements.payload().values {
        let start = element.value().start();
        let scope = elements
            .derive(
                held,
                |_| element.native_span(),
                |reader| Scope::scan_with_ledger(reader, start, schema, Some(ledger)),
            )?
            .transpose();
        let decoded = scope.and_then(|scope| decode(&scope, held, ledger));
        match decoded {
            Ok(value) => values.push(value)?,
            Err(error) if error.defaultable() => {}
            Err(error) => return Err(error),
        }
    }
    Ok(values.finish())
}
pub(super) fn field_count(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    limit: usize,
) -> std::result::Result<usize, Error> {
    let field = parent.payload().required(key)?;
    let start = field.start();
    parent.replay(held, |_| field.span(), |reader| count(reader, start, limit))?
}

pub(super) struct Members {
    values: Vec<crate::loading::json_frames::Member>,
    _allocation: NativeScratch,
}
impl Members {
    fn scan(
        reader: &mut dyn std::io::Read,
        start: usize,
        capacity: usize,
        ledger: &DecodeLedger,
    ) -> std::result::Result<Self, Error> {
        let allocation = NativeScratch::reserve(
            ledger,
            capacity
                .checked_mul(size_of::<crate::loading::json_frames::Member>())
                .ok_or(nocterm_ai::model::json::deferred::JsonDecodeError::Arithmetic)?,
        )?;
        let mut values = Vec::new();
        values.try_reserve_exact(capacity)?;
        if values.capacity() != capacity {
            return Err(nocterm_ai::model::json::deferred::JsonDecodeError::CapacityProfile.into());
        }
        let mut frames = RootFrames::new_literal_managed(reader, start)?;
        let RootFrames::Object(map) = &mut frames else {
            return Err(Error::Type);
        };
        while let Some(member) = map.next_member_with_ledger(Some(ledger))? {
            if values.len() == capacity {
                return Err(Error::Collection);
            }
            values.push(member)
        }
        frames.finish()?;
        if values.len() != capacity {
            return Err(Error::Collection);
        }
        Ok(Self {
            values,
            _allocation: allocation,
        })
    }
}
pub(super) fn validate_members(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    keys: Option<&[Key]>,
    mut validate: impl FnMut(
        &AuthenticatedPlan<Members>,
        &mut HeldFile,
        &crate::loading::json_frames::Member,
        &DecodeLedger,
    ) -> std::result::Result<(), Error>,
) -> std::result::Result<(), Error> {
    if parent.payload().is_native() {
        return Ok(());
    }
    let field = parent.payload().required(key)?;
    let start = field.start();
    let count = parent.replay(
        held,
        |_| field.span(),
        |reader| {
            let mut frames = RootFrames::new_literal_managed(reader, start)?;
            let RootFrames::Object(map) = &mut frames else {
                return Err(Error::Type);
            };
            let mut count = 0usize;
            while map.next_member_with_ledger(Some(ledger))?.is_some() {
                count = count
                    .checked_add(1)
                    .ok_or(nocterm_ai::model::json::deferred::JsonDecodeError::Arithmetic)?;
            }
            frames.finish()?;
            Ok(count)
        },
    )??;
    let members = parent
        .derive(
            held,
            |_| field.span(),
            |reader| Members::scan(reader, start, count, ledger),
        )?
        .transpose()?;
    for member in &members.payload().values {
        if keys.is_none_or(|keys| keys.contains(&member.kind())) {
            validate(&members, held, member, ledger)?;
        }
    }
    Ok(())
}

pub(super) fn primitive_skipping<T>(
    parent: &AuthenticatedPlan<Scope>,
    held: &mut HeldFile,
    key: Key,
    ledger: &DecodeLedger,
    limit: usize,
    mut decode: impl FnMut(
        nocterm_ai::model::json::deferred::native::NativeNode,
        &DecodeLedger,
    ) -> std::result::Result<Option<NativeOwned<T>>, Error>,
) -> std::result::Result<NativeOwned<Vec<T>>, Error> {
    if parent.payload().is_native() && !parent.payload().has(key) {
        return Ok(NativeList::new(ledger, 0)?.finish());
    }
    if parent.payload().native_kind(key).is_some() {
        let node = super::fields::node(parent, held, key, ledger)?;
        if node.kind() != nocterm_ai::model::json::deferred::native::NativeKind::Array {
            return Err(Error::Type);
        }
        let count = node.container_len().ok_or(Error::Type)?;
        if count > limit {
            return Err(Error::Collection);
        }
        let mut values = NativeList::new(ledger, count)?;
        let mut children = node.into_children()?;
        while let Some((_, child)) = children.next_child()? {
            if let Some(value) = decode(child, ledger)? {
                values.push(value)?;
            }
        }
        return Ok(values.finish());
    }
    let Some(field) = parent.payload().field(key) else {
        return Ok(NativeList::new(ledger, 0)?.finish());
    };
    let start = field.start();
    let count = parent.replay(held, |_| field.span(), |reader| count(reader, start, limit))??;
    let elements = parent
        .derive(
            held,
            |_| field.span(),
            |reader| Elements::scan(reader, start, count, ledger),
        )?
        .transpose()?;
    let mut values = NativeList::new(ledger, count)?;
    for element in &elements.payload().values {
        let value = crate::loading::literal_reader::decode_authenticated_native(
            &elements,
            held,
            |_| element.native_span(),
            ledger,
        )??;
        if let Some(value) = decode(value, ledger)? {
            values.push(value)?;
        }
    }
    Ok(values.finish())
}
