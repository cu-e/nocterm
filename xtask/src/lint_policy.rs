//! Complexity expectations are an inventory of old guarded lints, including tests.
use anyhow::bail;
use std::{fs, path::Path};
use syn::{
    Meta, Token,
    punctuated::Punctuated,
    visit::{self, Visit},
};

const GUARDED: [&str; 2] = ["too_many_lines", "cognitive_complexity"];

pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let mut files = Vec::new();
    for directory in ["src", "crates", "xtask"] {
        let directory = root.join(directory);
        if directory.is_dir() {
            crate::source_files(&directory, &mut files)?;
        }
    }
    let mut problems = Vec::new();
    for file in files {
        let name = file.strip_prefix(root).unwrap_or(&file).display();
        let (_, violations) = inspect(&fs::read_to_string(&file)?)?;
        for (line, problem) in violations {
            problems.push(format!("{name}:{line}: {problem}"));
        }
    }
    if !problems.is_empty() {
        bail!("complexity exception policy:\n{}", problems.join("\n"));
    }
    Ok(())
}

pub(crate) fn inspect(text: &str) -> anyhow::Result<(usize, Vec<(usize, String)>)> {
    let mut inspector = Inspector {
        count: 0,
        problems: Vec::new(),
    };
    inspector.visit_file(&syn::parse_file(text)?);
    Ok((inspector.count, inspector.problems))
}

struct Inspector {
    count: usize,
    problems: Vec<(usize, String)>,
}

impl<'ast> Visit<'ast> for Inspector {
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        self.meta(
            &attribute.meta,
            attribute.bracket_token.span.open().start().line,
        );
        visit::visit_attribute(self, attribute);
    }
}

impl Inspector {
    fn meta(&mut self, meta: &Meta, line: usize) {
        let Meta::List(list) = meta else { return };
        let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        else {
            return;
        };
        if list.path.is_ident("cfg_attr") {
            for meta in arguments.iter().skip(1) {
                self.meta(meta, line);
            }
            return;
        }
        let guarded = arguments
            .iter()
            .filter(|argument| {
                let path = argument.path();
                path.segments.len() == 2
                    && path.segments[0].ident == "clippy"
                    && GUARDED.iter().any(|name| path.segments[1].ident == *name)
            })
            .count();
        if guarded == 0 {
            return;
        }
        if !list.path.is_ident("expect") {
            self.problems.push((line, "guarded complexity lints require an existing #[expect], never allow/warn/deny overrides".into()));
            return;
        }
        self.count += guarded;
        let reason = arguments.iter().find_map(|meta| {
            let Meta::NameValue(value) = meta else {
                return None;
            };
            if !value.path.is_ident("reason") {
                return None;
            }
            let syn::Expr::Lit(literal) = &value.value else {
                return None;
            };
            let syn::Lit::Str(reason) = &literal.lit else {
                return None;
            };
            Some(reason.value())
        });
        if reason.as_deref() != Some("predates the limit") {
            self.problems.push((
                line,
                "guarded expectations require reason = \"predates the limit\"".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_conditional_exceptions_cannot_bypass_the_inventory() {
        let source = r#"
@[expect(clippy::too_many_lines, reason = "predates the limit")]
fn a() {}
@[cfg_attr(test, cfg_attr(unix, expect(clippy::cognitive_complexity, reason = "new work")))]
fn b() {}
@![allow(clippy::cognitive_complexity)]
@[expect(clippy::too_many_lines)]
fn c() {}
@[allow(clippy::needless_pass_by_value)]
fn d() {}
"#
        .replace('@', "#");
        // Inner crate attributes must precede items, so test that override separately.
        let source = source.replace(
            "#![allow(clippy::cognitive_complexity)]",
            "#[allow(clippy::cognitive_complexity)]",
        );
        let (count, problems) = inspect(&source).unwrap();
        assert_eq!(count, 3);
        assert_eq!(problems.len(), 3);
    }

    #[test]
    fn comments_and_literals_are_not_attributes() {
        let source = r##"fn a() { let _ = "#[allow(clippy::too_many_lines)]"; }
// #[expect(clippy::too_many_lines)]
"##;
        let (count, problems) = inspect(source).unwrap();
        assert_eq!(count, 0);
        assert!(problems.is_empty());
    }
}
