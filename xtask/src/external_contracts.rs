//! The hub contract lives in a public repository outside this workspace (ADR 0096).
//!
//! Its packages carry `nocterm-` names but no architecture layer, so the generic dependency
//! check would reject them. This module is the one exception, and it is closed: a listed
//! package may be used only by its listed consumer, only from the pinned git repository, and
//! never through a path or a patch. Any other `nocterm-*` package without a layer remains an
//! error, and so does a contract package used by any other crate.

use std::path::Path;

use anyhow::{Context as _, bail};

use crate::{Dependency, Package};

/// The only repository a contract package may come from.
pub(crate) const REPOSITORY: &str = "https://github.com/cu-e/nocterm-contract";

/// The adapter that speaks the hub contract; the only crate that may use it or the protocol SDK.
pub(crate) const HUB_ADAPTER: &str = "nocterm-hub-io";

/// A package published by the contract repository and the one crate allowed to use it.
pub(crate) struct ExternalContract {
    pub(crate) package: &'static str,
    pub(crate) consumer: &'static str,
}

pub(crate) const EXTERNAL_CONTRACTS: &[ExternalContract] = &[
    ExternalContract {
        package: "nocterm-cloud-device",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-hub-client",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-hub-grpc",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-hub-model",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-hub-proto",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-keys",
        consumer: HUB_ADAPTER,
    },
    ExternalContract {
        package: "nocterm-link",
        consumer: HUB_ADAPTER,
    },
];

/// Protocol SDK crates (`tonic`, `tonic-*`, `prost`, `prost-*`) are used by the hub adapter only.
pub(crate) fn is_protocol_sdk(name: &str) -> bool {
    ["tonic", "prost"].iter().any(|sdk| {
        name == *sdk
            || name
                .strip_prefix(sdk)
                .is_some_and(|rest| rest.starts_with('-'))
    })
}

pub(crate) fn is_contract(name: &str) -> bool {
    EXTERNAL_CONTRACTS
        .iter()
        .any(|contract| contract.package == name)
}

/// The prefix of the `source` of a dependency or lock entry on the pinned repository.
fn git_prefix() -> String {
    format!("git+{REPOSITORY}?rev=")
}

/// `rev=<40 hex>` followed by nothing or a `#<resolved commit>` suffix.
fn pinned_commit(source: &str) -> bool {
    let Some(rest) = source.strip_prefix(&git_prefix()) else {
        return false;
    };
    let rev = rest.split_once('#').map_or(rest, |(rev, _)| rev);
    rev.len() == 40 && rev.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A dependency on a contract package: right consumer, pinned git source, no path.
pub(crate) fn check_dependency(package: &str, dependency: &Dependency) -> anyhow::Result<()> {
    let name = dependency.name.as_str();
    let Some(contract) = EXTERNAL_CONTRACTS
        .iter()
        .find(|contract| contract.package == name)
    else {
        return Ok(());
    };
    if package != contract.consumer {
        bail!(
            "{package} depends on hub contract package {name}; only {} may",
            contract.consumer
        );
    }
    if dependency.path.is_some() {
        bail!("hub contract package {name} must come from {REPOSITORY}, not from a path");
    }
    match dependency.source.as_deref() {
        Some(source) if pinned_commit(source) => Ok(()),
        Some(source) => bail!(
            "hub contract package {name} must come from {REPOSITORY} at a full commit `rev`, found {source}"
        ),
        None => bail!("hub contract package {name} must come from {REPOSITORY} at a pinned `rev`"),
    }
}

/// A workspace member must not take a contract package's name.
pub(crate) fn check_members(packages: &[Package]) -> anyhow::Result<()> {
    for package in packages {
        if is_contract(&package.name) {
            bail!(
                "workspace member {} reuses the name of a hub contract package",
                package.name
            );
        }
    }
    Ok(())
}

/// Every contract package the build resolves comes from the pinned repository. A `[patch]` or
/// path override shows up in `Cargo.lock` as a package without a git source.
pub(crate) fn check_lock(root: &Path) -> anyhow::Result<()> {
    let path = root.join("Cargo.lock");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    check_lock_text(&text)
}

fn check_lock_text(text: &str) -> anyhow::Result<()> {
    let lock: toml::Table = text.parse().context("parse Cargo.lock")?;
    let packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten();
    for package in packages {
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        if !is_contract(name) {
            continue;
        }
        let source = package.get("source").and_then(toml::Value::as_str);
        if !source.is_some_and(pinned_commit) {
            bail!(
                "Cargo.lock resolves hub contract package {name} from {}; it must come from {REPOSITORY} at a pinned commit",
                source.unwrap_or("a path")
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "b7658b46601629edd44c51d6105605e29df78fd9";

    fn dependency(name: &str, source: Option<String>, path: Option<&str>) -> Dependency {
        Dependency {
            name: name.into(),
            kind: None,
            rename: None,
            features: Vec::new(),
            uses_default_features: true,
            source,
            path: path.map(Into::into),
        }
    }

    fn pinned() -> Option<String> {
        Some(format!("{}{COMMIT}", git_prefix()))
    }

    #[test]
    fn a_contract_package_is_allowed_to_its_consumer_from_the_pinned_repository() {
        for contract in EXTERNAL_CONTRACTS {
            let dependency = dependency(contract.package, pinned(), None);
            assert!(check_dependency(contract.consumer, &dependency).is_ok());
        }
        let resolved = dependency(
            "nocterm-keys",
            Some(format!("{}{COMMIT}#{COMMIT}", git_prefix())),
            None,
        );
        assert!(check_dependency("nocterm-hub-io", &resolved).is_ok());
    }

    #[test]
    fn any_other_crate_is_refused_a_contract_package() {
        for contract in EXTERNAL_CONTRACTS {
            for package in [
                "nocterm",
                "nocterm-hubs",
                "nocterm-hubs-model",
                "nocterm-ssh",
            ] {
                let dependency = dependency(contract.package, pinned(), None);
                assert!(check_dependency(package, &dependency).is_err());
            }
        }
    }

    #[test]
    fn a_contract_package_from_a_path_or_another_source_is_refused() {
        let from_path = dependency("nocterm-keys", None, Some("/tmp/contract/keys"));
        assert!(check_dependency("nocterm-hub-io", &from_path).is_err());
        let path_with_source = dependency("nocterm-keys", pinned(), Some("../keys"));
        assert!(check_dependency("nocterm-hub-io", &path_with_source).is_err());
        let unspecified = dependency("nocterm-keys", None, None);
        assert!(check_dependency("nocterm-hub-io", &unspecified).is_err());
        for source in [
            "registry+https://github.com/rust-lang/crates.io-index".to_owned(),
            format!("git+https://github.com/someone/nocterm-contract?rev={COMMIT}"),
            format!("git+ssh://git@github.com/cu-e/nocterm-contract?rev={COMMIT}"),
            format!("git+{REPOSITORY}?branch=main"),
            format!("git+{REPOSITORY}?tag=v1"),
            format!("git+{REPOSITORY}"),
            format!("git+{REPOSITORY}?rev=b7658b4"),
            format!("git+{REPOSITORY}?rev={}", COMMIT.to_uppercase()),
        ] {
            let dependency = dependency("nocterm-hub-client", Some(source.clone()), None);
            assert!(
                check_dependency("nocterm-hub-io", &dependency).is_err(),
                "{source}"
            );
        }
    }

    #[test]
    fn other_packages_are_not_judged_by_the_contract_rule() {
        let unknown = dependency("nocterm-forgotten", None, None);
        assert!(check_dependency("nocterm-hub-io", &unknown).is_ok());
        assert!(!is_contract("nocterm-forgotten"));
        assert!(is_contract("nocterm-hub-client"));
    }

    #[test]
    fn protocol_sdk_crates_are_recognised_by_name() {
        for name in [
            "tonic",
            "tonic-prost",
            "tonic-build",
            "prost",
            "prost-types",
        ] {
            assert!(is_protocol_sdk(name), "{name}");
        }
        for name in ["tonicity", "prosty", "protobuf", "tokio"] {
            assert!(!is_protocol_sdk(name), "{name}");
        }
    }

    #[test]
    fn the_lock_file_must_resolve_contract_packages_from_the_pinned_commit() {
        let good = format!(
            "[[package]]\nname = \"nocterm-keys\"\nversion = \"0.1.0\"\nsource = \"{}{COMMIT}#{COMMIT}\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n",
            git_prefix()
        );
        assert!(check_lock_text(&good).is_ok());
        let patched = "[[package]]\nname = \"nocterm-keys\"\nversion = \"0.1.0\"\n";
        assert!(check_lock_text(patched).is_err());
        let forked = format!(
            "[[package]]\nname = \"nocterm-link\"\nversion = \"0.1.0\"\nsource = \"git+https://github.com/other/nocterm-contract?rev={COMMIT}#{COMMIT}\"\n"
        );
        assert!(check_lock_text(&forked).is_err());
        assert!(check_lock_text("[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\n").is_ok());
        assert!(check_lock_text("").is_ok());
    }

    #[test]
    fn a_workspace_member_cannot_take_a_contract_name() {
        let member = |name: &str| Package {
            id: name.into(),
            name: name.into(),
            description: None,
            manifest_path: "Cargo.toml".into(),
            metadata: serde_json::Value::Null,
            dependencies: Vec::new(),
            targets: Vec::new(),
            features: Default::default(),
        };
        assert!(check_members(&[member("nocterm-hub-io")]).is_ok());
        assert!(check_members(&[member("nocterm-hub-io"), member("nocterm-keys")]).is_err());
    }
}
