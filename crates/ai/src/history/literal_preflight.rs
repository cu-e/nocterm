//! Exact existing lexical v2 N/T metric, independent of any recursive parser.
//! This is not a legacy skipped/defaulted/raw-expansion acceptance metric.
use super::{PreflightBounds, PreflightFailure};
use crate::model::json::deferred::JsonDecodeError;
type Failure = PreflightFailure<JsonDecodeError>;
/// Streaming text fragments need no retained String; every original duplicate
/// key/value is counted before replacement, exactly as the existing seed.
#[derive(Default)]
pub struct LiteralPreflight {
    bounds: PreflightBounds,
    current_text: Option<usize>,
}
impl LiteralPreflight {
    fn add(&mut self, bytes: usize) -> Result<(), Failure> {
        self.bounds.structural_bytes = self
            .bounds
            .structural_bytes
            .checked_add(bytes)
            .ok_or(Failure::ArithmeticOverflow)?;
        if self.bounds.structural_bytes > super::preflight::LIMIT {
            return Err(Failure::StructuralLimit);
        }
        Ok(())
    }
    pub fn node(&mut self) -> Result<(), Failure> {
        if self.current_text.is_some() {
            return Err(Failure::Json(JsonDecodeError::ConstructionOrder));
        }
        self.bounds.nodes = self
            .bounds
            .nodes
            .checked_add(1)
            .ok_or(Failure::ArithmeticOverflow)?;
        self.add(512)
    }
    pub fn start_text(&mut self) -> Result<(), Failure> {
        self.node()?;
        self.current_text = Some(0);
        Ok(())
    }
    pub fn text_fragment(&mut self, fragment: &str) -> Result<(), Failure> {
        let current = self
            .current_text
            .as_mut()
            .ok_or(Failure::Json(JsonDecodeError::ConstructionOrder))?;
        *current = current
            .checked_add(fragment.len())
            .ok_or(Failure::ArithmeticOverflow)?;
        self.bounds.maximum_text_bytes = self.bounds.maximum_text_bytes.max(*current);
        self.bounds.decoded_text_bytes = self
            .bounds
            .decoded_text_bytes
            .checked_add(fragment.len())
            .ok_or(Failure::ArithmeticOverflow)?;
        self.add(
            fragment
                .len()
                .checked_mul(2)
                .ok_or(Failure::ArithmeticOverflow)?,
        )
    }
    pub fn end_text(&mut self) -> Result<(), Failure> {
        if self.current_text.take().is_none() {
            return Err(Failure::Json(JsonDecodeError::ConstructionOrder));
        }
        Ok(())
    }
    pub fn finish(self) -> Result<PreflightBounds, Failure> {
        if self.current_text.is_some() {
            return Err(Failure::Json(JsonDecodeError::ConstructionOrder));
        }
        Ok(self.bounds)
    }
}
