//! Terminal failures retain their established user-facing recovery instruction.
use nocterm_ai::ToolFailure;
pub(super) fn unreachable(reason: &str) -> ToolFailure {
    format!("{reason} Tell the user right away, in one short sentence, which terminal or server you could not reach and why. Do not try to connect another way: nocterm holds the credentials.").into()
}
