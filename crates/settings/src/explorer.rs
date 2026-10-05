//! `[explorer]`: folder statistics and the programs that open files.

use std::ops::RangeInclusive;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Entry limits a user may pick for one folder's statistics.
pub const INDEXING_ENTRIES_RANGE: RangeInclusive<u32> = 1_000..=10_000_000;
/// The placeholder an argument may use for the file being opened.
pub const FILE_PLACEHOLDER: &str = "{file}";

/// The Explorer in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ExplorerSettings {
    /// Counting the files and the size of the folder being shown.
    pub indexing: IndexingSettings,
    /// The programs that open files.
    pub open: OpenSettings,
}

/// Counting the files and the size of the folder being shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IndexingSettings {
    /// Count the files and the size of local folders.
    pub local: bool,
    /// Count the files and the size of remote folders, over SFTP.
    pub remote: bool,
    /// Do not count a home directory itself: it is usually huge. Its
    /// subfolders are still counted.
    pub skip_home: bool,
    /// Do not count the root of a file system (`/`, `C:\`).
    pub skip_root: bool,
    /// Folders with these names are never entered while counting, wherever
    /// they are. The folder being shown is always counted.
    pub excluded: Vec<String>,
    /// Stop counting a local folder after this many entries.
    #[schemars(extend("minimum" = INDEXING_ENTRIES_RANGE.start(), "maximum" = INDEXING_ENTRIES_RANGE.end()))]
    pub max_local_entries: u32,
    /// Stop counting a remote folder after this many entries. Every folder
    /// costs a round trip to the server.
    #[schemars(extend("minimum" = INDEXING_ENTRIES_RANGE.start(), "maximum" = INDEXING_ENTRIES_RANGE.end()))]
    pub max_remote_entries: u32,
}

impl Default for IndexingSettings {
    fn default() -> Self {
        Self {
            local: true,
            remote: true,
            skip_home: true,
            skip_root: true,
            excluded: [
                ".cache",
                ".git",
                "node_modules",
                "target",
                ".cargo",
                ".rustup",
                ".npm",
                ".gradle",
                ".m2",
                ".venv",
                "__pycache__",
                ".local",
                ".var",
                "snap",
                "proc",
                "sys",
                "dev",
            ]
            .map(str::to_owned)
            .to_vec(),
            max_local_entries: 1_000_000,
            max_remote_entries: 20_000,
        }
    }
}

impl IndexingSettings {
    /// Whether counting may enter a folder called `name`.
    pub fn enters(&self, name: &str) -> bool {
        !self.excluded.iter().any(|excluded| excluded == name)
    }
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Opener {
    /// The executable, such as `nvim` or `code`. Empty uses the default.
    pub program: String,
    /// Arguments, passed one by one without shell parsing. `{file}` stands for
    /// the file; without it the file is added last.
    pub args: Vec<String>,
}

impl Opener {
    /// A program with arguments.
    pub fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        }
    }

    /// Whether this opener names a program.
    pub fn is_set(&self) -> bool {
        !self.program.trim().is_empty()
    }

    /// The arguments for opening `file`.
    pub fn arguments(&self, file: &str) -> Vec<String> {
        let mut placed = false;
        let mut arguments: Vec<String> = self
            .args
            .iter()
            .map(|arg| {
                placed |= arg.contains(FILE_PLACEHOLDER);
                arg.replace(FILE_PLACEHOLDER, file)
            })
            .collect();
        if !placed {
            arguments.push(file.to_owned());
        }
        arguments
    }

    fn sanitize(&mut self) {
        self.program = self.program.trim().to_owned();
        self.args.retain(|arg| !arg.contains('\0'));
        if self.program.contains('\0') {
            self.program.clear();
        }
    }
}

/// The programs that open files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OpenSettings {
    /// Opens files on this computer. Empty uses the system's default
    /// application for the file.
    pub local: Opener,
    /// Opens files on a server, in a new terminal tab on that server: an
    /// editor such as `nano`, `vim` or `nvim`.
    pub remote: Opener,
    /// Programs for particular file types. The first rule naming a file's
    /// extension wins; a rule's empty program falls back to the defaults
    /// above.
    pub rules: Vec<OpenRule>,
}

impl Default for OpenSettings {
    fn default() -> Self {
        Self {
            local: Opener::default(),
            remote: Opener::new("nano", &[]),
            rules: Vec::new(),
        }
    }
}

/// Programs for files with particular extensions.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct OpenRule {
    /// Extensions without the dot, such as `md` or `tar.gz`, or whole file
    /// names such as `Dockerfile`. Case is ignored.
    pub extensions: Vec<String>,
    /// Opens matching files on this computer.
    pub local: Opener,
    /// Opens matching files on a server.
    pub remote: Opener,
}

impl OpenRule {
    /// Whether the rule covers a file called `name`.
    pub fn matches(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.extensions.iter().any(|extension| {
            let extension = extension.to_lowercase();
            name == extension
                || name
                    .strip_suffix(&extension)
                    .is_some_and(|stem| stem.len() > 1 && stem.ends_with('.'))
        })
    }
}

impl OpenSettings {
    /// The program that opens a file called `name`: on a server when
    /// `remote`, else on this computer. `None` on this computer means the
    /// system default.
    pub fn opener_for(&self, name: &str, remote: bool) -> Option<&Opener> {
        fn pick<'a>(local: &'a Opener, server: &'a Opener, remote: bool) -> &'a Opener {
            if remote { server } else { local }
        }
        self.rules
            .iter()
            .filter(|rule| rule.matches(name))
            .map(|rule| pick(&rule.local, &rule.remote, remote))
            .chain([pick(&self.local, &self.remote, remote)])
            .find(|opener| opener.is_set())
    }
}

impl ExplorerSettings {
    pub(crate) fn sanitize(&mut self) {
        let indexing = &mut self.indexing;
        let clamp = |value: u32| {
            value.clamp(
                *INDEXING_ENTRIES_RANGE.start(),
                *INDEXING_ENTRIES_RANGE.end(),
            )
        };
        indexing.max_local_entries = clamp(indexing.max_local_entries);
        indexing.max_remote_entries = clamp(indexing.max_remote_entries);
        indexing.excluded = clean_names(std::mem::take(&mut indexing.excluded));
        self.open.local.sanitize();
        self.open.remote.sanitize();
        for rule in &mut self.open.rules {
            rule.extensions = clean_names(std::mem::take(&mut rule.extensions))
                .into_iter()
                .map(|extension| extension.trim_start_matches('.').to_owned())
                .filter(|extension| !extension.is_empty())
                .collect();
            rule.local.sanitize();
            rule.remote.sanitize();
        }
    }
}

/// Trimmed, nonempty, separator-free names, each once, in order.
fn clean_names(names: Vec<String>) -> Vec<String> {
    let mut clean: Vec<String> = Vec::with_capacity(names.len());
    for name in names {
        let name = name.trim();
        if !name.is_empty()
            && !name.contains(['/', '\\', '\0'])
            && !clean.iter().any(|existing| existing == name)
        {
            clean.push(name.to_owned());
        }
    }
    clean
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_match_extensions_and_names_case_insensitively() {
        let rule = OpenRule {
            extensions: vec!["md".into(), "tar.gz".into(), "Dockerfile".into()],
            ..OpenRule::default()
        };
        assert!(rule.matches("README.MD"));
        assert!(rule.matches("backup.tar.gz"));
        assert!(rule.matches("dockerfile"));
        assert!(!rule.matches("notes.mdx"));
        assert!(!rule.matches(".md"));
    }

    #[test]
    fn first_rule_with_a_program_wins_then_the_default() {
        let open = OpenSettings {
            rules: vec![
                OpenRule {
                    extensions: vec!["rs".into()],
                    local: Opener::new("code", &[]),
                    remote: Opener::default(),
                },
                OpenRule {
                    extensions: vec!["rs".into()],
                    local: Opener::new("zed", &[]),
                    remote: Opener::new("nvim", &[]),
                },
            ],
            ..OpenSettings::default()
        };
        assert_eq!(open.opener_for("main.rs", false).unwrap().program, "code");
        assert_eq!(open.opener_for("main.rs", true).unwrap().program, "nvim");
        assert_eq!(open.opener_for("notes.txt", true).unwrap().program, "nano");
        assert!(
            open.opener_for("notes.txt", false).is_none(),
            "system default"
        );
    }

    #[test]
    fn arguments_place_the_file_or_append_it() {
        let opener = Opener::new("vim", &["+{file}:1", "-R"]);
        assert_eq!(opener.arguments("/a b"), ["+/a b:1", "-R"]);
        let opener = Opener::new("nvim", &["-R"]);
        assert_eq!(opener.arguments("/x"), ["-R", "/x"]);
    }

    #[test]
    fn sanitizing_cleans_names_and_clamps_limits() {
        let mut explorer = ExplorerSettings::default();
        explorer.indexing.excluded = vec![" a ".into(), "a".into(), "".into(), "x/y".into()];
        explorer.indexing.max_remote_entries = 0;
        explorer.open.rules = vec![OpenRule {
            extensions: vec![".md".into(), " ".into()],
            local: Opener::new(" code ", &[]),
            remote: Opener::default(),
        }];
        explorer.sanitize();
        assert_eq!(explorer.indexing.excluded, ["a"]);
        assert_eq!(explorer.indexing.max_remote_entries, 1_000);
        assert_eq!(explorer.open.rules[0].extensions, ["md"]);
        assert_eq!(explorer.open.rules[0].local.program, "code");
        let mut defaults = ExplorerSettings::default();
        defaults.sanitize();
        assert_eq!(defaults, ExplorerSettings::default());
    }
}
