//! Sanitizing of single-line text.

use super::marks::is_stacking_mark;
use alloc::string::String;

/// Nonspacing and enclosing marks kept after one base character.
const MAX_MARKS: usize = 2;

/// Turns `raw` into one inert line.
///
/// Removes C0 and C1 control characters (newline and tab included), every character the Unicode
/// Standard marks `Default_Ignorable_Code_Point` (zero-width characters and joiners, soft hyphen,
/// variation selectors, tag characters, Hangul fillers, bidirectional and invisible format
/// controls), the interlinear annotation characters, U+2028 and U+2029. It keeps at most two
/// nonspacing or enclosing marks (categories Mn and Me) per base character. Whitespace that is not
/// a control character is kept as typed.
///
/// Stripping the joiners U+200C and U+200D splits emoji sequences and changes Persian and Indic
/// spelling. That is deliberate: an invisible character cannot be told from a hidden payload.
pub fn sanitize_text(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut marks = 0;
    for character in raw.chars().filter(|character| !is_invisible(*character)) {
        if is_stacking_mark(character) {
            marks += 1;
            if marks > MAX_MARKS {
                continue;
            }
        } else {
            marks = 0;
        }
        text.push(character);
    }
    text
}

/// A control character, a line or paragraph separator, an interlinear annotation character or a
/// `Default_Ignorable_Code_Point` of Unicode 16.0.
fn is_invisible(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00AD}' // soft hyphen
                | '\u{034F}' // combining grapheme joiner
                | '\u{061C}' // Arabic letter mark
                | '\u{115F}'..='\u{1160}' // Hangul choseong and jungseong fillers
                | '\u{17B4}'..='\u{17B5}' // Khmer inherent vowels
                | '\u{180B}'..='\u{180F}' // Mongolian free variation selectors and separator
                | '\u{200B}'..='\u{200F}' // zero-width space, joiners, direction marks
                | '\u{2028}' | '\u{2029}' // line and paragraph separators
                | '\u{202A}'..='\u{202E}' // bidi embeddings and overrides
                | '\u{2060}'..='\u{206F}' // word joiner, invisible operators, bidi isolates
                | '\u{3164}' // Hangul filler
                | '\u{FE00}'..='\u{FE0F}' // variation selectors
                | '\u{FEFF}' // zero-width no-break space
                | '\u{FFA0}' // halfwidth Hangul filler
                | '\u{FFF0}'..='\u{FFFB}' // reserved specials and interlinear annotation
                | '\u{1BCA0}'..='\u{1BCA3}' // shorthand format controls
                | '\u{1D173}'..='\u{1D17A}' // musical symbol format controls
                | '\u{E0000}'..='\u{E0FFF}' // tag characters and variation selectors supplement
        )
}
