//! Fixed-size evidence borrows ranges from retained literal fields, never copies labels.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    start: usize,
    end: usize,
}
impl TextRange {
    /// Both references must describe the same allocation; otherwise reject.
    pub fn supplied(full: &str, part: &str) -> Option<Self> {
        let start = (part.as_ptr() as usize).checked_sub(full.as_ptr() as usize)?;
        let end = start.checked_add(part.len())?;
        (full.get(start..end) == Some(part)).then_some(Self { start, end })
    }
    pub fn get(self, full: &str) -> Option<&str> {
        full.get(self.start..self.end)
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LiteralIdentity {
    #[default]
    Absent,
    Invalid,
    Registered {
        owner: TextRange,
        tool: TextRange,
    },
    Historical {
        tool: TextRange,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiteralEvidence {
    pub name: LiteralIdentity,
    pub title: LiteralIdentity,
    pub input: LiteralIdentity,
}
impl LiteralEvidence {
    pub fn apply(&mut self, incoming: Self, name: bool, title: bool, input: bool) {
        if name {
            self.name = incoming.name;
        }
        if title {
            self.title = incoming.title;
        }
        if input {
            self.input = incoming.input;
        }
    }
}
