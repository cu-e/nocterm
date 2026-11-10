//! Sanitizing of every string a guest supplies.

use alloc::string::String;
use nocterm_plugin_model::sanitize_text;

/// `text` as one inert line of at most `cap` characters.
///
/// Strips C0 and C1 controls, bidirectional controls (U+202A to U+202E, U+2066 to U+2069),
/// zero-width characters, U+2028, U+2029 and U+FEFF, and keeps at most two combining marks per
/// base character. It applies to every guest string: view text, labels, help and option labels.
/// There is no normalization, because it would add a dependency and names are ASCII.
pub fn sanitize_display(text: &str, cap: usize) -> String {
    sanitize_text(text).chars().take(cap).collect()
}

/// `text` as inert lines of at most `cap` characters in all: each line as
/// [`sanitize_display`] keeps it, tabs as four spaces, and `\n` between the lines.
pub fn sanitize_code(text: &str, cap: usize) -> String {
    let mut out = String::with_capacity(text.len().min(cap));
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&sanitize_text(&line.replace('\t', "    ")));
    }
    out.chars().take(cap).collect()
}
