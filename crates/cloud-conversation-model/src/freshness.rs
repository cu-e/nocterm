//! How far the shown history of a chat is confirmed by its runner.

/// A transcript is always a verified prefix of the chat; freshness says whether it is known to
/// be all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// The runner answered a fresh head probe and the replay reached that head. Later records
    /// keep this state.
    Current { witnessed_at_ms: i64 },
    /// The runner did not answer (a sleeping seat, a runner that is off): the stored history may
    /// be followed by newer records. Waking the seat or refreshing may confirm it.
    Unconfirmed { last_witnessed_at_ms: Option<i64> },
    /// The runner witnessed a head the hub's replay did not reach: the hub withholds history.
    Withheld { witnessed_at_ms: i64 },
}
impl Freshness {
    /// What a chat is before its first round.
    pub const OPENING: Self = Self::Unconfirmed {
        last_witnessed_at_ms: None,
    };

    pub fn is_current(self) -> bool {
        matches!(self, Self::Current { .. })
    }
}
impl Default for Freshness {
    fn default() -> Self {
        Self::OPENING
    }
}
