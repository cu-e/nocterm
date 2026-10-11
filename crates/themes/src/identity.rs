//! Portable registry identities, independent of filesystem operations.
/// Registry identities are a single bounded lowercase path component.
pub fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && (id.as_bytes()[0].is_ascii_lowercase() || id.as_bytes()[0].is_ascii_digit())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
