use super::*;
use crate::fixtures::{Monitor, Terminal};

fn document(text: &str) -> SettingsDocument {
    SettingsDocument::from_table(toml::from_str(text).unwrap())
}

#[test]
fn an_unregistered_section_absent_from_the_file_is_at_its_defaults() {
    let document = SettingsDocument::default();
    assert_eq!(document.get::<Terminal>(), &Terminal::default());
    assert!(!document.is_registered::<Terminal>());
}

#[test]
fn registering_reads_the_section_and_sanitizes_it() {
    let mut document = document("[terminal]\nfont_size = 500.0\n");
    document.register::<Terminal>();
    assert_eq!(document.get::<Terminal>().font_size, Some(72.0));
}

#[test]
fn a_broken_section_leaves_the_others_alone() {
    let mut document = document("[terminal]\nfont_szie = 1\n[monitor]\ninterval_secs = 7\n");
    document.register::<Terminal>();
    document.register::<Monitor>();
    assert_eq!(document.get::<Terminal>(), &Terminal::default());
    assert_eq!(document.get::<Monitor>().interval_secs, 7);
    let errors = document.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].key, "terminal");
    assert!(errors[0].error.contains("font_szie"), "{}", errors[0]);
}

#[test]
fn updating_sanitizes_and_reports_changes() {
    let mut document = SettingsDocument::default();
    assert!(!document.update::<Terminal>(|terminal| terminal.font_size = Some(f32::NAN)));
    assert!(document.update::<Terminal>(|terminal| terminal.font_size = Some(100.0)));
    assert_eq!(document.get::<Terminal>().font_size, Some(72.0));
    assert!(!document.update::<Terminal>(|terminal| terminal.font_size = Some(72.0)));
}

#[test]
fn an_unchanged_update_keeps_a_broken_table_and_its_error() {
    let mut document = document("[terminal]\nfont_szie = 1\n");
    document.update::<Terminal>(|_| {});
    assert_eq!(document.errors().len(), 1);
    assert!(document.to_table().contains_key("terminal"));
    assert_eq!(
        document.to_table()["terminal"].as_table().unwrap()["font_szie"],
        toml::Value::Integer(1)
    );
}

#[test]
fn unclaimed_tables_are_reported_once_sections_are_registered() {
    let mut document = document("[future]\nflag = true\n");
    assert!(document.errors().is_empty());
    document.register::<Monitor>();
    let errors = document.errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].key, "future");
    assert!(errors[0].error.contains("`monitor`"), "{}", errors[0]);
}

#[test]
fn documents_compare_by_their_sections() {
    let mut left = SettingsDocument::default();
    let mut right = SettingsDocument::default();
    left.register::<Monitor>();
    assert_ne!(left, right);
    right.register::<Monitor>();
    assert_eq!(left, right);
    let copy = left.clone();
    left.update::<Monitor>(|monitor| monitor.interval_secs = 3);
    assert_ne!(left, copy);
}
