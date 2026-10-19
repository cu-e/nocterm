//! One held authenticated document reaches terminal admission in its own ledger.
use super::{
    Error, document, legacy,
    route::{RouteError, RoutePlan},
    schema::{self as s, Scope},
};
use crate::{
    format::Format,
    loading::{held_file::HeldFile, json_fields::Key},
};
use nocterm_ai::{
    history::{HistoryOrigin, NativeMaterialized},
    model::json::deferred::DecodeLedger,
    storage::AgentStorageError,
};
#[cfg(test)]
use nocterm_ai::{
    history::{NativePhase, ReaderCompletion},
    model::retention::document::{ConstructionRole, RetainedRole},
};
use std::path::Path;
#[cfg(test)]
pub(crate) fn read(
    dir: &Path,
    id: &str,
    reader: ConstructionRole,
    delivery: RetainedRole,
) -> ReaderCompletion {
    let phase = match NativePhase::begin(reader, delivery) {
        Ok(phase) => phase,
        Err(completion) => return completion,
    };
    phase.read(|ledger| materialize(dir, id, ledger))
}
#[cfg(test)]
pub(crate) fn read_request(
    dir: &Path,
    id: &str,
    request: nocterm_ai::history::ReaderRequest,
) -> nocterm_ai::history::ReaderDelivery {
    request.read(|ledger| materialize(dir, id, ledger))
}
pub(crate) fn materialize(
    dir: &Path,
    id: &str,
    ledger: &DecodeLedger,
) -> std::result::Result<NativeMaterialized, AgentStorageError> {
    let validation = super::failure_capture::FailureCapture::prepare(ledger)?;
    let context = crate::loading::held_file::managed::ReadFaultContext::prepare(dir, id, ledger)?;
    let path = crate::writing::native_paths::document(dir, id, ".json", ledger)?;
    let mut held = match HeldFile::open_managed(path, context, ledger) {
        Ok(held) => held,
        Err(error) => return Err(validation.held(error, id)),
    };
    let result = (|| -> std::result::Result<_, Error> {
        // The original format classifier checks raw-source K/depth before
        // examining format/version, including unsupported routes.
        let (stamp, format) = match held.preflight_routed_source(ledger)? {
            Ok(stamp) => {
                let format = classify(&mut held, &stamp, ledger)?;
                (stamp, format)
            }
            // Ordinary v2 JSON may nest legacy RawValue expansions past one
            // stock parser budget; its iterative reader has no such limit.
            Err(deep) => {
                let stamp = match held.preflight_literal(ledger) {
                    Ok(stamp) => stamp,
                    Err(error) => return Err(deep.or_native(error).into()),
                };
                match classify(&mut held, &stamp, ledger) {
                    Ok(Format::Version2) => (stamp, Format::Version2),
                    Err(Error::Held(error) | Error::Route(RouteError::Held(error))) => {
                        return Err(deep.or_native(error).into());
                    }
                    Err(Error::Route(RouteError::Admission(error))) => {
                        return Err(RouteError::Admission(error).into());
                    }
                    Ok(Format::LegacyV1) | Err(_) => return Err(deep.into_error().into()),
                }
            }
        };
        let source_resident = stamp.bounds().structural_bytes;
        let (chat, origin, grammar) = match format {
            Format::Version2 => {
                let envelope = held
                    .plan_pass(&stamp)
                    .scan(|reader| Scope::scan_with_ledger(reader, 0, s::ENVELOPE, Some(ledger)))?
                    .transpose()?;
                let document =
                    super::fields::child(&envelope, &mut held, Key::Document, s::DOCUMENT, ledger)?;
                (
                    document::document(&document, &mut held, ledger, id)?,
                    HistoryOrigin::Version2,
                    None,
                )
            }
            Format::LegacyV1 => {
                let document = held
                    .plan_pass(&stamp)
                    .scan(|reader| {
                        Scope::scan_with_ledger(reader, 0, s::LEGACY_DOCUMENT, Some(ledger))
                    })?
                    .transpose()?;
                let rows = super::lists::field_count(&document, &mut held, Key::Entries, 2000)?;
                let mut grammar = crate::legacy::ControlledGrammar::new(ledger, rows)?;
                let chat =
                    legacy::document::document(&document, &mut held, ledger, id, &mut grammar)?;
                (chat, HistoryOrigin::LegacyV1, Some(grammar.finish(ledger)?))
            }
        };
        // Authenticate all lexical bytes again, including ignored fields, before
        // publication of any typed owner. Field replays already checked digests.
        held.plan_pass(&stamp).scan(|_| ())?;
        Ok(NativeMaterialized::new(
            chat,
            origin,
            grammar,
            source_resident,
        ))
    })();
    result.map_err(|error| validation.storage(error, id))
}
fn classify(
    held: &mut HeldFile,
    stamp: &crate::loading::held_file::ReadStamp,
    ledger: &DecodeLedger,
) -> std::result::Result<Format, Error> {
    let route = held
        .plan_pass(stamp)
        .scan(|reader| RoutePlan::scan(reader, ledger))?
        .transpose()
        .map_err(Error::Route)?;
    RoutePlan::classify(&route, held, ledger).map_err(Error::Route)
}
