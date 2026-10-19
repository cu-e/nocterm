//! Borrowed source values materialize only through genuine neutral constructors.
use super::{budget::SourceBudget, error::DecodeError, fields::Fields, scan::Scanner};
use agent_client_protocol::managed::{Source, Span};
use nocterm_ai::model::{
    json::deferred::{
        JsonDecodeError,
        literal::{LiteralBuilder, LiteralText},
        native::NativeNode,
    },
    native::NativeOwned,
};
use std::ops::Range;

type Error = DecodeError<JsonDecodeError>;
#[derive(Clone)]
pub(super) struct Value<'a> {
    pub(super) source: &'a Source<SourceBudget>,
    pub(super) range: Range<usize>,
    pub(super) budget: &'a SourceBudget,
}
impl<'a> Value<'a> {
    pub(super) fn new(
        source: &'a Source<SourceBudget>,
        range: Range<usize>,
        budget: &'a SourceBudget,
    ) -> Self {
        Self {
            source,
            range,
            budget,
        }
    }
    pub(super) fn from_span(
        span: &'a Span<SourceBudget>,
        budget: &'a SourceBudget,
    ) -> Result<Self, Error> {
        if !span.belongs_to(budget) {
            return Err(DecodeError::Allocation(
                agent_client_protocol::managed::ManagedError::InvalidEnvelope,
            ));
        }
        Ok(Self::new(span.source(), span.range(), budget))
    }
    pub(super) fn ledger(&self) -> &nocterm_ai::model::json::deferred::DecodeLedger {
        self.budget.ledger()
    }
    pub(super) fn raw(&self) -> &str {
        &self.source.text()[self.range.clone()]
    }
    pub(super) fn fields<const N: usize>(
        &self,
        names: [&str; N],
    ) -> Result<[Option<Self>; N], Error> {
        Ok(
            Fields::new(self.source.text(), self.range.clone(), self.budget.clone())?
                .select(names)?
                .map(|range| range.map(|range| Self::new(self.source, range, self.budget))),
        )
    }
    pub(super) fn required(value: Option<Self>, field: &'static str) -> Result<Self, Error> {
        value.ok_or(DecodeError::Field(field))
    }
    pub(super) fn equals(&self, expected: &str) -> Result<bool, Error> {
        Scanner::new(self.source.text(), self.budget.clone())
            .key_equals(self.range.clone(), expected)
    }
    pub(super) fn text(&self) -> Result<NativeOwned<String>, Error> {
        if !self.raw().starts_with('"') {
            return Err(DecodeError::Field("string"));
        }
        let mut builder = LiteralBuilder::new(self.ledger())?;
        builder.string(self.literal_text(&builder)?)?;
        Ok(NativeOwned::text(builder.finish_native()?.into_text()?))
    }
    pub(super) fn copy_text(&self, text: &str) -> Result<NativeOwned<String>, Error> {
        let mut builder = LiteralBuilder::new(self.ledger())?;
        let mut value = builder.exact_text(text.len())?;
        value.append(text)?;
        builder.string(value.finish()?)?;
        Ok(NativeOwned::text(builder.finish_native()?.into_text()?))
    }
    pub(super) fn literal_text(&self, builder: &LiteralBuilder) -> Result<LiteralText, Error> {
        let scanner = Scanner::new(self.source.text(), self.budget.clone());
        let mut count = 0usize;
        scanner.decoded_string(self.range.clone(), |part| {
            count = count
                .checked_add(part.len())
                .ok_or(DecodeError::Field("string length"))?;
            Ok(())
        })?;
        let mut text = builder.exact_text(count)?;
        scanner.decoded_string(self.range.clone(), |part| {
            text.append(part)?;
            Ok(())
        })?;
        Ok(text.finish()?)
    }
    /// The complete source grammar was validated before routing. This traversal
    /// owns no bare serde tree: every scalar, key and container is constructed
    /// by the domain's prepaid builder and has its certified cleanup frontier.
    pub(super) fn literal(&self) -> Result<NativeNode, Error> {
        self.literal_filtered(None)
    }
    pub(super) fn literal_excluding(&self, key: &'static str) -> Result<NativeNode, Error> {
        if !self.raw().starts_with('{') {
            return Err(DecodeError::Field("metadata object"));
        }
        self.literal_filtered(Some(key))
    }
    fn literal_filtered(&self, excluded: Option<&str>) -> Result<NativeNode, Error> {
        super::literal::build(self, excluded)
    }
}

pub(super) struct Elements<'a> {
    value: Value<'a>,
    scanner: Scanner<'a, SourceBudget>,
    position: usize,
}
impl<'a> Value<'a> {
    pub(super) fn elements(&self) -> Result<Elements<'a>, Error> {
        if !self.raw().starts_with('[') {
            return Err(DecodeError::Field("array"));
        }
        Ok(Elements {
            value: self.clone(),
            scanner: Scanner::new(self.source.text(), self.budget.clone()),
            position: self.range.start + 1,
        })
    }
    pub(super) fn array_len(&self) -> Result<usize, Error> {
        let mut elements = self.elements()?;
        let mut count = 0;
        while elements.next()?.is_some() {
            count += 1;
        }
        Ok(count)
    }
}
impl<'a> Elements<'a> {
    pub(super) fn next(&mut self) -> Result<Option<Value<'a>>, Error> {
        self.position = self.scanner.whitespace(self.position);
        if self.value.source.text().as_bytes().get(self.position) == Some(&b']') {
            return Ok(None);
        }
        let start = self.position;
        let end = self.scanner.value(start)?;
        self.position = self.scanner.whitespace(end);
        if self.value.source.text().as_bytes().get(self.position) == Some(&b',') {
            self.position += 1;
        }
        Ok(Some(Value::new(
            self.value.source,
            start..end,
            self.value.budget,
        )))
    }
}
