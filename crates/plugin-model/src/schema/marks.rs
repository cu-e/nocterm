//! Unicode 16 stacking categories; spacing marks advance the line.
use unicode_general_category::{GeneralCategory, get_general_category};

pub(super) fn is_stacking_mark(character: char) -> bool {
    matches!(
        get_general_category(character),
        GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark
    )
}
