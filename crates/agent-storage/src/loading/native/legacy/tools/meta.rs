//! Historical displays move only after full borrowed shape validation.
use super::super::super::Error;
use nocterm_ai::model::{
    controls::Presence,
    extensions::Extensions,
    json::deferred::{
        DecodeLedger, JsonDecodeError, NativeScratch,
        native::{NativeKind, NativeNode, NativeObject, NativeText},
    },
    native::NativeOwned,
    shared_display::SharedDisplay,
    tools::DisplayOutcome,
};
use std::mem::size_of;
/// Actual guarded entries retain original ordering until extraction succeeds.
struct Object {
    entries: Vec<Option<(NativeText, NativeNode)>>,
    _allocation: NativeScratch,
}
impl Object {
    fn read(node: NativeNode, ledger: &DecodeLedger) -> std::result::Result<Self, Error> {
        if node.kind() != NativeKind::Object {
            return Err(Error::Type);
        }
        let len = node.container_len().ok_or(Error::Type)?;
        let allocation = NativeScratch::reserve(
            ledger,
            len.checked_mul(size_of::<Option<(NativeText, NativeNode)>>())
                .ok_or(JsonDecodeError::Arithmetic)?,
        )?;
        let mut entries = Vec::new();
        entries.try_reserve_exact(len)?;
        if entries.capacity() != len {
            return Err(JsonDecodeError::CapacityProfile.into());
        }
        let mut children = node.into_children()?;
        while let Some((key, node)) = children.next_child()? {
            entries.push(Some((key.ok_or(Error::Type)?, node)))
        }
        Ok(Self {
            entries,
            _allocation: allocation,
        })
    }
    fn get(&self, name: &str) -> Option<&NativeNode> {
        self.entries
            .iter()
            .flatten()
            .find(|(key, _)| key.as_str() == name)
            .map(|(_, node)| node)
    }
    fn take(&mut self, name: &str) -> Option<(NativeText, NativeNode)> {
        self.entries
            .iter_mut()
            .find(|entry| entry.as_ref().is_some_and(|(key, _)| key.as_str() == name))
            .and_then(Option::take)
    }
    fn restore(self, ledger: &DecodeLedger) -> std::result::Result<NativeNode, Error> {
        let mut result = NativeObject::new(ledger, self.entries.iter().flatten().count())?;
        for (key, value) in self.entries.into_iter().flatten() {
            result.push(key, value)?
        }
        Ok(result.finish()?)
    }
}
type DisplayOrder = crate::legacy::ControlledFieldOrder;
type Metadata = (
    NativeOwned<Option<Extensions>>,
    NativeOwned<Option<SharedDisplay>>,
    Option<(usize, DisplayOrder)>,
);
pub(in crate::loading::native) fn metadata(
    node: Option<NativeNode>,
    ledger: &DecodeLedger,
) -> std::result::Result<Metadata, Error> {
    let Some(node) = node else {
        return Ok((
            NativeOwned::<Extensions>::absent(ledger)?,
            NativeOwned::<SharedDisplay>::absent(ledger)?,
            None,
        ));
    };
    let mut meta = Object::read(node, ledger)?;
    let index = meta.entries.iter().position(|entry| {
        entry
            .as_ref()
            .is_some_and(|(key, _)| key.as_str() == "nocterm/tool-display")
    });
    let mut display = NativeOwned::<SharedDisplay>::absent(ledger)?;
    let mut order = None;
    if let Some(index) = index {
        let (key, node) = meta.entries[index].take().ok_or(Error::Type)?;
        match extract(node, ledger)? {
            Ok((value, fields)) => {
                display = NativeOwned::supplied(value);
                order = Some((index, fields));
            }
            Err(node) => meta.entries[index] = Some((key, node)),
        }
    }
    let extensions = NativeOwned::supplied(nocterm_ai::model::native::content::extensions(
        meta.restore(ledger)?,
        ledger,
    )?);
    Ok((extensions, display, order))
}
fn extract(
    node: NativeNode,
    ledger: &DecodeLedger,
) -> std::result::Result<
    std::result::Result<(NativeOwned<SharedDisplay>, DisplayOrder), NativeNode>,
    Error,
> {
    if node.kind() != NativeKind::Object {
        return Ok(Err(node));
    }
    let mut object = Object::read(node, ledger)?;
    if !valid(&object) {
        return Ok(Err(object.restore(ledger)?));
    }
    let fields = crate::legacy::controlled_field_order(
        object.entries.iter().flatten().map(|(key, _)| key.as_str()),
    );
    let outcome = match object.take("outcome") {
        None => NativeOwned::<DisplayOutcome>::not_supplied(ledger)?,
        Some((_, node)) if node.kind() == NativeKind::Null => {
            NativeOwned::<DisplayOutcome>::null(ledger)?
        }
        Some((key, node)) => match decode_outcome(node, ledger)? {
            Ok(value) => value.presence(),
            Err(node) => {
                let slot = object
                    .entries
                    .iter_mut()
                    .find(|entry| entry.is_none())
                    .ok_or(Error::Type)?;
                *slot = Some((key, node));
                return Ok(Err(object.restore(ledger)?));
            }
        },
    };
    let version = object
        .take("version")
        .and_then(|(_, v)| v.number().and_then(|n| n.as_u64()))
        .ok_or(Error::Type)? as u8;
    let tool = NativeOwned::text(object.take("tool").ok_or(Error::Type)?.1.into_text()?);
    let arguments = nocterm_ai::model::native::content::json(
        object.take("arguments").ok_or(Error::Type)?.1,
        ledger,
    )?;
    let destination = match object.take("destination") {
        None => NativeOwned::<String>::not_supplied(ledger)?,
        Some((_, node)) if node.kind() == NativeKind::Null => NativeOwned::<String>::null(ledger)?,
        Some((_, node)) => NativeOwned::text(node.into_text()?).presence(),
    };
    let redacted = match object.take("redacted") {
        None => Presence::Absent,
        Some((_, node)) => Presence::Value(node.boolean().ok_or(Error::Type)?),
    };
    Ok(Ok((
        nocterm_ai::model::native::tools::shared(nocterm_ai::model::native::tools::display(
            version,
            tool,
            arguments,
            destination,
            redacted,
            outcome,
        )?)?,
        fields,
    )))
}
fn valid(object: &Object) -> bool {
    object.entries.iter().flatten().all(|(key, _)| {
        matches!(
            key.as_str(),
            "version" | "tool" | "arguments" | "destination" | "redacted" | "outcome"
        )
    }) && matches!(
        object
            .get("version")
            .and_then(|v| v.number())
            .and_then(|n| n.as_u64()),
        Some(1 | 2)
    ) && object
        .get("tool")
        .is_some_and(|v| v.kind() == NativeKind::String)
        && object.get("arguments").is_some()
        && object
            .get("destination")
            .is_none_or(|v| matches!(v.kind(), NativeKind::Null | NativeKind::String))
        && object
            .get("redacted")
            .is_none_or(|v| v.kind() == NativeKind::Boolean)
}
fn decode_outcome(
    node: NativeNode,
    ledger: &DecodeLedger,
) -> std::result::Result<std::result::Result<NativeOwned<DisplayOutcome>, NativeNode>, Error> {
    if node.kind() == NativeKind::String {
        return if node.text() == Some("Pending") {
            Ok(Ok(nocterm_ai::model::native::tools::pending(ledger)?))
        } else {
            Ok(Err(node))
        };
    }
    if node.kind() != NativeKind::Object {
        return Ok(Err(node));
    }
    let mut object = Object::read(node, ledger)?;
    if object.entries.len() != 1 {
        return Ok(Err(object.restore(ledger)?));
    }
    if object.get("Ok").is_some() {
        return Ok(Ok(nocterm_ai::model::native::tools::succeeded(
            nocterm_ai::model::native::content::json(
                object.take("Ok").ok_or(Error::Type)?.1,
                ledger,
            )?,
        )));
    }
    if object
        .get("Err")
        .is_some_and(|v| v.kind() == NativeKind::String)
    {
        return Ok(Ok(nocterm_ai::model::native::tools::failed(
            NativeOwned::text(object.take("Err").ok_or(Error::Type)?.1.into_text()?),
        )));
    }
    Ok(Err(object.restore(ledger)?))
}
