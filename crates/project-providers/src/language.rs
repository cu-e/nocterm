use crate::ProjectSetupError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProgrammingLanguage {
    Python,
    Rust,
    Bun,
    Node,
    Java,
}
impl ProgrammingLanguage {
    pub const ALL: [Self; 5] = [Self::Python, Self::Rust, Self::Bun, Self::Node, Self::Java];
    pub fn name(self) -> &'static str {
        match self {
            Self::Python => "Python",
            Self::Rust => "Rust",
            Self::Bun => "Bun",
            Self::Node => "Node.js",
            Self::Java => "Java",
        }
    }
    pub fn mark(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Bun => "bun",
            Self::Node => "nodejs",
            Self::Java => "java",
        }
    }
}

/// A version request, checked against its language by `LanguageSelection::new`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionChoice {
    DistributionDefault,
    Stable,
    Latest,
    Numeric(String),
}
impl VersionChoice {
    pub fn as_str(&self) -> &str {
        match self {
            Self::DistributionDefault => "distribution",
            Self::Stable => "stable",
            Self::Latest => "latest",
            Self::Numeric(value) => value,
        }
    }
}

/// Immutable validated language/version pair; arbitrary shell fragments cannot enter it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageSelection {
    language: ProgrammingLanguage,
    version: VersionChoice,
}
impl LanguageSelection {
    pub fn new(
        language: ProgrammingLanguage,
        version: VersionChoice,
    ) -> Result<Self, ProjectSetupError> {
        let valid = match &version {
            VersionChoice::DistributionDefault => matches!(
                language,
                ProgrammingLanguage::Python | ProgrammingLanguage::Node | ProgrammingLanguage::Java
            ),
            VersionChoice::Stable => language == ProgrammingLanguage::Rust,
            VersionChoice::Latest => language == ProgrammingLanguage::Bun,
            VersionChoice::Numeric(value) => valid_numeric(language, value),
        };
        if !valid {
            return Err(ProjectSetupError::InvalidInput(
                "choose a supported language version",
            ));
        }
        Ok(Self { language, version })
    }
    pub fn default_for(language: ProgrammingLanguage) -> Self {
        let version = match language {
            ProgrammingLanguage::Rust => VersionChoice::Stable,
            ProgrammingLanguage::Bun => VersionChoice::Latest,
            _ => VersionChoice::DistributionDefault,
        };
        Self { language, version }
    }
    pub fn parse(language: ProgrammingLanguage, value: &str) -> Result<Self, ProjectSetupError> {
        let version = match value {
            "distribution" => VersionChoice::DistributionDefault,
            "stable" => VersionChoice::Stable,
            "latest" => VersionChoice::Latest,
            _ => VersionChoice::Numeric(value.to_owned()),
        };
        Self::new(language, version)
    }
    pub fn language(&self) -> ProgrammingLanguage {
        self.language
    }
    pub fn version(&self) -> &VersionChoice {
        &self.version
    }
}

fn valid_numeric(language: ProgrammingLanguage, value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    if value.len() > 32
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 5
                || !part.bytes().all(|b| b.is_ascii_digit())
                || part.len() > 1 && part.starts_with('0')
        })
    {
        return false;
    }
    match language {
        ProgrammingLanguage::Python => {
            matches!(parts.len(), 2 | 3) && parts[0] == "3" && parts[1] != "0"
        }
        // rustup archives require a complete release, rather than an ambiguous minor.
        ProgrammingLanguage::Rust => parts.len() == 3 && parts[0] == "1",
        ProgrammingLanguage::Bun => parts.len() == 3 && parts[0] != "0",
        ProgrammingLanguage::Node => matches!(parts.len(), 1 | 3) && parts[0] != "0",
        ProgrammingLanguage::Java => parts.len() == 1 && parts[0] != "0",
    }
}
