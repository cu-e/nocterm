//! Real Cargo manifests exercise feature bindings and dependency kinds.
use super::*;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(features: &str, dependencies: &str, child_features: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "nocterm-support-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("session/src")).unwrap();
        fs::write(root.join("Cargo.toml"), format!("[package]\nname='fixture-app'\nversion='0.0.0'\nedition='2024'\n[workspace]\nmembers=['session']\n[features]\n{features}\n{dependencies}\n")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("session/Cargo.toml"), format!("[package]\nname='session'\nversion='0.0.0'\nedition='2024'\n[features]\n{child_features}\n")).unwrap();
        fs::write(root.join("session/src/lib.rs"), "#[cfg(feature=\"test-support\")] pub fn fake()->&'static str {\"runtime fake support active\"}\n").unwrap();
        Self { root }
    }
    fn cargo(&self) -> Command {
        let mut command = Command::new("cargo");
        command
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", self.root.join("target"));
        command
    }
    fn metadata(&self) -> Metadata {
        let output = self
            .cargo()
            .args([
                "metadata",
                "--offline",
                "--no-deps",
                "--format-version",
                "1",
            ])
            .current_dir(&self.root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut metadata: Metadata = serde_json::from_slice(&output.stdout).unwrap();
        // Ordering is unspecified; assert the root forwarding diagnostic before
        // any independently invalid local alias in the child package.
        metadata
            .packages
            .sort_by_key(|package| package.name != "fixture-app");
        metadata
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn owning_support_cannot_forward_a_runtime_fake_in_real_cargo() {
    let fixture = Fixture::new(
        "test-support=['session/test-support']",
        "[dependencies]\nsession={path='session',default-features=false}",
        "test-support=[]",
    );
    fs::write(
        fixture.root.join("src/main.rs"),
        "fn main(){println!(\"{}\", session::fake());}\n",
    )
    .unwrap();
    let output = fixture
        .cargo()
        .args(["run", "--offline", "--features", "test-support"])
        .current_dir(&fixture.root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "runtime fake support active"
    );
    assert!(check(&fixture.metadata()).is_err());
}

#[test]
fn normal_build_renamed_optional_and_combined_bindings_reject_forwarding() {
    for dependencies in [
        "[dependencies]\nremote={package='session',path='session',default-features=false}",
        "[build-dependencies]\nremote={package='session',path='session',default-features=false}",
        "[dependencies]\nremote={package='session',path='session',optional=true,default-features=false}",
        "[dependencies]\nremote={package='session',path='session',default-features=false}\n[dev-dependencies]\nremote={package='session',path='session',default-features=false}",
    ] {
        let fixture = Fixture::new(
            "test-support=['remote/support-alias']",
            dependencies,
            "test-support=[]\nsupport-alias=['bridge']\nbridge=['test-support']",
        );
        let error = check(&fixture.metadata()).unwrap_err();
        assert!(
            error.to_string().contains("fixture-app"),
            "{dependencies}: {error}"
        );
    }
}

#[test]
fn weak_forwarding_and_child_defaults_do_not_hide_support() {
    for features in [
        "test-support=['dep:remote','remote?/support-alias']",
        "test-support=['remote/default']",
        "test-support=['forward']\nforward=['remote/support-alias']",
    ] {
        let fixture = Fixture::new(
            features,
            "[dependencies]\nremote={package='session',path='session',optional=true,default-features=false}",
            "test-support=[]\ndefault=['support-alias']\nsupport-alias=['test-support']",
        );
        assert!(check(&fixture.metadata()).is_err(), "{features}");
    }
}

#[test]
fn empty_local_and_development_only_support_definitions_remain_allowed() {
    for features in [
        "test-support=[]",
        "test-support=['local']\nlocal=[]",
        "test-support=['remote/test-support']",
        "ordinary=['remote/test-support']\ntest-support=[]",
        "default=['dev-mode']\ndev-mode=['remote/test-support']\ntest-support=[]",
    ] {
        let fixture = Fixture::new(
            features,
            "[dev-dependencies]\nremote={package='session',path='session',default-features=false}",
            "test-support=[]",
        );
        assert!(check(&fixture.metadata()).is_ok(), "{features}");
        let output = fixture
            .cargo()
            .args([
                "check",
                "--offline",
                "--all-targets",
                "--features",
                "test-support",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{features}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let fixture = Fixture::new(
        "test-support=[]",
        "[dev-dependencies]\nsession={path='session',features=['test-support']}",
        "test-support=[]",
    );
    assert!(check(&fixture.metadata()).is_ok());
}

#[test]
fn declared_runtime_features_defaults_and_local_support_aliases_remain_rejected() {
    for dependencies in [
        "[dependencies]\nsession={path='session',features=['test-support'],default-features=false}",
        "[build-dependencies]\nsession={path='session',features=['support-alias'],default-features=false}",
        "[dependencies]\nsession={path='session'}",
    ] {
        let fixture = Fixture::new(
            "test-support=[]",
            dependencies,
            "test-support=[]\ndefault=['support-alias']\nsupport-alias=['test-support']",
        );
        let error = check(&fixture.metadata()).unwrap_err();
        assert!(
            error.to_string().contains("fixture-app"),
            "{dependencies}: {error}"
        );
    }
    let fixture = Fixture::new(
        "default=['cycle']\ncycle=['cycle','test-support']\ntest-support=[]",
        "",
        "test-support=[]",
    );
    assert!(check(&fixture.metadata()).is_err());
    let fixture = Fixture::new("test-support=[]\ncycle=['cycle']", "", "test-support=[]");
    assert!(check(&fixture.metadata()).is_ok());
}
