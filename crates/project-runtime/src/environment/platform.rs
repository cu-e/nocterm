use super::*;
use nocterm_project_providers::{
    DistributionFamily, ProgrammingLanguage, SetupPhase, VersionChoice,
};

pub(crate) struct InstallPlatform {
    family: DistributionFamily,
    musl: bool,
    architecture: String,
    freshly_created: bool,
}
impl InstallPlatform {
    pub(crate) fn new(
        snapshot: &EnvironmentSnapshot,
        freshly_created: bool,
    ) -> Result<Self, EnvironmentError> {
        Ok(Self {
            family: snapshot.os.family,
            musl: snapshot.os.libc == nocterm_project_providers::LibcFamily::Musl,
            architecture: snapshot.os.architecture.clone(),
            freshly_created,
        })
    }
    pub(crate) fn musl(&self) -> bool {
        self.musl
    }
    pub(crate) fn architecture(&self) -> &str {
        &self.architecture
    }
    pub(crate) async fn prepare(
        &self,
        commands: &Commands,
        snapshot: &EnvironmentSnapshot,
        selections: &[LanguageSelection],
    ) -> Result<(), EnvironmentError> {
        if snapshot.account.uid != 0 {
            return self.check_user_prerequisites(snapshot, selections);
        }
        if self.family == DistributionFamily::Arch
            && !snapshot
                .available_programs
                .iter()
                .any(|program| program == "pacman-database")
        {
            if self.freshly_created {
                commands
                    .run(
                        SetupPhase::InstallingTools,
                        ExecRequest::new("pacman").args(["-Syu", "--noconfirm"]),
                    )
                    .await?;
            } else {
                return Err(EnvironmentError::MissingPrerequisite {
                    program: "pacman package databases",
                    packages: "an administrator-completed pacman -Syu system update before installing workspace tools",
                });
            }
        }
        let (manager, initial, args) = match self.family {
            DistributionFamily::Debian => (
                "apt-get",
                Some(ExecRequest::new("apt-get").arg("update")),
                vec!["install", "-y", "--no-install-recommends"],
            ),
            DistributionFamily::Alpine => ("apk", None, vec!["add", "--no-cache"]),
            DistributionFamily::Arch => ("pacman", None, vec!["-S", "--needed", "--noconfirm"]),
            DistributionFamily::Unsupported => {
                return Err(EnvironmentError::UnsupportedOs(snapshot.os.id.clone()));
            }
        };
        if !snapshot
            .available_programs
            .iter()
            .any(|program| program == manager)
        {
            return Err(EnvironmentError::MissingPrerequisite {
                program: manager,
                packages: "the distribution package manager",
            });
        }
        if let Some(request) = initial {
            commands.run(SetupPhase::InstallingTools, request).await?;
        }
        let mut packages: Vec<&str> = self.prerequisites().split_whitespace().collect();
        for selection in selections {
            if selection.language() == ProgrammingLanguage::Rust {
                packages.extend(self.build_packages().split_whitespace());
            }
            if matches!(selection.version(), VersionChoice::DistributionDefault) {
                packages.extend(
                    self.distribution_packages(selection.language())
                        .split_whitespace(),
                );
            }
        }
        packages.sort_unstable();
        packages.dedup();
        let request = ExecRequest::new("sh")
            .args([
                "-c",
                "export DEBIAN_FRONTEND=noninteractive; exec \"$@\"",
                "nocterm-packages",
                manager,
            ])
            .args(args)
            .args(packages);
        commands.run(SetupPhase::InstallingTools, request).await?;
        Ok(())
    }
    fn check_user_prerequisites(
        &self,
        snapshot: &EnvironmentSnapshot,
        selections: &[LanguageSelection],
    ) -> Result<(), EnvironmentError> {
        for selection in selections {
            if matches!(selection.version(), VersionChoice::DistributionDefault) {
                if !snapshot
                    .tools
                    .iter()
                    .any(|tool| tool.language == selection.language() && tool.version.is_some())
                {
                    return Err(EnvironmentError::PrivilegeRequired);
                }
            } else {
                for program in [
                    "curl",
                    "tar",
                    "gzip",
                    "xz",
                    "unzip",
                    "sha256sum",
                    "awk",
                    "mktemp",
                    "bash",
                ] {
                    if !snapshot
                        .available_programs
                        .iter()
                        .any(|available| available == program)
                    {
                        return Err(EnvironmentError::MissingPrerequisite {
                            program,
                            packages: self.prerequisites(),
                        });
                    }
                }
                if selection.language() == ProgrammingLanguage::Rust
                    && !snapshot
                        .available_programs
                        .iter()
                        .any(|program| program == "cc")
                {
                    return Err(EnvironmentError::MissingPrerequisite {
                        program: "cc",
                        packages: self.build_packages(),
                    });
                }
            }
        }
        Ok(())
    }
    fn prerequisites(&self) -> &'static str {
        match self.family {
            DistributionFamily::Debian => {
                "ca-certificates git curl bash tar gzip xz-utils unzip coreutils findutils libstdc++6 zlib1g libfreetype6"
            }
            DistributionFamily::Alpine => {
                "ca-certificates git curl bash tar gzip xz unzip coreutils findutils libstdc++ zlib freetype"
            }
            DistributionFamily::Arch => {
                "ca-certificates git curl bash tar gzip xz unzip coreutils findutils gcc-libs zlib freetype2"
            }
            DistributionFamily::Unsupported => "curl bash tar gzip xz unzip coreutils",
        }
    }
    fn build_packages(&self) -> &'static str {
        match self.family {
            DistributionFamily::Debian => "build-essential",
            DistributionFamily::Alpine => "build-base",
            _ => "base-devel",
        }
    }
    fn distribution_packages(&self, language: ProgrammingLanguage) -> &'static str {
        match (self.family, language) {
            (DistributionFamily::Debian, ProgrammingLanguage::Python) => {
                "python3 python3-pip python3-venv"
            }
            (DistributionFamily::Alpine, ProgrammingLanguage::Python) => "python3 py3-pip",
            (DistributionFamily::Arch, ProgrammingLanguage::Python) => "python python-pip",
            (_, ProgrammingLanguage::Node) => "nodejs npm",
            (DistributionFamily::Debian, ProgrammingLanguage::Java) => "default-jdk-headless",
            (DistributionFamily::Alpine, ProgrammingLanguage::Java) => "openjdk17-jdk",
            (DistributionFamily::Arch, ProgrammingLanguage::Java) => "jdk-openjdk",
            _ => "",
        }
    }
}
