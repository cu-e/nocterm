use crate::environment::{paths::InstallPaths, platform::InstallPlatform};
use nocterm_project_providers::{
    EnvironmentError, LanguageSelection, ProgrammingLanguage, VersionChoice,
};
use nocterm_session::ExecRequest;
mod java;
mod layout;
mod node;
mod python;
mod rust_bun;

/// One shared installer for newly created containers and explicitly admitted existing environments.
pub(crate) fn installation(
    selection: &LanguageSelection,
    platform: &InstallPlatform,
    paths: &InstallPaths,
) -> Result<ExecRequest, EnvironmentError> {
    let mut script = String::from(layout::PREPARE);
    script.push_str(match (selection.language(), selection.version()) {
        (ProgrammingLanguage::Python, VersionChoice::DistributionDefault) => python::DISTRIBUTION,
        (ProgrammingLanguage::Python, _) => python::MANAGED,
        (ProgrammingLanguage::Rust, _) => rust_bun::RUST,
        (ProgrammingLanguage::Bun, _) => rust_bun::BUN,
        (ProgrammingLanguage::Node, VersionChoice::DistributionDefault) => node::DISTRIBUTION,
        (ProgrammingLanguage::Node, _) => node::OFFICIAL,
        (ProgrammingLanguage::Java, VersionChoice::DistributionDefault) => java::DISTRIBUTION,
        (ProgrammingLanguage::Java, _) => java::ADOPTIUM,
    });
    script.push_str(layout::FINISH);
    Ok(ExecRequest::new("sh")
        .args([
            "-s",
            "--",
            paths.home(),
            selection.version().as_str(),
            if platform.musl() { "musl" } else { "gnu" },
            platform.architecture(),
        ])
        .stdin(script.into_bytes()))
}
