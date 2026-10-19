use nocterm_session::fs::path;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionPolicy {
    #[default]
    Skip,
    Rename,
    Replace,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionDecision {
    Skip,
    Use,
    Rename,
    Wait,
}
pub fn collision_decision(
    policy: CollisionPolicy,
    exists: bool,
    reserved: bool,
) -> CollisionDecision {
    match (policy, exists, reserved) {
        (CollisionPolicy::Skip, true, _) | (CollisionPolicy::Skip, _, true) => {
            CollisionDecision::Skip
        }
        (CollisionPolicy::Rename, true, _) | (CollisionPolicy::Rename, _, true) => {
            CollisionDecision::Rename
        }
        (CollisionPolicy::Replace, _, true) => CollisionDecision::Wait,
        _ => CollisionDecision::Use,
    }
}
pub fn renamed(original: &str, number: u32) -> String {
    let name = path::file_name(original);
    let (stem, extension) = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or((name, String::new()), |(stem, extension)| {
            (stem, format!(".{extension}"))
        });
    path::join(
        path::parent(original),
        &format!("{stem} ({number}){extension}"),
    )
}
pub fn valid_remote_source(source: &str) -> bool {
    source.starts_with('/')
        && !source.contains('\0')
        && !source.split('/').any(|part| part == "." || part == "..")
        && valid_download_name(path::file_name(source))
}
pub fn valid_download_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 1024
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\0', ':'])
        || name.ends_with(['.', ' '])
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return false;
    }
    !(stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}
