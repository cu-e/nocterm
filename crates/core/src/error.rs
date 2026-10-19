//! Immutable shared causes preserve the original concrete error in source chains.
use std::{error::Error, fmt, sync::Arc};

/// Cheaply clone an error without requiring the original error to implement Clone.
///
/// The holder is one source frame; the next frame is the original error, including
/// when that error has no source of its own. No diagnostic text or payload is copied.
pub struct SharedError<E: Error + Send + Sync + 'static + ?Sized = dyn Error + Send + Sync> {
    inner: Arc<E>,
}

/// A shared cause whose concrete type can be recovered through immutable downcasts.
pub type SharedCause = SharedError<dyn Error + Send + Sync>;

impl<E: Error + Send + Sync + 'static> SharedError<E> {
    /// Retain an owned, statically typed error.
    pub fn from_error(error: E) -> Self {
        Self::from_arc(Arc::new(error))
    }
}

impl SharedCause {
    /// Retain an owned error while erasing only its static type.
    pub fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self::from_arc(Arc::new(error))
    }

    pub fn downcast_ref<E: Error + 'static>(&self) -> Option<&E> {
        self.inner.downcast_ref()
    }
}

impl<E: Error + Send + Sync + 'static + ?Sized> SharedError<E> {
    /// Reuse an existing allocation without cloning its payload.
    pub fn from_arc(inner: Arc<E>) -> Self {
        Self { inner }
    }

    /// Transfer the existing shared payload into an older typed boundary.
    /// Consuming the holder preserves its allocation and all surviving aliases.
    pub fn into_arc(self) -> Arc<E> {
        self.inner
    }
}

impl<E: Error + Send + Sync + 'static + ?Sized> AsRef<E> for SharedError<E> {
    fn as_ref(&self) -> &E {
        self.inner.as_ref()
    }
}

impl<E: Error + Send + Sync + 'static + ?Sized> Clone for SharedError<E> {
    fn clone(&self) -> Self {
        Self::from_arc(self.inner.clone())
    }
}

impl<E: Error + Send + Sync + 'static + ?Sized> fmt::Debug for SharedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_ref(), f)
    }
}

impl<E: Error + Send + Sync + 'static + ?Sized> fmt::Display for SharedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_ref(), f)
    }
}

impl<E: Error + Send + Sync + 'static> Error for SharedError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.as_ref())
    }
}

impl Error for SharedCause {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.as_ref())
    }
}

#[cfg(test)]
mod tests;
