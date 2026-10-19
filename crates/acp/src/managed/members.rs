//! Original map ordinal and winning source spans without discarded value trees.
use super::{budget::SourceBudget, error::DecodeError, scan::Scanner};
use agent_client_protocol::managed::{Buffer, Index};
use nocterm_ai::model::json::deferred::JsonDecodeError;
use std::{
    hash::{Hash, Hasher},
    ops::Range,
};

type Error = DecodeError<JsonDecodeError>;
pub(super) struct Key<'a> {
    input: &'a str,
    range: Range<usize>,
    budget: &'a SourceBudget,
    hash: u64,
}
impl<'a> Key<'a> {
    pub(super) fn new(
        input: &'a str,
        range: Range<usize>,
        budget: &'a SourceBudget,
    ) -> Result<Self, Error> {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        Scanner::new(input, budget.clone()).decoded_string(range.clone(), |part| {
            for byte in part.bytes() {
                hash.write_u8(byte);
            }
            Ok(())
        })?;
        Ok(Self {
            input,
            range,
            budget,
            hash: hash.finish(),
        })
    }
}
impl Hash for Key<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}
impl PartialEq for Key<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.hash != other.hash {
            return false;
        }
        let left = Scanner::new(self.input, self.budget.clone());
        let right = Scanner::new(other.input, other.budget.clone());
        let (mut a, mut b) = (self.range.start + 1, other.range.start + 1);
        while a < self.range.end - 1 && b < other.range.end - 1 {
            // Keys are private lexical proofs: construction follows string validation.
            match (left.decoded_char(a), right.decoded_char(b)) {
                (Ok((x, next_a)), Ok((y, next_b))) if x == y => {
                    a = next_a;
                    b = next_b;
                }
                _ => return false,
            }
        }
        a == self.range.end - 1 && b == other.range.end - 1
    }
}
impl Eq for Key<'_> {}

pub(super) struct Member {
    pub(super) key: Range<usize>,
    pub(super) value: Range<usize>,
}
pub(super) struct Members<'a> {
    ordered: Buffer<Member, SourceBudget>,
    index: Index<Key<'a>, usize, SourceBudget>,
}
impl<'a> Members<'a> {
    pub(super) fn new(
        input: &'a str,
        range: Range<usize>,
        budget: &'a SourceBudget,
        excluded: Option<&str>,
    ) -> Result<Self, Error> {
        let mut result = Self {
            ordered: Buffer::new(budget.clone()),
            index: Index::new(budget.clone()),
        };
        let mut scanner = Scanner::new(input, budget.clone());
        let mut position = scanner.whitespace(range.start + 1);
        while input.as_bytes().get(position) != Some(&b'}') {
            let end = scanner.string(position)?;
            let key_range = position..end;
            let colon = scanner.whitespace(end);
            if input.as_bytes().get(colon) != Some(&b':') {
                return Err(DecodeError::Syntax(colon));
            }
            let start = scanner.whitespace(colon + 1);
            let end = scanner.value(start)?;
            if !excluded
                .map(|name| scanner.key_equals(key_range.clone(), name))
                .transpose()?
                .unwrap_or(false)
            {
                let key = Key::new(input, key_range.clone(), budget)?;
                if let Some(&ordinal) = result.index.get(&key) {
                    result.ordered.as_mut_slice()[ordinal].value = start..end;
                } else {
                    let ordinal = result.ordered.len();
                    result
                        .index
                        .insert(key, ordinal)
                        .map_err(|(error, _, _)| DecodeError::Allocation(error))?;
                    result
                        .ordered
                        .push(Member {
                            key: key_range,
                            value: start..end,
                        })
                        .map_err(|(error, _)| DecodeError::Allocation(error))?;
                }
            }
            position = scanner.whitespace(end);
            if input.as_bytes().get(position) == Some(&b',') {
                position = scanner.whitespace(position + 1);
            }
        }
        Ok(result)
    }
    pub(super) fn get(&self, index: usize) -> Option<&Member> {
        self.ordered.as_slice().get(index)
    }
}
