//! Pure path resolution and bounded completion state. No filesystem or UI work.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Directory {
    Local(PathBuf),
    Remote(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Query {
    pub directory: Directory,
    pub head: String,
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    pub name: String,
    pub directory: bool,
}

pub(super) fn resolve(
    value: &str,
    base: &Directory,
    home: Option<&Directory>,
) -> Result<Directory, String> {
    if value.is_empty() || value.contains('\0') {
        return Err("Enter a directory path.".into());
    }
    if value == "~" {
        return home
            .cloned()
            .ok_or_else(|| "The home directory is unavailable.".into());
    }
    if let Some(relative) = value.strip_prefix("~/").or_else(|| {
        matches!(base, Directory::Local(_))
            .then(|| value.strip_prefix("~\\"))
            .flatten()
            .filter(|_| cfg!(windows))
    }) {
        return home
            .map(|home| join(home, relative))
            .ok_or_else(|| "The home directory is unavailable.".into());
    }
    Ok(join(base, value))
}

fn join(base: &Directory, value: &str) -> Directory {
    match base {
        Directory::Local(base) => Directory::Local(base.join(value)),
        Directory::Remote(base) => Directory::Remote(if value.starts_with('/') {
            value.into()
        } else {
            nocterm_session::fs::path::join(base, value)
        }),
    }
}

pub(super) fn query(
    value: &str,
    base: &Directory,
    home: Option<&Directory>,
) -> Result<Query, String> {
    let value = if value == "~" { "~/" } else { value };
    let separator = value
        .rfind(|c| c == '/' || (c == '\\' && cfg!(windows) && matches!(base, Directory::Local(_))));
    let (head, prefix) = match separator {
        Some(index) => (&value[..index + 1], &value[index + 1..]),
        None => ("", value),
    };
    Ok(Query {
        directory: if head.is_empty() {
            base.clone()
        } else {
            resolve(head, base, home)?
        },
        head: head.into(),
        prefix: prefix.into(),
    })
}

#[derive(Default)]
pub(super) struct Cycle {
    pub candidates: Vec<Candidate>,
    pub selected: Option<usize>,
    pub head: String,
}
impl Cycle {
    pub(super) fn new(query: Query, entries: &[Candidate]) -> Self {
        Self {
            candidates: entries
                .iter()
                .filter(|entry| entry.directory && entry.name.starts_with(&query.prefix))
                .cloned()
                .collect(),
            head: query.head,
            selected: None,
        }
    }
    pub(super) fn step(&mut self, reverse: bool) -> Option<String> {
        let count = self.candidates.len();
        if count == 0 {
            return None;
        }
        let index = match (self.selected, reverse) {
            (None, false) => 0,
            (None, true) => count - 1,
            (Some(index), false) => (index + 1) % count,
            (Some(index), true) => (index + count - 1) % count,
        };
        self.choose(index)
    }
    pub(super) fn choose(&mut self, index: usize) -> Option<String> {
        let entry = self.candidates.get(index)?;
        self.selected = Some(index);
        Some(format!(
            "{}{}{}",
            self.head,
            entry.name,
            if entry.directory { "/" } else { "" }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Directory {
        Directory::Remote("/work/current".into())
    }
    #[test]
    fn relative_absolute_home_and_spaces_remain_literal() {
        let home = Directory::Remote("/home/資料".into());
        assert_eq!(
            resolve("../a b/", &base(), None).unwrap(),
            Directory::Remote("/work/current/../a b/".into())
        );
        assert_eq!(
            resolve("/root/", &base(), None).unwrap(),
            Directory::Remote("/root/".into())
        );
        assert_eq!(
            resolve("~/a", &base(), Some(&home)).unwrap(),
            Directory::Remote("/home/資料/a".into())
        );
        assert_eq!(resolve("~", &base(), Some(&home)).unwrap(), home);
        assert!(resolve("~/a", &base(), None).is_err());
        assert!(resolve("", &base(), None).is_err());
        assert!(resolve("a\0b", &base(), None).is_err());
    }
    #[test]
    fn query_handles_root_empty_relative_and_home() {
        for (input, directory, head, prefix) in [
            ("/", "/", "/", ""),
            ("/etc/ss", "/etc/", "/etc/", "ss"),
            ("", "/work/current", "", ""),
            ("../.hid", "/work/current/../", "../", ".hid"),
            ("~/資料", "/home/test/", "~/", "資料"),
        ] {
            assert_eq!(
                query(
                    input,
                    &base(),
                    Some(&Directory::Remote("/home/test".into()))
                )
                .unwrap(),
                Query {
                    directory: Directory::Remote(directory.into()),
                    head: head.into(),
                    prefix: prefix.into()
                }
            );
        }
    }
    #[test]
    fn candidates_exclude_files_and_cycle_directories_both_ways_without_narrowing() {
        let entries = vec![
            Candidate {
                name: "abc".into(),
                directory: true,
            },
            Candidate {
                name: "abd".into(),
                directory: true,
            },
            Candidate {
                name: "ab-file".into(),
                directory: false,
            },
        ];
        let mut cycle = Cycle::new(query("ab", &base(), None).unwrap(), &entries);
        assert_eq!(cycle.candidates.len(), 2);
        assert_eq!(cycle.step(false).as_deref(), Some("abc/"));
        assert_eq!(cycle.step(false).as_deref(), Some("abd/"));
        assert_eq!(cycle.step(false).as_deref(), Some("abc/"));
        assert_eq!(cycle.step(true).as_deref(), Some("abd/"));
        let mut reverse = Cycle::new(query("ab", &base(), None).unwrap(), &entries);
        assert_eq!(reverse.step(true).as_deref(), Some("abd/"));
        assert!(
            Cycle::new(query("z", &base(), None).unwrap(), &entries)
                .step(false)
                .is_none()
        );
    }
    #[test]
    fn local_resolution_does_not_normalize_away_symlink_parent_traversal() {
        let base = Directory::Local(PathBuf::from("/tmp/current"));
        assert_eq!(
            resolve("link/../data", &base, None).unwrap(),
            Directory::Local(PathBuf::from("/tmp/current/link/../data"))
        );
    }
}
