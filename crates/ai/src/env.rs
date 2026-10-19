use crate::AgentLaunch;
use std::collections::BTreeMap;
pub fn valid_name(name: &str) -> bool {
    let mut b = name.bytes();
    b.next()
        .is_some_and(|v| v.is_ascii_alphabetic() || v == b'_')
        && b.all(|v| v.is_ascii_alphanumeric() || v == b'_')
}
pub fn blocked(name: &str) -> bool {
    ascii_starts_with(name, "NOCTERM_")
        || [
            "SSH_AUTH_SOCK",
            "SSH_AGENT_PID",
            "GPG_AGENT_INFO",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "DYLD_LIBRARY_PATH",
            "BASH_ENV",
            "ENV",
            "ZDOTDIR",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "PYTHONSTARTUP",
        ]
        .iter()
        .any(|blocked| name.eq_ignore_ascii_case(blocked))
}
/// Recognized authentication/credential names must use explicit inheritance,
/// never plaintext settings overrides. Arbitrarily named secrets remain unknowable.
pub fn credential_override(name: &str) -> bool {
    ascii_ends_with(name, "_KEY")
        || name.eq_ignore_ascii_case("TOKEN")
        || ascii_ends_with(name, "_TOKEN")
        || ascii_contains(name, "PASSWORD")
        || ascii_contains(name, "PASSWD")
        || ascii_contains(name, "SECRET")
        || name.eq_ignore_ascii_case("AUTHORIZATION")
}
fn ascii_starts_with(value: &str, prefix: &str) -> bool {
    value
        .as_bytes()
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
}
fn ascii_ends_with(value: &str, suffix: &str) -> bool {
    value
        .len()
        .checked_sub(suffix.len())
        .and_then(|start| value.as_bytes().get(start..))
        .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix.as_bytes()))
}
fn ascii_contains(value: &str, needle: &str) -> bool {
    value
        .as_bytes()
        .windows(needle.len())
        .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
}
pub fn allowed(name: &str) -> bool {
    [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "LANG",
        "TMPDIR",
        "TMP",
        "TEMP",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "DBUS_SESSION_BUS_ADDRESS",
        "SYSTEMROOT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "APPDATA",
        "LOCALAPPDATA",
        "USERPROFILE",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ]
    .contains(&name)
        || name.starts_with("LC_")
        || name.starts_with("XDG_")
}
pub fn sanitized_environment(
    launch: &AgentLaunch,
    parent: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    sanitized_environment_fields(
        &launch.inherit_env,
        launch
            .env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
        parent,
    )
}
pub fn sanitized_environment_fields<'a>(
    inherit_env: &[String],
    explicit: impl IntoIterator<Item = (&'a str, &'a str)>,
    parent: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    let mut result: BTreeMap<_, _> = parent
        .into_iter()
        .filter(|(k, v)| {
            valid_name(k)
                && !blocked(k)
                && !v.contains('\0')
                && (allowed(k) || inherit_env.contains(k))
        })
        .collect();
    result.extend(
        explicit
            .into_iter()
            .filter(|(k, v)| {
                valid_name(k) && !blocked(k) && !credential_override(k) && !v.contains('\0')
            })
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    result
}
