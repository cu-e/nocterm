use super::*;
use nocterm_project_providers::{
    ContainerFacts, ContainerLocation, DistributionFamily, EnvironmentAccount, EnvironmentOs,
    LibcFamily, ProgrammingLanguage, SetupPhase, ToolObservation,
};
use std::collections::BTreeMap;

pub(crate) const PROBE: &str = r#"emit() { printf '%s\000%s\000' "$1" "$2"; }
emit kernel "$(uname -s 2>/dev/null)"
emit architecture "$(uname -m 2>/dev/null)"
emit os_release "$(head -c 65536 /etc/os-release 2>/dev/null)"
emit libc "$(getconf GNU_LIBC_VERSION 2>/dev/null || ldd --version 2>&1 | head -c 2048)"
emit uid "$(id -u)"
emit user "$(id -un)"
emit home "${HOME:-}"
if test -n "${HOME:-}" && test -d "$HOME" && test ! -L "$HOME" && test -w "$HOME"; then emit writable yes; else emit writable no; fi
for program in curl bash tar gzip xz unzip sha256sum awk mktemp cc make find readlink apt-get apk pacman; do
    if command -v "$program" >/dev/null 2>&1; then emit program "$program"; fi
done
if test -s /var/lib/pacman/sync/core.db && test -s /var/lib/pacman/sync/extra.db; then emit program pacman-database; fi
emit python "$(python3 --version 2>/dev/null | head -c 256)"
emit rust "$(rustc --version 2>/dev/null | head -c 256)"
emit bun "$(bun --version 2>/dev/null | head -c 256)"
emit node "$(node --version 2>/dev/null | head -c 256)"
emit java "$(java -version 2>&1 | head -n 1 | head -c 256)"
"#;

pub(super) async fn inspect(
    commands: &dyn EnvironmentCommands,
    source: &EnvironmentSource,
) -> Result<EnvironmentSnapshot, EnvironmentError> {
    let output = commands
        .observation(None)
        .run(
            SetupPhase::Inspecting,
            crate::activation::activate_request(ExecRequest::new("sh").args(["-c", PROBE])),
        )
        .await
        .map_err(|error| match error {
            nocterm_project_providers::ProjectSetupError::Canceled => EnvironmentError::Canceled,
            nocterm_project_providers::ProjectSetupError::TimedOut => EnvironmentError::TimedOut,
            error => EnvironmentError::Probe(Box::new(error)),
        })?;
    let mut snapshot = decode(&output)?;
    if let Ok(location) = ContainerLocation::from_target(source.target()) {
        let mut facts = ContainerFacts {
            engine: location.engine(),
            id: location.container().to_owned(),
            name: None,
            image: None,
        };
        if let Some(exec) = source.host_metadata() {
            let metadata = commands.observation(Some(exec));
            if let Ok(output) = metadata
                .run(
                    SetupPhase::Inspecting,
                    location.engine().command().args([
                        "inspect",
                        "--type",
                        "container",
                        location.container(),
                    ]),
                )
                .await
            {
                decode_container_metadata(&mut facts, &output);
            }
        }
        snapshot.container = Some(facts);
    }
    commands.check()?;
    Ok(snapshot)
}
fn decode_container_metadata(facts: &mut ContainerFacts, output: &str) {
    let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(output) else {
        return;
    };
    let Some(row) = rows
        .first()
        .filter(|row| row["Id"].as_str() == Some(facts.id.as_str()))
    else {
        return;
    };
    facts.name = row["Name"]
        .as_str()
        .map(|name| name.trim_start_matches('/').chars().take(256).collect());
    facts.image = row["Config"]["Image"]
        .as_str()
        .map(|image| image.chars().take(512).collect());
}
fn decode(output: &str) -> Result<EnvironmentSnapshot, EnvironmentError> {
    let mut fields = BTreeMap::new();
    let mut programs = Vec::new();
    let parts: Vec<_> = output.split_terminator('\0').collect();
    if parts.len() % 2 != 0 {
        return Err(EnvironmentError::InvalidInput(
            "incomplete environment observation",
        ));
    }
    for pair in parts.as_chunks::<2>().0 {
        if pair[0] == "program" {
            programs.push(pair[1].to_owned());
        } else {
            fields.insert(pair[0], pair[1]);
        }
    }
    let get = |key| fields.get(key).copied().unwrap_or("");
    let release = get("os_release");
    let id =
        nocterm_connections_model::os::from_os_release(release).map_or("unknown", |id| id.as_str());
    let family = family(id, release);
    let libc = if get("libc").to_ascii_lowercase().contains("musl") {
        LibcFamily::Musl
    } else if get("libc").contains("glibc")
        || get("libc").contains("GLIBC")
        || get("libc").contains("GNU libc")
    {
        LibcFamily::Glibc
    } else {
        LibcFamily::Unknown
    };
    let home = get("home");
    if !home.starts_with('/') || home.len() > 4096 || home.chars().any(char::is_control) {
        return Err(EnvironmentError::InvalidInput("invalid observed user home"));
    }
    let tools = ProgrammingLanguage::ALL
        .into_iter()
        .map(|language| {
            let key = match language {
                ProgrammingLanguage::Python => "python",
                ProgrammingLanguage::Rust => "rust",
                ProgrammingLanguage::Bun => "bun",
                ProgrammingLanguage::Node => "node",
                ProgrammingLanguage::Java => "java",
            };
            let version = get(key).trim();
            ToolObservation {
                language,
                version: (!version.is_empty() && !version.contains("not found"))
                    .then(|| version.chars().take(256).collect()),
            }
        })
        .collect();
    Ok(EnvironmentSnapshot {
        os: EnvironmentOs {
            kernel: get("kernel").to_owned(),
            id: id.to_owned(),
            name: release_field(release, "PRETTY_NAME").unwrap_or_else(|| id.to_owned()),
            release: release_field(release, "VERSION_ID").unwrap_or_default(),
            architecture: get("architecture").to_owned(),
            libc,
            family,
        },
        account: EnvironmentAccount {
            user: get("user").chars().take(256).collect(),
            uid: get("uid")
                .parse()
                .map_err(|_| EnvironmentError::InvalidInput("invalid observed UID"))?,
            home: home.to_owned(),
            writable_home: get("writable") == "yes",
        },
        tools,
        container: None,
        available_programs: programs,
    })
}
fn family(id: &str, release: &str) -> DistributionFamily {
    use DistributionFamily::*;
    let known = |id: &str| match id {
        "ubuntu" | "debian" | "kali" | "linuxmint" | "pop" | "elementary" | "raspbian"
        | "devuan" | "parrot" | "neon" | "zorin" => Debian,
        "alpine" | "postmarketos" => Alpine,
        "arch" | "manjaro" | "endeavouros" | "garuda" | "artix" | "cachyos" => Arch,
        _ => Unsupported,
    };
    let family = known(id);
    if family != Unsupported {
        return family;
    }
    release_field(release, "ID_LIKE")
        .and_then(|value| {
            value
                .split_whitespace()
                .map(known)
                .find(|family| *family != Unsupported)
        })
        .unwrap_or(Unsupported)
}
fn release_field(release: &str, key: &str) -> Option<String> {
    release
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == key)
        .map(|(_, value)| {
            value
                .trim()
                .trim_matches(['\'', '"'])
                .chars()
                .take(256)
                .collect()
        })
}

#[cfg(test)]
mod tests;
