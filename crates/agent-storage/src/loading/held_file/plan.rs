//! Current-pass plan authentication. No separately replayed value is trusted here.
use super::{HeldFile, HeldReadError, ReadStamp, pass::PassReader};
use sha2::{Digest, Sha256};
use std::io::{self, Read, Seek, SeekFrom};
/// Constructor private to the held source; provisional span digests cannot create it.
pub(in crate::loading) struct AuthenticatedPlan<T> {
    payload: T,
    identity: super::identity::WeakIdentity,
    parent_digest: [u8; 32],
}
/// A digest from an actual framed span; unusable without its authenticated parent.
#[derive(Clone)]
pub(in crate::loading) struct NativeSpan {
    start: usize,
    value_bytes: usize,
    covered_bytes: usize,
    digest: [u8; 32],
}
impl NativeSpan {
    pub(in crate::loading) fn start(&self) -> usize {
        self.start
    }
    pub(in crate::loading) fn value_bytes(&self) -> usize {
        self.value_bytes
    }
    pub(in crate::loading) fn from_scan(
        start: usize,
        value_bytes: usize,
        covered_end: usize,
        digest: [u8; 32],
    ) -> Self {
        Self {
            start,
            value_bytes,
            covered_bytes: covered_end - start,
            digest,
        }
    }
}
impl<T> AuthenticatedPlan<T> {
    /// Selection must come from a span framed by THIS payload's authenticated
    /// scan. Kept adapter-private; no caller-supplied hashes cross a crate port.
    pub(in crate::loading) fn replay<U>(
        &self,
        held: &mut HeldFile,
        select: impl FnOnce(&T) -> NativeSpan,
        inspect: impl FnOnce(&mut dyn Read) -> U,
    ) -> Result<U, HeldReadError> {
        if !self.identity.matches(&held.identity) {
            return Err(HeldReadError::ForeignSource);
        }
        let span = select(&self.payload);
        if span.covered_bytes > nocterm_ai::history::MAX_FILE_BYTES as usize
            || span.value_bytes > span.covered_bytes
        {
            return Err(HeldReadError::EncodedLimit);
        }
        held.file
            .seek(SeekFrom::Start(span.start as u64))
            .map_err(|error| {
                super::native_fault(
                    held.managed.as_ref(),
                    super::Operation::Seek,
                    &held.path,
                    error,
                )
            })?;
        let mut source = SpanReader {
            source: (&mut held.file).take(span.covered_bytes as u64),
            digest: Sha256::new(),
            bytes: 0,
            failure: None,
            context: held.managed.as_ref(),
        };
        let result = {
            let value = (&mut source).take(span.value_bytes as u64);
            let mut buffered = super::replay_buffer::ReplayBuffer::new(value);
            inspect(&mut buffered)
        };
        if let Some(failure) = source
            .context
            .and_then(super::managed::ReadFaultContext::failure)
        {
            return Err(HeldReadError::ManagedNative(failure));
        }
        if let Some(source) = &source.failure {
            return Err(HeldReadError::Native {
                operation: super::Operation::Read,
                path: held.path.to_path_buf(),
                source: source.clone(),
            });
        }
        let mut drain = [0; 256];
        loop {
            match source.read(&mut drain) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if let Some(source) = &source.failure {
                        return Err(HeldReadError::Native {
                            operation: super::Operation::Read,
                            path: held.path.to_path_buf(),
                            source: source.clone(),
                        });
                    }
                    if let Some(failure) = source
                        .context
                        .and_then(super::managed::ReadFaultContext::failure)
                    {
                        return Err(HeldReadError::ManagedNative(failure));
                    }
                    return Err(HeldReadError::native(
                        super::Operation::Read,
                        &held.path,
                        error,
                    ));
                }
            }
        }
        if source.bytes != span.covered_bytes
            || <[u8; 32]>::from(source.digest.finalize()) != span.digest
        {
            return Err(HeldReadError::ChangedDocument);
        }
        Ok(result)
    }
    /// A nested plan becomes authentic only after the exact parent-selected
    /// bytes delivered to its scanner have matched their authenticated digest.
    /// Absolute offsets must originate from the selected native span.
    pub(in crate::loading) fn derive<U>(
        &self,
        held: &mut HeldFile,
        select: impl FnOnce(&T) -> NativeSpan,
        inspect: impl FnOnce(&mut dyn Read) -> U,
    ) -> Result<AuthenticatedPlan<U>, HeldReadError> {
        let payload = self.replay(held, select, inspect)?;
        Ok(AuthenticatedPlan {
            payload,
            identity: self.identity.clone(),
            parent_digest: self.parent_digest,
        })
    }
    /// Semantic native children inherit only authenticated origin, never a new
    /// physical span or permission to replay bytes under their semantic layout.
    pub(in crate::loading) fn native_scope(
        &self,
        native: crate::loading::native::schema::NativeScope,
    ) -> AuthenticatedPlan<crate::loading::native::schema::Scope> {
        AuthenticatedPlan {
            payload: crate::loading::native::schema::Scope::adopt(native),
            identity: self.identity.clone(),
            parent_digest: self.parent_digest,
        }
    }
    pub(in crate::loading) fn payload(&self) -> &T {
        &self.payload
    }
}
impl<T, E> AuthenticatedPlan<Result<T, E>> {
    pub(in crate::loading) fn transpose(self) -> Result<AuthenticatedPlan<T>, E> {
        Ok(AuthenticatedPlan {
            payload: self.payload?,
            identity: self.identity,
            parent_digest: self.parent_digest,
        })
    }
}
/// Bytes handed to semantic validation are exactly the bytes hashed, independent
/// of buffered scan state or later mutations of the descriptor.
struct SpanReader<'a, R> {
    context: Option<&'a super::managed::ReadFaultContext>,
    source: R,
    digest: Sha256,
    bytes: usize,
    failure: Option<nocterm_core::SharedError<io::Error>>,
}
impl<R: Read> Read for SpanReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self
            .context
            .and_then(super::managed::ReadFaultContext::failure)
            .is_some()
        {
            return Err(io::ErrorKind::Other.into());
        }
        let count = match self.source.read(output) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                if let Some(context) = self.context {
                    context.capture(error);
                    return Err(io::ErrorKind::Other.into());
                }
                let source = nocterm_core::SharedError::from_error(error);
                self.failure = Some(source.clone());
                return Err(io::Error::other(source));
            }
        };
        self.digest.update(&output[..count]);
        self.bytes += count;
        Ok(count)
    }
}
pub(in crate::loading) struct PlanPass<'a> {
    held: &'a mut HeldFile,
    stamp: &'a ReadStamp,
}
impl<'a> PlanPass<'a> {
    pub(super) fn new(held: &'a mut HeldFile, stamp: &'a ReadStamp) -> Self {
        Self { held, stamp }
    }
    /// The inspector must independently bound its allocations. Authentication is
    /// performed AFTER inspection and authorizes only plans produced from these
    /// exact observed bytes, never different bytes read by a later seek replay.
    pub(in crate::loading) fn scan<T>(
        self,
        inspect: impl FnOnce(&mut dyn Read) -> T,
    ) -> Result<AuthenticatedPlan<T>, HeldReadError> {
        if !self.stamp.identity.matches(&self.held.identity) {
            return Err(HeldReadError::ForeignSource);
        }
        self.held.rewind()?;
        let mut input = PassReader::with_context(&mut self.held.file, self.held.managed.as_ref());
        let payload = {
            let mut buffered =
                super::replay_buffer::ReplayBuffer::<_, 256>::with_inline_capacity(&mut input);
            let result = inspect(&mut buffered);
            // Consume the buffered remainder before dropping the buffer so the
            // inspector's read-ahead is never counted twice or skipped in hash.
            let mut drain = [0; 256];
            loop {
                match buffered.read(&mut drain) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            result
        };
        input.check_fault(&self.held.path)?;
        let (digest, bytes) = input.finish();
        if (digest, bytes) != (self.stamp.digest, self.stamp.bytes) {
            return Err(HeldReadError::ChangedDocument);
        }
        Ok(AuthenticatedPlan {
            payload,
            identity: self.stamp.identity.clone(),
            parent_digest: digest,
        })
    }
}
