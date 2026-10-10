//! Actual offline Cargo membership and semantic manifest lint inheritance.
use super::*;
use std::{
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

const INHERIT: &str = "[lints]\nworkspace = true\n";

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nocterm-manifest-lints-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let fixture = Self { root };
        fixture.package(".", "fixture-root", INHERIT, true);
        fixture.package("crates/child", "child", INHERIT, false);
        fixture.package("xtask", "fixture-tool", INHERIT, true);
        fixture.package("vendor/upstream", "upstream", "", false);
        let manifest = fixture.root.join("Cargo.toml");
        let mut source = fs::read_to_string(&manifest).unwrap();
        source.push_str("[workspace]\nmembers=['crates/*','xtask']\nexclude=['vendor/upstream']\n[workspace.lints.rust]\nunsafe_code='forbid'\n");
        fs::write(manifest, source).unwrap();
        fixture
    }
    fn package(&self, directory: &str, name: &str, inheritance: &str, binary: bool) {
        let directory = self.root.join(directory);
        fs::create_dir_all(directory.join("src")).unwrap();
        fs::write(
            directory.join("Cargo.toml"),
            format!("{inheritance}[package]\nname='{name}'\nversion='0.0.0'\nedition='2024'\n"),
        )
        .unwrap();
        fs::write(
            directory.join(if binary { "src/main.rs" } else { "src/lib.rs" }),
            if binary { "fn main() {}\n" } else { "" },
        )
        .unwrap();
    }
    fn raw_metadata(&self) -> anyhow::Result<serde_json::Value> {
        let output = Command::new("cargo")
            .args([
                "metadata",
                "--offline",
                "--no-deps",
                "--format-version",
                "1",
            ])
            .env("CARGO_BUILD_JOBS", "1")
            .env("CARGO_TARGET_DIR", self.root.join("target"))
            .current_dir(&self.root)
            .output()?;
        if !output.status.success() {
            bail!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    }
    fn metadata(&self) -> Metadata {
        serde_json::from_value(self.raw_metadata().unwrap()).unwrap()
    }
    fn omit_inheritance(&self, relative: &str) {
        let manifest = self.root.join(relative);
        fs::write(
            &manifest,
            fs::read_to_string(&manifest).unwrap().replace(INHERIT, ""),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn real_workspace_members_inherit_and_excluded_vendor_keeps_its_own_policy() {
    let fixture = Fixture::new();
    let vendor = fixture.root.join("vendor/upstream/Cargo.toml");
    let original = fs::read(&vendor).unwrap();
    let metadata = fixture.metadata();
    assert_eq!(metadata.workspace_members.len(), 3);
    assert!(check(&fixture.root, &metadata).is_ok());
    assert!(
        !metadata
            .packages
            .iter()
            .any(|package| package.manifest_path == vendor)
    );
    assert_eq!(fs::read(vendor).unwrap(), original);
}

#[test]
fn missing_inheritance_is_rejected_although_actual_cargo_metadata_succeeds() {
    let fixture = Fixture::new();
    fixture.omit_inheritance("crates/child/Cargo.toml");
    let metadata = fixture.metadata();
    let error = check(&fixture.root, &metadata).unwrap_err();
    assert!(error.to_string().contains("crates/child/Cargo.toml"));
    assert!(
        error
            .to_string()
            .contains("must inherit [lints] workspace = true")
    );
}

#[test]
fn architecture_rejects_dropped_inheritance_after_actual_workspace_discovery() {
    let fixture = Fixture::new();
    for relative in ["Cargo.toml", "crates/child/Cargo.toml", "xtask/Cargo.toml"] {
        let manifest = fixture.root.join(relative);
        let mut source = fs::read_to_string(&manifest).unwrap();
        source.push_str("[package.metadata.nocterm]\nlayer='domain'\n");
        fs::write(manifest, source).unwrap();
    }
    fixture.omit_inheritance("crates/child/Cargo.toml");
    let error = crate::architecture(&fixture.root).unwrap_err();
    assert!(error.to_string().contains("crates/child/Cargo.toml"));
    assert!(
        error
            .to_string()
            .contains("must inherit [lints] workspace = true")
    );
}

#[test]
fn root_and_binary_tool_members_cannot_omit_inheritance() {
    for relative in ["Cargo.toml", "xtask/Cargo.toml"] {
        let fixture = Fixture::new();
        fixture.omit_inheritance(relative);
        let error = check(&fixture.root, &fixture.metadata()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&fixture.root.join(relative).display().to_string())
        );
    }
}

#[test]
fn cargo_wildcards_discover_new_proc_macro_and_vendor_named_first_party_members() {
    for name in ["new-member", "vendor-widget"] {
        let fixture = Fixture::new();
        let directory = format!("crates/{name}");
        fixture.package(&directory, name, "[lib]\nproc-macro=true\n", false);
        let metadata = fixture.metadata();
        assert_eq!(metadata.workspace_members.len(), 4);
        let error = check(&fixture.root, &metadata).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("crates/{name}/Cargo.toml"))
        );
    }
}

#[test]
fn boolean_true_is_required_and_inline_inheritance_is_semantically_valid() {
    let manifest = Path::new("crates/example/Cargo.toml");
    for source in ["[lints]\nworkspace=true", "lints={workspace=true}"] {
        assert!(inspect(manifest, source).is_ok(), "{source}");
    }
    for source in [
        "",
        "[lints]",
        "[lints]\nworkspace=false",
        "[lints]\nworkspace='true'",
        "[lints]\nworkspace=1",
        "[lints]\nworkspace=[true]",
        "[lints]\nworkspaces=true",
    ] {
        let error = inspect(manifest, source).unwrap_err();
        assert!(
            error.to_string().contains("must inherit"),
            "{source}: {error}"
        );
    }
}

#[test]
fn real_cargo_accepts_inline_inheritance_and_false_never_passes() {
    let fixture = Fixture::new();
    fixture.package("crates/child", "child", "lints={workspace=true}\n", false);
    assert!(check(&fixture.root, &fixture.metadata()).is_ok());
    fixture.package("crates/child", "child", "[lints]\nworkspace=false\n", false);
    match fixture.raw_metadata() {
        Ok(value) => {
            let metadata = serde_json::from_value(value).unwrap();
            assert!(check(&fixture.root, &metadata).is_err());
        }
        Err(error) => {
            assert!(error.to_string().contains("cargo metadata failed"));
            assert!(error.to_string().contains("workspace"));
        }
    }
}

#[test]
fn comments_strings_metadata_and_workspace_definitions_do_not_inherit() {
    let source = "# [lints]\n# workspace=true\n[package.metadata.lints]\nworkspace=true\ntext='[lints] workspace = true'\n[workspace.lints.rust]\nunsafe_code='forbid'\n";
    assert!(inspect(Path::new("Cargo.toml"), source).is_err());
}

#[test]
fn malformed_manifest_preserves_native_parse_source_and_affected_path() {
    let fixture = Fixture::new();
    let metadata = fixture.metadata();
    let manifest = fixture.root.join("crates/child/Cargo.toml");
    fs::write(&manifest, "[lints\nworkspace=true\n").unwrap();
    let error = check(&fixture.root, &metadata).unwrap_err();
    assert!(error.to_string().contains(&manifest.display().to_string()));
    assert!(error.downcast_ref::<toml::de::Error>().is_some());
    assert!(format!("{error:#}").contains("TOML"));
}

#[test]
fn missing_and_unreadable_manifest_never_pass_after_discovery() {
    for directory_instead in [false, true] {
        let fixture = Fixture::new();
        let metadata = fixture.metadata();
        let manifest = fixture.root.join("crates/child/Cargo.toml");
        fs::remove_file(&manifest).unwrap();
        if directory_instead {
            fs::create_dir(&manifest).unwrap();
        }
        let error = check(&fixture.root, &metadata).unwrap_err();
        assert!(error.to_string().contains(&manifest.display().to_string()));
        assert!(error.downcast_ref::<std::io::Error>().is_some());
    }
}

#[test]
fn discovery_requires_complete_unique_nonempty_workspace_ids() {
    let fixture = Fixture::new();
    for corruption in 0..7 {
        let mut metadata = fixture.metadata();
        match corruption {
            0 => {
                metadata.packages.pop();
            }
            1 => {
                metadata.packages[1].id = metadata.packages[0].id.clone();
            }
            2 => {
                metadata.workspace_members.pop();
            }
            3 => {
                metadata
                    .workspace_members
                    .push(metadata.workspace_members[0].clone());
            }
            4 => {
                metadata.workspace_members[0].clear();
            }
            5 => {
                metadata.packages[0].id.clear();
            }
            _ => {
                metadata.workspace_members.clear();
                metadata.packages.clear();
            }
        }
        let error = check(&fixture.root, &metadata).unwrap_err();
        assert!(format!("{error:#}").contains("workspace"));
    }
}

#[test]
fn required_metadata_fields_cannot_disappear_during_deserialization() {
    let fixture = Fixture::new();
    let mut value = fixture.raw_metadata().unwrap();
    value.as_object_mut().unwrap().remove("workspace_members");
    assert!(serde_json::from_value::<Metadata>(value).is_err());
    let mut value = fixture.raw_metadata().unwrap();
    value["packages"][0].as_object_mut().unwrap().remove("id");
    assert!(serde_json::from_value::<Metadata>(value).is_err());
}

#[test]
fn vendor_component_is_a_boundary_but_unclassified_and_outside_paths_fail() {
    let fixture = Fixture::new();
    assert!(
        !first_party(
            &fixture.root,
            &fixture.root.join("vendor/upstream/Cargo.toml")
        )
        .unwrap()
    );
    assert!(
        first_party(
            &fixture.root,
            &fixture.root.join("crates/vendor-widget/Cargo.toml")
        )
        .unwrap()
    );
    for manifest in [
        fixture.root.join("vendor-widget/Cargo.toml"),
        fixture.root.join("extras/child/Cargo.toml"),
        fixture.root.join("crates/../extra/Cargo.toml"),
        fixture.root.parent().unwrap().join("external/Cargo.toml"),
    ] {
        let error = first_party(&fixture.root, &manifest).unwrap_err();
        assert!(format!("{error:#}").contains(&manifest.display().to_string()));
    }
}
