//! Iterative literal metric with stock scalar and Unicode leaves.
//! Original-source mode preserves its physical parser budget separately from
//! semantic native depth and per-RawValue parser invocations.
use super::{JsonStringReader, LiteralReadError, input::Input, scalar};
use nocterm_ai::{
    history::{LiteralPreflight, MAX_FILE_BYTES, PreflightBounds},
    model::json::deferred::{
        DecodeLedger,
        literal::{LiteralFrontier, LiteralSyntax, StringRole},
    },
};
use std::io::Read;
#[cfg(test)]
mod tests;
pub(in crate::loading) fn preflight<R: Read>(
    source: R,
    ledger: &DecodeLedger,
) -> Result<PreflightBounds, LiteralReadError> {
    walk(source, ledger, |_, _, _, _| Ok(()))
}
pub(in crate::loading) fn walk<R: Read>(
    source: R,
    ledger: &DecodeLedger,
    mut text: impl FnMut(usize, usize, usize, StringRole) -> Result<(), LiteralReadError>,
) -> Result<PreflightBounds, LiteralReadError> {
    let mut input = Input::new_managed_marker(source);
    let result = current(&mut input, ledger, &mut text, None, true, false);
    input.check()?;
    result
}
/// Additional checked-plan phase; ordinary preflight keeps its old allocation path.
pub(in crate::loading) fn planned_walk<R: Read>(
    source: R,
    ledger: &DecodeLedger,
    mut text: impl FnMut(usize, usize, usize, StringRole) -> Result<(), LiteralReadError>,
) -> Result<(PreflightBounds, usize), LiteralReadError> {
    let mut input = Input::new_managed_marker(source);
    let mut frontier = LiteralFrontier::new(ledger)?;
    let result = current(
        &mut input,
        ledger,
        &mut text,
        Some(&mut frontier),
        true,
        false,
    );
    input.check()?;
    Ok((result?, frontier.finish()?))
}
struct WalkState<'a> {
    syntax: LiteralSyntax,
    bounds: Option<LiteralPreflight>,
    frontier: Option<&'a mut LiteralFrontier>,
    original_depth: Option<usize>,
}
impl<'a> WalkState<'a> {
    fn new(
        ledger: &DecodeLedger,
        frontier: Option<&'a mut LiteralFrontier>,
        measure: bool,
        original: bool,
    ) -> Result<Self, LiteralReadError> {
        Ok(Self {
            syntax: LiteralSyntax::new(ledger)?,
            bounds: measure.then(LiteralPreflight::default),
            frontier,
            original_depth: original.then_some(0),
        })
    }
    fn open(&mut self, object: bool) -> Result<(), LiteralReadError> {
        if let Some(depth) = &mut self.original_depth {
            if *depth == 127 {
                return Err(LiteralReadError::SourceDepth);
            }
            *depth += 1;
        }
        if let Some(bounds) = &mut self.bounds {
            bounds.node()?;
        }
        if object {
            self.syntax.start_object()?
        } else {
            self.syntax.start_array()?
        }
        if let Some(frontier) = self.frontier.as_deref_mut() {
            frontier.start()?;
        }
        Ok(())
    }
    fn close(&mut self, object: bool) -> Result<(), LiteralReadError> {
        if object {
            self.syntax.end_object()?
        } else {
            self.syntax.end_array()?
        }
        if let Some(depth) = &mut self.original_depth {
            *depth = depth.checked_sub(1).ok_or(LiteralReadError::Token)?;
        }
        if let Some(frontier) = self.frontier.as_deref_mut() {
            frontier.end()?;
        }
        Ok(())
    }
    fn scalar(&mut self) -> Result<(), LiteralReadError> {
        self.syntax.scalar()?;
        if let Some(frontier) = self.frontier.as_deref_mut() {
            frontier.scalar()?;
        }
        if let Some(bounds) = &mut self.bounds {
            bounds.node()?;
        }
        Ok(())
    }
    fn string(&mut self) -> Result<StringRole, LiteralReadError> {
        let role = self.syntax.string()?;
        if role == StringRole::Value
            && let Some(frontier) = self.frontier.as_deref_mut()
        {
            frontier.scalar()?;
        }
        if let Some(bounds) = &mut self.bounds {
            bounds.start_text()?;
        }
        Ok(role)
    }
    fn fragment(&mut self, text: &str) -> Result<(), LiteralReadError> {
        if let Some(bounds) = &mut self.bounds {
            bounds.text_fragment(text)?;
        }
        Ok(())
    }
    fn end_text(&mut self) -> Result<(), LiteralReadError> {
        if let Some(bounds) = &mut self.bounds {
            bounds.end_text()?;
        }
        Ok(())
    }
    fn finish(self) -> Result<PreflightBounds, LiteralReadError> {
        self.syntax.finish()?;
        Ok(match self.bounds {
            Some(bounds) => bounds.finish()?,
            None => PreflightBounds::default(),
        })
    }
}
fn current<R: Read>(
    input: &mut Input<R>,
    ledger: &DecodeLedger,
    text: &mut impl FnMut(usize, usize, usize, StringRole) -> Result<(), LiteralReadError>,
    frontier: Option<&mut LiteralFrontier>,
    measure: bool,
    original: bool,
) -> Result<PreflightBounds, LiteralReadError> {
    let mut state = WalkState::new(ledger, frontier, measure, original)?;
    while let Some(byte) = input.nonwhite()? {
        match byte {
            b'[' => state.open(false)?,
            b'{' => state.open(true)?,
            b']' => state.close(false)?,
            b'}' => state.close(true)?,
            b',' => state.syntax.comma()?,
            b':' => state.syntax.colon()?,
            b'"' => read_text(input, &mut state, text, ledger)?,
            b'n' | b't' | b'f' | b'-' | b'0'..=b'9' => {
                state.scalar()?;
                input.unread(byte)?;
                scalar::read(input, ledger)?;
            }
            _ => return Err(LiteralReadError::Token),
        }
    }
    state.finish()
}
fn read_text<R: Read>(
    input: &mut Input<R>,
    state: &mut WalkState<'_>,
    text: &mut impl FnMut(usize, usize, usize, StringRole) -> Result<(), LiteralReadError>,
    ledger: &DecodeLedger,
) -> Result<(), LiteralReadError> {
    let role = state.string()?;
    let start = input.offset() - 1;
    let mut decoded_bytes = 0usize;
    input.unread(b'"')?;
    let mut string = JsonStringReader::new_managed(&mut *input, MAX_FILE_BYTES as usize, ledger)?;
    while let Some(fragment) = string.next_fragment()? {
        state.fragment(fragment)?;
        decoded_bytes = decoded_bytes
            .checked_add(fragment.len())
            .ok_or(nocterm_ai::model::json::deferred::JsonDecodeError::Arithmetic)?;
    }
    state.end_text()?;
    text(start, input.offset(), decoded_bytes, role)
}

/// Original source-only K and original physical recursion guard. This does not
/// perform legacy Value expansion or replace field default/skip semantics.
pub(in crate::loading) fn original_source<R: Read>(
    source: R,
    ledger: &DecodeLedger,
) -> Result<PreflightBounds, LiteralReadError> {
    let mut input = Input::new_managed_marker(source);
    let result = current(
        &mut input,
        ledger,
        &mut |_, _, _, _| Ok(()),
        None,
        true,
        true,
    );
    input.check()?;
    result
}
