//! Managed native operations hold the original operation ledger through cleanup.
mod png;
use nocterm_ai::{
    images::{
        self, ImageCodecError, ImageFacts, ImageFormat, NativeImageAllocationScope,
        NativeImageErrorSlot, NativeImageScopeError, managed::ManagedImageBytes,
    },
    model::json::deferred::{DecodeLedger, NativeAllocationError},
};

#[derive(Debug, thiserror::Error)]
enum NativeFailure {
    #[error("{0}")]
    Image(#[from] image::ImageError),
    #[error("{0}")]
    Scope(#[from] NativeImageScopeError),
    #[error("{0}")]
    Allocation(#[from] ::png::budget::AllocationRefused),
    // The closed first-frame driver derives its exact output size from Reader.
    // Retain the original typed native API error if that invariant ever fails.
    #[error("The parameter is malformed: {0}")]
    Parameter(#[source] ::png::DecodingError),
}

pub(super) fn inspect(
    source: &ManagedImageBytes,
    ledger: &DecodeLedger,
) -> Result<ImageFacts, ImageCodecError> {
    let facts = images::preflight(source.bytes())?;
    if facts.format() != ImageFormat::Png {
        return Err(ImageCodecError::ManagedInspectionUnsupported);
    }
    let error = NativeImageErrorSlot::<NativeFailure>::prepare(
        ledger,
        std::mem::size_of::<::png::DecodingError>(),
    )
    .map_err(admission)?;
    let scope = NativeImageAllocationScope::begin(ledger).map_err(admission)?;
    let result = png::inspect(source, facts, &scope);
    // The original optional ICC parser may ignore a corrupt profile or a hook's
    // refusal. Genuine first admission failure takes precedence over its result.
    if let Err(failure) = scope.finish() {
        let limited = matches!(
            failure,
            NativeImageScopeError::Allocation(NativeAllocationError::Limit { .. })
        );
        let source = error.capture_with_nested(NativeFailure::Scope(failure), 0);
        return Err(if limited {
            ImageCodecError::ManagedAllocationLimit { source }
        } else {
            ImageCodecError::ManagedInvalidData { source }
        });
    }
    match result {
        Ok(facts) => Ok(facts),
        Err(png::Failure::Dimensions) => Err(ImageCodecError::Dimensions),
        Err(png::Failure::Evidence) => Err(ImageCodecError::InvalidEvidence),
        Err(png::Failure::Allocation(failure)) => Err(ImageCodecError::ManagedInvalidData {
            source: error.capture_with_nested(failure.into(), 0),
        }),
        Err(png::Failure::Decoder(failure)) => {
            let limited = matches!(failure, ::png::DecodingError::LimitsExceeded);
            let (failure, nested) = original_png_error(failure);
            let source = error.capture_with_nested(failure, nested);
            Err(if limited {
                ImageCodecError::ManagedAllocationLimit { source }
            } else {
                ImageCodecError::ManagedInvalidData { source }
            })
        }
    }
}
fn admission(source: NativeAllocationError) -> ImageCodecError {
    ImageCodecError::ManagedAdmission { source }
}
/// Original image0.25 PNG mapping, retaining the original concrete native error.
/// Its sole owned native cause Box is admitted before parser construction.
fn original_png_error(error: ::png::DecodingError) -> (NativeFailure, usize) {
    use ::png::DecodingError;
    match error {
        DecodingError::IoError(error) => (image::ImageError::IoError(error).into(), 0),
        error @ DecodingError::Format(_) => (
            image::ImageError::Decoding(image::error::DecodingError::new(
                image::ImageFormat::Png.into(),
                error,
            ))
            .into(),
            std::mem::size_of::<DecodingError>(),
        ),
        error @ DecodingError::Parameter(_) => (NativeFailure::Parameter(error), 0),
        DecodingError::LimitsExceeded => (
            image::ImageError::Limits(image::error::LimitError::from_kind(
                image::error::LimitErrorKind::InsufficientMemory,
            ))
            .into(),
            0,
        ),
    }
}

#[cfg(test)]
mod tests;
