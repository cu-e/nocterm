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
const ALLOWED: &[&str] = &[
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
];
/// Windows system locations that Node.js, npm and npx resolve their install,
/// cache and shell paths from. None of them carries a credential.
const WINDOWS_ALLOWED: &[&str] = &[
    "SYSTEMDRIVE",
    "PROGRAMFILES",
    "PROGRAMW6432",
    "PROGRAMDATA",
    "COMMONPROGRAMFILES",
    "ALLUSERSPROFILE",
    "PUBLIC",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "OS",
    "PROCESSOR_ARCHITECTURE",
    "NUMBER_OF_PROCESSORS",
    "NVM_HOME",
    "NVM_SYMLINK",
    "VOLTA_HOME",
];
/// How the native platform compares environment names. Windows names are
/// case-insensitive: the system spells the search path `Path`, not `PATH`.
#[derive(Clone, Copy)]
pub(crate) struct NameRules {
    windows: bool,
}
impl NameRules {
    pub(crate) const fn native() -> Self {
        Self {
            windows: cfg!(windows),
        }
    }
    #[cfg(test)]
    pub(crate) const fn windows() -> Self {
        Self { windows: true }
    }
    fn same(self, left: &str, right: &str) -> bool {
        if self.windows {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }
    /// One spelling per variable, so an override replaces rather than duplicates it.
    fn key(self, name: &str) -> String {
        if self.windows {
            name.to_ascii_uppercase()
        } else {
            name.to_owned()
        }
    }
    fn allowed(self, name: &str) -> bool {
        let listed = |names: &[&str]| names.iter().any(|known| self.same(known, name));
        listed(ALLOWED)
            || (self.windows && listed(WINDOWS_ALLOWED))
            || name.starts_with("LC_")
            || name.starts_with("XDG_")
    }
}
pub fn sanitized_environment(
    launch: &AgentLaunch,
    parent: impl IntoIterator<Item = (String, String)>,
) -> BTreeMap<String, String> {
    sanitized_with(launch, parent, NameRules::native())
}
pub(crate) fn sanitized_with(
    launch: &AgentLaunch,
    parent: impl IntoIterator<Item = (String, String)>,
    rules: NameRules,
) -> BTreeMap<String, String> {
    let inherited = |name: &str| {
        launch
            .inherit_env
            .iter()
            .any(|known| rules.same(known, name))
    };
    let mut result: BTreeMap<_, _> = parent
        .into_iter()
        .filter(|(k, v)| {
            valid_name(k) && !blocked(k) && !v.contains('\0') && (rules.allowed(k) || inherited(k))
        })
        .map(|(k, v)| (rules.key(&k), v))
        .collect();
    result.extend(
        launch
            .env
            .iter()
            .filter(|(k, v)| {
                valid_name(k) && !blocked(k) && !credential_override(k) && !v.contains('\0')
            })
            .map(|(k, v)| (rules.key(k), v.clone())),
    );
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn launch() -> AgentLaunch {
        AgentLaunch {
            id: "codex".into(),
            name: "Codex".into(),
            command: "npx".into(),
            args: Vec::new(),
            env: Default::default(),
            inherit_env: vec!["Custom_Inherited".into()],
            hub: None,
        }
    }
    fn windows_parent() -> Vec<(String, String)> {
        [
            ("Path", r"C:\Program Files\nodejs;C:\Windows\system32"),
            ("PATHEXT", ".COM;.EXE;.BAT;.CMD"),
            ("SystemRoot", r"C:\Windows"),
            ("ComSpec", r"C:\Windows\system32\cmd.exe"),
            ("ProgramFiles", r"C:\Program Files"),
            ("APPDATA", r"C:\Users\user\AppData\Roaming"),
            ("CUSTOM_INHERITED", "kept"),
            ("Unrequested_Secret", "dropped"),
            ("ssh_auth_sock", "dropped"),
        ]
        .map(|(key, value)| (key.into(), value.into()))
        .into()
    }
    #[test]
    fn windows_names_match_without_case_and_get_one_spelling() {
        let env = sanitized_with(&launch(), windows_parent(), NameRules::windows());
        assert_eq!(env["PATH"], r"C:\Program Files\nodejs;C:\Windows\system32");
        assert_eq!(env["SYSTEMROOT"], r"C:\Windows");
        assert_eq!(env["COMSPEC"], r"C:\Windows\system32\cmd.exe");
        assert_eq!(env["PROGRAMFILES"], r"C:\Program Files");
        assert_eq!(env["CUSTOM_INHERITED"], "kept");
        assert!(!env.contains_key("Path"));
        assert!(!env.values().any(|value| value == "dropped"));
    }
    #[test]
    fn a_windows_override_replaces_the_inherited_path_instead_of_duplicating_it() {
        let mut launch = launch();
        launch.env.insert("path".into(), r"D:\node".into());
        let env = sanitized_with(&launch, windows_parent(), NameRules::windows());
        assert_eq!(env["PATH"], r"D:\node");
        assert_eq!(
            env.keys()
                .filter(|key| key.eq_ignore_ascii_case("path"))
                .count(),
            1
        );
    }
    #[test]
    fn unix_names_stay_case_sensitive() {
        let parent = [
            ("Path".to_owned(), "/wrong".to_owned()),
            ("PATH".into(), "/bin".into()),
        ];
        let env = sanitized_with(&launch(), parent, NameRules { windows: false });
        assert_eq!(env["PATH"], "/bin");
        assert!(!env.contains_key("Path"));
        assert!(!env.contains_key("PROGRAMFILES"));
    }
}
