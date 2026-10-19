//! Original UTF-8 backing and checked source spans; no parsed Value graph.
use super::{AllocationKind, Budget, Buffer, ManagedError, Shared};
use std::ops::Range;

struct Text<C, B> {
    value: String,
    _credit: Option<C>,
    budget: B,
}
/// One actual UTF-8 backing allocation retained by authentic source identity.
pub struct Source<B: Budget> {
    text: Shared<Text<B::Credit, B>, B>,
}
impl<B: Budget> Clone for Source<B> {
    fn clone(&self) -> Self {
        Self {
            text: self.text.clone(),
        }
    }
}
impl<B: Budget> Source<B> {
    /// Start an original prepaid native reader/encoder buffer.
    pub fn buffer(budget: B) -> Buffer<u8, B> {
        Buffer::with_kind(budget, AllocationKind::Source)
    }
    /// Consume actual prepaid bytes without copying their backing.
    pub fn from_buffer(buffer: Buffer<u8, B>) -> Result<Self, ManagedError<B::Error>> {
        let (bytes, credit, budget) = buffer.into_parts();
        let value = String::from_utf8(bytes).map_err(|_| ManagedError::InvalidUtf8)?;
        Ok(Self {
            text: Shared::new(
                Text {
                    value,
                    _credit: credit,
                    budget: budget.clone(),
                },
                &budget,
            )?,
        })
    }
    /// Copy borrowed source text only after exact native approval.
    pub fn copy(text: &str, budget: B) -> Result<Self, ManagedError<B::Error>> {
        let mut buffer = Self::buffer(budget);
        buffer.extend_bytes(text.as_bytes())?;
        Self::from_buffer(buffer)
    }
    /// Borrow original UTF-8 bytes while retaining their source guard.
    pub fn text(&self) -> &str {
        &self.text.get().value
    }
    /// Keep a checked UTF-8 range in the original source, without copying.
    pub fn span(&self, range: Range<usize>) -> Result<Span<B>, ManagedError<B::Error>> {
        if self.text().get(range.clone()).is_none() {
            return Err(ManagedError::InvalidSpan);
        }
        Ok(Span {
            source: self.clone(),
            range,
        })
    }
    /// Borrow the complete source range, without allocating a second owner.
    pub fn full(&self) -> Span<B> {
        Span {
            source: self.clone(),
            range: 0..self.text().len(),
        }
    }
    /// Test authentic budget context identity without retaining a new context.
    pub fn belongs_to(&self, budget: &B) -> bool {
        self.text.get().budget.same_source(budget)
    }
    /// Test genuine original source identity, independently of byte equality.
    pub fn same_source(&self, other: &Self) -> bool {
        self.text.same_allocation(&other.text)
    }
}
impl<B: Budget> Buffer<u8, B> {
    /// Append native bytes, prepaying the exact old/new backing overlap.
    pub fn extend_bytes(&mut self, bytes: &[u8]) -> Result<(), ManagedError<B::Error>> {
        self.reserve_amortized(bytes.len())?;
        for &byte in bytes {
            // Capacity was reserved as one operation; push cannot allocate.
            if let Err((error, _)) = self.push(byte) {
                return Err(error);
            }
        }
        Ok(())
    }
}
/// An immutable checked range retaining its genuine original UTF-8 source.
pub struct Span<B: Budget> {
    source: Source<B>,
    range: Range<usize>,
}
impl<B: Budget> Clone for Span<B> {
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            range: self.range.clone(),
        }
    }
}
impl<B: Budget> Span<B> {
    /// Test authentic budget context identity without retaining a new context.
    pub fn belongs_to(&self, budget: &B) -> bool {
        self.source.belongs_to(budget)
    }
    /// Borrow the actual retained source range.
    pub fn text(&self) -> &str {
        &self.source.text()[self.range.clone()]
    }
    /// Borrow the original backing owner; the whole backing remains charged.
    pub fn source(&self) -> &Source<B> {
        &self.source
    }
    /// Original UTF-8 byte range inside the retained backing source.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }
    /// Create a checked nested span relative to this range.
    pub fn slice(&self, range: Range<usize>) -> Result<Self, ManagedError<B::Error>> {
        if self.text().get(range.clone()).is_none() {
            return Err(ManagedError::InvalidSpan);
        }
        let start = self
            .range
            .start
            .checked_add(range.start)
            .ok_or(ManagedError::InvalidSpan)?;
        let end = self
            .range
            .start
            .checked_add(range.end)
            .ok_or(ManagedError::InvalidSpan)?;
        self.source.span(start..end)
    }
}
