use crate::AgentLaunch;
use std::collections::BTreeMap;
pub fn valid_name(name: &str) -> bool {
    let mut b = name.bytes();
    b.next()
        .is_some_and(|v| v.is_ascii_alphabetic() || v == b'_')
        && b.all(|v| v.is_ascii_alphanumeric() || v == b'_')
}
pub fn blocked(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let name = upper.as_str();
    name.starts_with("NOCTERM_")
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
        .contains(&name)
}
/// Recognized authentication/credential names must use explicit inheritance,
/// never plaintext settings overrides. Arbitrarily named secrets remain unknowable.
pub fn credential_override(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.ends_with("_KEY")
        || name == "TOKEN"
        || name.ends_with("_TOKEN")
        || name.contains("PASSWORD")
        || name.contains("PASSWD")
        || name.contains("SECRET")
        || name == "AUTHORIZATION"
}
fn allowed(name: &str) -> bool {
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
    let mut result: BTreeMap<_, _> = parent
        .into_iter()
        .filter(|(k, v)| {
            valid_name(k)
                && !blocked(k)
                && !v.contains('\0')
                && (allowed(k) || launch.inherit_env.contains(k))
        })
        .collect();
    result.extend(
        launch
            .env
            .iter()
            .filter(|(k, v)| {
                valid_name(k) && !blocked(k) && !credential_override(k) && !v.contains('\0')
            })
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    result
}
