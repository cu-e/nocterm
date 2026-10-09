//! The settings reference: one table of `settings.toml` per section.
//!
//! Every crate defines the sections it reads. The reference lists them here,
//! and the architecture check fails when a crate defines a section this list
//! forgets, so the documentation cannot silently fall behind.
use std::{any::type_name, collections::BTreeSet, fs, path::Path};

use anyhow::bail;
use nocterm_ai::AiSettings;
use nocterm_files::ExplorerSettings;
use nocterm_monitor_ui::MonitorSettings;
use nocterm_session::{LocalShellSettings, LoggingOptions, SshSettings};
use nocterm_settings::SettingsSection;
use nocterm_ui::{AppearanceSettings, TerminalSettings};
use nocterm_vault_ui::VaultSettings;
use schemars::schema_for;

/// A section's key, its type's name and its reference rows.
struct Section {
    key: &'static str,
    name: &'static str,
    rows: String,
}

fn section<S: SettingsSection>() -> anyhow::Result<Section> {
    let schema = serde_json::to_value(schema_for!(S))?;
    let defaults = serde_json::to_value(S::default())?;
    let mut rows = String::new();
    crate::schema_rows(&schema, &schema, &defaults, S::KEY, &mut rows)?;
    let name = type_name::<S>().rsplit("::").next().unwrap_or_default();
    Ok(Section {
        key: S::KEY,
        name,
        rows,
    })
}

/// Every section the application registers.
fn sections() -> anyhow::Result<Vec<Section>> {
    let mut sections = vec![
        section::<AiSettings>()?,
        section::<AppearanceSettings>()?,
        section::<ExplorerSettings>()?,
        section::<LocalShellSettings>()?,
        section::<LoggingOptions>()?,
        section::<MonitorSettings>()?,
        section::<SshSettings>()?,
        section::<TerminalSettings>()?,
        section::<VaultSettings>()?,
    ];
    sections.sort_by_key(|section| section.key);
    Ok(sections)
}

/// `docs/reference/settings.md`, with the sections in key order.
pub(crate) fn render() -> anyhow::Result<String> {
    let mut text = crate::reference_header("Settings");
    for section in sections()? {
        text.push_str(&section.rows);
    }
    Ok(text)
}

/// Fails when a crate implements `SettingsSection` for a type the reference
/// does not list. Sections inside modules and the settings crate's own test
/// fixtures are not counted.
pub(crate) fn check(root: &Path) -> anyhow::Result<()> {
    let listed: BTreeSet<_> = sections()?.iter().map(|section| section.name).collect();
    let mut files = Vec::new();
    for directory in ["src", "crates"] {
        crate::source_files(&root.join(directory), &mut files)?;
    }
    let mut missing = Vec::new();
    let fixtures = root.join("crates/settings");
    for file in files
        .into_iter()
        .filter(|file| !file.starts_with(&fixtures))
    {
        for name in implementations(&fs::read_to_string(&file)?)? {
            if !listed.contains(name.as_str()) {
                let file = file.strip_prefix(root).unwrap_or(&file).display();
                missing.push(format!("{name} ({file})"));
            }
        }
    }
    if !missing.is_empty() {
        bail!(
            "settings sections missing from xtask/src/settings_reference.rs: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

/// The types a file implements `SettingsSection` for at its top level.
fn implementations(source: &str) -> anyhow::Result<Vec<String>> {
    let mut names = Vec::new();
    for item in syn::parse_file(source)?.items {
        let syn::Item::Impl(item) = item else {
            continue;
        };
        let Some((_, path, _)) = &item.trait_ else {
            continue;
        };
        if !path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "SettingsSection")
        {
            continue;
        }
        if let syn::Type::Path(ty) = item.self_ty.as_ref()
            && let Some(segment) = ty.path.segments.last()
        {
            names.push(segment.ident.to_string());
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_reference_resolves_definitions_and_uses_actual_defaults() {
        let text = render().unwrap();
        assert!(text.contains("`terminal.scrollback_lines`"));
        assert!(text.contains("`10000`"));
        assert!(text.contains("Lines of history kept above the visible screen."));
        assert!(text.contains("system"));
        assert!(text.contains("light"));
        assert!(text.contains("dark"));
        assert!(text.contains("6.0–72.0"));
    }

    #[test]
    fn only_top_level_section_implementations_are_found() {
        let source = "impl nocterm_settings::SettingsSection for Terminal {}\n\
                      impl SettingsSection for crate::Monitor {}\n\
                      impl Default for Other {}\n\
                      mod tests { impl SettingsSection for Fixture {} }";
        assert_eq!(implementations(source).unwrap(), ["Terminal", "Monitor"]);
    }
}
