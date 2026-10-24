use proptest::prelude::*;

use crate::sanitize_display;

#[test]
fn invisible_and_directional_characters_are_removed() {
    let hostile = "a\u{202E}b\u{2066}c\u{200B}d\u{7}e\u{2028}f\u{FEFF}g\u{85}h\n";
    assert_eq!(sanitize_display(hostile, 50), "abcdefgh");
}

#[test]
fn combining_marks_stop_at_two_per_base() {
    let stacked = "e\u{301}\u{302}\u{303}\u{304}x";
    assert_eq!(sanitize_display(stacked, 50), "e\u{301}\u{302}x");
}

#[test]
fn text_is_cut_to_the_cap_in_characters() {
    assert_eq!(sanitize_display("héllo wörld", 5), "héllo");
    assert_eq!(sanitize_display("abc", 0), "");
}

fn clean(c: char) -> bool {
    !c.is_control()
        && !matches!(
            c,
            '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{FEFF}'
        )
}

proptest! {
    #[test]
    fn sanitizing_is_bounded_idempotent_and_leaves_no_controls(text in ".{0,200}", cap in 0usize..64) {
        let once = sanitize_display(&text, cap);
        prop_assert!(once.chars().count() <= cap);
        prop_assert_eq!(sanitize_display(&once, cap), once.clone());
        prop_assert!(once.chars().all(clean));
    }
}
