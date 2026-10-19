//! One original Foreground operation encloses decoding, encoding and its future Box.
use super::{PreparedPromptImage, validate_owned};
use crate::{
    history::{MAX_RESIDENT_BYTES, MeasurementError},
    images::{ImageCodecError, ImageInspector, PromptImageError},
    model::{
        json::deferred::{DecodeLedger, JsonDecodeError, NativeScratch},
        native::{NativeBytes, NativeOwned},
        retention::document::ConstructionRole,
    },
};
use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

/// Closed validation returns its original causes by value, without allocating
/// another shared error after a construction reservation has already failed.
#[derive(Debug)]
pub enum ImageOperationError {
    Image(PromptImageError),
    Construction(JsonDecodeError),
    Measurement(MeasurementError),
}
impl From<ImageCodecError> for ImageOperationError {
    fn from(error: ImageCodecError) -> Self {
        Self::Image(error.into())
    }
}
impl From<JsonDecodeError> for ImageOperationError {
    fn from(error: JsonDecodeError) -> Self {
        Self::Construction(error)
    }
}
impl From<MeasurementError> for ImageOperationError {
    fn from(error: MeasurementError) -> Self {
        Self::Measurement(error)
    }
}
impl fmt::Display for ImageOperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Image(error) => fmt::Display::fmt(error, f),
            Self::Construction(JsonDecodeError::Limit { .. }) => f.write_str("This chat exceeds the structural memory limit. Shorten the message or remove attachments."),
            Self::Construction(JsonDecodeError::Arithmetic) | Self::Measurement(MeasurementError::ArithmeticOverflow) => f.write_str("Saved chat measurement arithmetic overflow"),
            Self::Construction(JsonDecodeError::Json(error)) | Self::Measurement(MeasurementError::Json(error)) => write!(f, "Could not encode or decode the saved chat: {error}"),
            Self::Construction(error) => write!(f, "Saved chat validation failed: {error}"),
            Self::Measurement(MeasurementError::Retention(error)) => write!(f, "retained history ownership failed: {error}"),
            Self::Measurement(MeasurementError::Formatting(error)) => write!(f, "Saved chat formatted key measurement failed: {error}"),
        }
    }
}
impl Error for ImageOperationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Image(error) => Some(error),
            Self::Construction(error) => Some(error),
            Self::Measurement(error) => Some(error),
        }
    }
}
impl ImageOperationError {
    pub(super) fn into_prompt(self) -> PromptImageError {
        match self {
            Self::Image(error) => error,
            Self::Construction(error) => PromptImageError::Encoding(error.into()),
            Self::Measurement(error) => PromptImageError::Encoding(error.into()),
        }
    }
}

/// An original admitted byte copy and its independent whole-operation ledger.
pub struct ManagedImageRequest {
    data: NativeOwned<Vec<u8>>,
    ledger: DecodeLedger,
}
impl ManagedImageRequest {
    pub(crate) fn prepare(
        role: &ConstructionRole,
        bytes: &[u8],
    ) -> Result<Self, ImageOperationError> {
        crate::images::preflight(bytes)?;
        let ledger = DecodeLedger::for_operation(role, MAX_RESIDENT_BYTES)?;
        let mut data = NativeBytes::new(&ledger, bytes.len())?;
        data.append(bytes)?;
        Ok(Self {
            data: data.finish(),
            ledger,
        })
    }
    pub fn inspect(
        self,
        inspector: Arc<dyn ImageInspector>,
    ) -> Result<ManagedImageFuture, ImageOperationError> {
        let Self { data, ledger } = self;
        let original = ledger.share();
        let work = async move { validate_owned(&original, data, inspector.as_ref()) };
        let allocation = NativeScratch::reserve_native(&ledger, std::mem::size_of_val(&work))
            .map_err(JsonDecodeError::from)?;
        Ok(ManagedImageFuture {
            work: Box::pin(work),
            _allocation: allocation,
            _operation: ImageOperationClose(ledger),
        })
    }
}
struct ImageOperationClose(DecodeLedger);
impl Drop for ImageOperationClose {
    fn drop(&mut self) {
        self.0.close_image_operation();
    }
}
/// Drop order frees the actual future Box before its credit and operation close.
/// Becoming Ready leaves the operation open until this physical owner is dropped.
pub struct ManagedImageFuture {
    work: Pin<
        Box<
            dyn Future<Output = Result<NativeOwned<PreparedPromptImage>, ImageOperationError>>
                + Send,
        >,
    >,
    _allocation: NativeScratch,
    _operation: ImageOperationClose,
}
impl Future for ManagedImageFuture {
    type Output = Result<NativeOwned<PreparedPromptImage>, ImageOperationError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().work.as_mut().poll(cx)
    }
}
