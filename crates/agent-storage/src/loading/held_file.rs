//! One held descriptor and independently byte-bounded authenticated replay passes.
//! This is input safety, not a proof of allocations made by an arbitrary seed.
mod identity;
mod literal_failure;
pub(in crate::loading) mod managed;
mod pass;
use identity::{StrongIdentity, WeakIdentity};
use managed::{HeldPath, ReadFaultContext};
pub(super) mod plan;
pub(in crate::loading) mod replay_buffer;
mod spool;
#[cfg(test)]
mod tests;
#[cfg(test)]
use super::string_reader::spans::SpanProbe;
#[cfg(test)]
use nocterm_ai::history::PreflightFailure;
use nocterm_ai::history::{MAX_FILE_BYTES, PreflightBounds};
use nocterm_core::SharedError;
#[cfg(test)]
use serde::de::DeserializeSeed;
#[cfg(test)]
use std::io::Read as _;
use std::io::Seek;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(super) enum Operation {
    #[error("open")]
    Open,
    #[error("metadata")]
    Metadata,
    #[error("read")]
    Read,
    #[error("seek")]
    Seek,
    #[error("create anonymous replay")]
    CreateReplay,
    #[error("write anonymous replay")]
    WriteReplay,
    #[error("seek anonymous replay")]
    SeekReplay,
}
#[derive(Clone, Debug, thiserror::Error)]
pub(super) enum HeldReadError {
    #[error(transparent)]
    ManagedReplay(nocterm_ai::storage::AgentStorageError),
    #[cfg(not(target_os = "linux"))]
    #[error("Managed anonymous replay is not available on this platform")]
    ReplayPlatformUnavailable,
    #[error("Saved document native allocation failed: {0}")]
    Admission(#[source] nocterm_ai::model::json::deferred::NativeAllocationError),
    #[error("Saved document native read failed: {0}")]
    ManagedNative(#[source] nocterm_ai::storage::ManagedFileFailure),
    #[error("Saved literal document validation failed: {0}")]
    Literal(#[source] SharedError<super::literal_reader::LiteralReadError>),
    #[error("Saved literal document validation failed: {0}")]
    ManagedLiteral(#[source] nocterm_ai::model::failure::ManagedNativeFailure),
    #[error("{operation} {} failed: {source}", path.display())]
    Native {
        operation: Operation,
        path: PathBuf,
        #[source]
        source: SharedError<io::Error>,
    },
    #[error("Saved document exceeds its encoded limit")]
    EncodedLimit,
    #[error("Saved document exceeds its structural limit")]
    StructuralLimit,
    #[error("Saved document arithmetic overflow")]
    Arithmetic,
    #[cfg(test)]
    #[error("Saved document JSON failed: {0}")]
    Json(#[source] SharedError<serde_json::Error>),
    #[error("Saved document changed between read passes")]
    ChangedDocument,
    #[error("Read receipt belongs to another held source")]
    ForeignSource,
}
/// The original physical recursion failure of a source deeper than one stock
/// parser invocation.
#[derive(Debug)]
pub(super) struct DeepSource(HeldReadError);
impl DeepSource {
    /// Native, resource and replay failures of the later pass take precedence;
    /// any other failure keeps the original recursion failure.
    pub(super) fn or_native(self, later: HeldReadError) -> HeldReadError {
        match later {
            error @ (HeldReadError::ManagedReplay(_)
            | HeldReadError::Admission(_)
            | HeldReadError::ManagedNative(_)
            | HeldReadError::Native { .. }
            | HeldReadError::ChangedDocument) => error,
            _ => self.0,
        }
    }
    pub(super) fn into_error(self) -> HeldReadError {
        self.0
    }
}
impl HeldReadError {
    fn native(operation: Operation, path: &Path, error: io::Error) -> Self {
        Self::Native {
            operation,
            path: path.to_owned(),
            source: SharedError::from_error(error),
        }
    }
    #[cfg(test)]
    fn json(error: serde_json::Error) -> Self {
        Self::Json(SharedError::from_error(error))
    }
}
pub(super) struct HeldFile {
    file: File,
    path: HeldPath,
    identity: StrongIdentity,
    managed: Option<ReadFaultContext>,
    captured: Option<([u8; 32], u64)>,
}
/// Private receipt cannot be fabricated from an equal path or a supplied hash.
pub(super) struct ReadStamp {
    identity: WeakIdentity,
    digest: [u8; 32],
    bytes: u64,
    bounds: PreflightBounds,
}
impl ReadStamp {
    pub(super) fn bounds(&self) -> PreflightBounds {
        self.bounds
    }
    #[cfg(test)]
    pub(super) fn encoded_bytes(&self) -> u64 {
        self.bytes
    }
}
impl HeldFile {
    pub(super) fn plan_pass<'a>(&'a mut self, stamp: &'a ReadStamp) -> plan::PlanPass<'a> {
        plan::PlanPass::new(self, stamp)
    }
    #[cfg(test)]
    pub(super) fn open(path: &Path) -> Result<Self, HeldReadError> {
        // Deliberately retain existing File::open final-symlink semantics.
        let file = File::open(path)
            .map_err(|error| HeldReadError::native(Operation::Open, path, error))?;
        Self::from_opened(file, path.to_owned())
    }
    #[cfg(test)]
    fn from_opened(file: File, path: PathBuf) -> Result<Self, HeldReadError> {
        Self::from_source(
            file,
            HeldPath::Legacy(path),
            StrongIdentity::legacy(),
            None,
            None,
        )
    }
    pub(super) fn open_managed(
        path: managed::PaidPath,
        context: ReadFaultContext,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<Self, HeldReadError> {
        let identity = StrongIdentity::managed(ledger).map_err(HeldReadError::Admission)?;
        let scratch =
            managed::open_scratch(path.as_path(), ledger).map_err(HeldReadError::Admission)?;
        let file = File::open(path.as_path()).map_err(|error| {
            native_fault(Some(&context), Operation::Open, path.as_path(), error)
        })?;
        drop(scratch);
        Self::from_source(
            file,
            HeldPath::Managed(path),
            identity,
            Some(context),
            Some(ledger),
        )
    }
    fn from_source(
        mut file: File,
        path: HeldPath,
        identity: StrongIdentity,
        managed: Option<ReadFaultContext>,
        ledger: Option<&nocterm_ai::model::json::deferred::DecodeLedger>,
    ) -> Result<Self, HeldReadError> {
        let metadata = file
            .metadata()
            .map_err(|error| native_fault(managed.as_ref(), Operation::Metadata, &path, error))?;
        if metadata.len() > MAX_FILE_BYTES {
            return Err(HeldReadError::EncodedLimit);
        }
        let (file, captured) = if metadata.is_file() && file.stream_position().is_ok() {
            (file, None)
        } else {
            let (replay, fingerprint) =
                spool::capture_with_context(file, &path, managed.as_ref(), ledger)?.into_parts();
            (replay, Some(fingerprint))
        };
        Ok(Self {
            file,
            path,
            identity,
            managed,
            captured,
        })
    }
    fn rewind(&mut self) -> Result<(), HeldReadError> {
        self.file.rewind().map_err(|error| {
            native_fault(self.managed.as_ref(), Operation::Seek, &self.path, error)
        })
    }
    #[cfg(test)]
    pub(super) fn preflight(&mut self) -> Result<ReadStamp, HeldReadError> {
        self.rewind()?;
        let mut input = pass::PassReader::with_context(&mut self.file, self.managed.as_ref());
        let mut decoder =
            serde_json::Deserializer::from_reader(
                replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(&mut input),
            );
        let bounds =
            nocterm_ai::history::preflight_decoder(&mut decoder).map_err(|error| match error {
                PreflightFailure::StructuralLimit => HeldReadError::StructuralLimit,
                PreflightFailure::ArithmeticOverflow => HeldReadError::Arithmetic,
                PreflightFailure::Json(error) => HeldReadError::json(error),
            });
        let result =
            bounds.and_then(|bounds| decoder.end().map(|()| bounds).map_err(HeldReadError::json));
        drop(decoder);
        input.check_fault(&self.path)?;
        let bounds = result?;
        let (digest, bytes) = input.finish();
        self.authenticated_stamp(digest, bytes, bounds)
    }
    /// Contextual ordinary v2 JSON only. Legacy raw/default/skip acceptance must
    /// continue using its own grammar validation, never this global lexical metric.
    pub(super) fn preflight_literal(
        &mut self,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<ReadStamp, HeldReadError> {
        self.rewind()?;
        let validation =
            literal_failure::PreparedLiteralFailure::prepare(ledger, self.managed.is_some())?;
        let mut input = pass::PassReader::with_context(&mut self.file, self.managed.as_ref());
        let result = {
            let mut buffered =
                replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(&mut input);
            super::literal_reader::preflight(&mut buffered, ledger)
        };
        input.check_fault(&self.path)?;
        let bounds = result.map_err(|error| validation.capture(error))?;
        let (digest, bytes) = input.finish();
        self.authenticated_stamp(digest, bytes, bounds)
    }
    #[cfg(test)]
    pub(super) fn preflight_source(
        &mut self,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<ReadStamp, HeldReadError> {
        self.preflight_routed_source(ledger)?
            .map_err(DeepSource::into_error)
    }
    /// Original source K and physical recursion guard. Exceeding only the
    /// recursion guard is returned separately: routes other than ordinary v2
    /// still owe that original failure.
    pub(super) fn preflight_routed_source(
        &mut self,
        ledger: &nocterm_ai::model::json::deferred::DecodeLedger,
    ) -> Result<Result<ReadStamp, DeepSource>, HeldReadError> {
        self.rewind()?;
        let validation =
            literal_failure::PreparedLiteralFailure::prepare(ledger, self.managed.is_some())?;
        let mut input = pass::PassReader::with_context(&mut self.file, self.managed.as_ref());
        let result = {
            let mut buffered =
                replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(&mut input);
            super::literal_reader::original_source(&mut buffered, ledger)
        };
        input.check_fault(&self.path)?;
        let bounds = match result {
            Err(super::literal_reader::LiteralReadError::SourceDepth) => {
                let error = super::literal_reader::LiteralReadError::SourceDepth;
                return Ok(Err(DeepSource(validation.capture(error))));
            }
            result => result.map_err(|error| validation.capture(error))?,
        };
        let (digest, bytes) = input.finish();
        self.authenticated_stamp(digest, bytes, bounds).map(Ok)
    }
    fn authenticated_stamp(
        &self,
        digest: [u8; 32],
        bytes: u64,
        bounds: PreflightBounds,
    ) -> Result<ReadStamp, HeldReadError> {
        if self
            .captured
            .is_some_and(|original| original != (digest, bytes))
        {
            return Err(HeldReadError::ChangedDocument);
        }
        Ok(ReadStamp {
            identity: self.identity.downgrade(),
            digest,
            bytes,
            bounds,
        })
    }
    /// Independently limits native bytes even if the held inode grew or changed.
    /// The seed MUST independently control its allocations before digest equality
    /// can authorize adoption. This generic input capability supplies no such proof.
    #[cfg(test)]
    pub(super) fn replay_seed<S, T>(
        &mut self,
        stamp: &ReadStamp,
        seed: S,
    ) -> Result<T, HeldReadError>
    where
        S: for<'de> DeserializeSeed<'de, Value = T>,
    {
        let probe = SpanProbe::default();
        self.replay_spans(stamp, &probe, seed)
    }
    #[cfg(test)]
    pub(super) fn replay_spans<S, T>(
        &mut self,
        stamp: &ReadStamp,
        probe: &SpanProbe,
        seed: S,
    ) -> Result<T, HeldReadError>
    where
        S: for<'de> DeserializeSeed<'de, Value = T>,
    {
        if !stamp.identity.matches(&self.identity) {
            return Err(HeldReadError::ForeignSource);
        }
        self.rewind()?;
        let mut input = pass::PassReader::with_context(&mut self.file, self.managed.as_ref());
        probe.reset_pass();
        let source = probe.wrap(replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(
            &mut input,
        ));
        let mut decoder = serde_json::Deserializer::from_reader(source);
        let parsed = seed.deserialize(&mut decoder).map_err(HeldReadError::json);
        let result =
            parsed.and_then(|value| decoder.end().map(|()| value).map_err(HeldReadError::json));
        drop(decoder);
        input.check_fault(&self.path)?;
        // A changed malformed source may fail before EOF. Finish the bounded hash
        // before reporting that local syntax error; native failures still win.
        let mut drain = [0; 256];
        loop {
            match input.read(&mut drain) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => {
                    input.check_fault(&self.path)?;
                }
            }
        }
        input.check_fault(&self.path)?;
        let (digest, bytes) = input.finish();
        if digest != stamp.digest || bytes != stamp.bytes {
            return Err(HeldReadError::ChangedDocument);
        }
        result
    }
}

fn native_fault(
    context: Option<&ReadFaultContext>,
    operation: Operation,
    path: &Path,
    error: io::Error,
) -> HeldReadError {
    match context {
        Some(context) => HeldReadError::ManagedNative(context.capture(error)),
        None => HeldReadError::native(operation, path, error),
    }
}
