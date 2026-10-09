use std::fs;

use super::*;
use crate::fixtures::{Agent, Agents, Monitor, Terminal, registered};

fn file() -> (tempfile::TempDir, SettingsFile) {
    let directory = tempfile::tempdir().unwrap();
    let file = SettingsFile::new(directory.path().join("settings.toml"));
    (directory, file)
}

fn load(file: &SettingsFile) -> SettingsDocument {
    registered(file.load().unwrap())
}

#[test]
fn missing_file_loads_the_defaults() {
    let (_directory, file) = file();
    let document = load(&file);
    assert_eq!(document.get::<Terminal>(), &Terminal::default());
    assert!(document.errors().is_empty());
}

#[test]
fn only_changed_settings_are_written() {
    let (_directory, file) = file();
    let mut document = load(&file);
    document.update::<Terminal>(|terminal| {
        terminal.cursor_shape = "underline".into();
        terminal.font_size = Some(15.0);
    });

    file.save(&document).unwrap();

    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "[terminal]\ncursor_shape = \"underline\"\nfont_size = 15.0\n"
    );
    assert_eq!(load(&file), document);
}

#[test]
fn restoring_a_default_removes_it_from_the_file() {
    let (_directory, file) = file();
    let mut document = load(&file);
    document.update::<Terminal>(|terminal| terminal.copy_on_select = true);
    file.save(&document).unwrap();

    document.set(Terminal::default());
    file.save(&document).unwrap();

    assert_eq!(fs::read_to_string(file.path()).unwrap(), "");
}

#[test]
fn saving_keeps_the_users_comments() {
    let (_directory, file) = file();
    fs::write(
        file.path(),
        "[terminal]\n# Easier on the eyes.\nfont_size = 15.0\n",
    )
    .unwrap();
    let mut document = load(&file);
    document.update::<Terminal>(|terminal| terminal.font_size = Some(16.0));

    file.save(&document).unwrap();

    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "[terminal]\n# Easier on the eyes.\nfont_size = 16.0\n"
    );
}

#[test]
fn nested_tables_are_written_without_their_defaults() {
    let (_directory, file) = file();
    let mut document = load(&file);
    document.update::<Agents>(|ai| {
        ai.agents.insert(
            "hermes".into(),
            Agent {
                enabled: false,
                ..Agent::default()
            },
        );
        ai.agents.insert(
            "mine".into(),
            Agent {
                command: Some("/opt/mine".into()),
                ..Agent::default()
            },
        );
    });

    file.save(&document).unwrap();

    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "[ai.agents.hermes]\nenabled = false\n\n[ai.agents.mine]\ncommand = \"/opt/mine\"\n"
    );
    assert_eq!(load(&file), document);
}

#[test]
fn loading_sanitizes_out_of_range_values() {
    let (_directory, file) = file();
    fs::write(file.path(), "[terminal]\nfont_size = 1.0\n").unwrap();

    assert_eq!(load(&file).get::<Terminal>().font_size, Some(6.0));
}

#[test]
fn a_broken_section_falls_back_alone_and_survives_saving() {
    let (_directory, file) = file();
    let broken = "[monitor]\n# Mine.\ninterval_secs = 5\nintervall = 3\n";
    fs::write(
        file.path(),
        format!("[terminal]\nfont_size = 15.0\n\n{broken}\n[future]\nflag = true\n"),
    )
    .unwrap();

    let mut document = load(&file);
    assert_eq!(document.get::<Terminal>().font_size, Some(15.0));
    assert_eq!(document.get::<Monitor>(), &Monitor::default());
    let errors = document.errors();
    let keys: Vec<_> = errors.iter().map(|error| error.key.as_str()).collect();
    assert_eq!(keys, ["future", "monitor"]);
    assert!(errors[1].error.contains("intervall"), "{}", errors[1]);

    document.update::<Terminal>(|terminal| terminal.font_size = Some(16.0));
    file.save(&document).unwrap();
    let text = fs::read_to_string(file.path()).unwrap();
    assert!(text.contains("font_size = 16.0"), "{text}");
    assert!(text.contains(broken), "{text}");
    assert!(text.contains("[future]\nflag = true"), "{text}");
    assert_eq!(load(&file).errors().len(), 2);
}

#[test]
fn changing_a_broken_section_rewrites_it() {
    let (_directory, file) = file();
    fs::write(file.path(), "[terminal]\nfont_szie = 15.0\n").unwrap();
    let mut document = load(&file);
    assert_eq!(document.errors().len(), 1);

    document.update::<Terminal>(|terminal| terminal.copy_on_select = true);
    assert!(document.errors().is_empty());
    file.save(&document).unwrap();

    assert_eq!(
        fs::read_to_string(file.path()).unwrap(),
        "[terminal]\ncopy_on_select = true\n"
    );
    assert!(load(&file).errors().is_empty());
}

#[test]
fn a_table_changed_on_disk_that_nothing_claims_is_kept() {
    let (_directory, file) = file();
    fs::write(file.path(), "[future]\nflag = true\n").unwrap();
    let mut document = load(&file);
    fs::write(file.path(), "[future]\nflag = false\n").unwrap();

    document.update::<Monitor>(|monitor| monitor.interval_secs = 9);
    file.save(&document).unwrap();

    let text = fs::read_to_string(file.path()).unwrap();
    assert!(text.contains("flag = false"), "{text}");
    assert!(text.contains("interval_secs = 9"), "{text}");
}

#[test]
fn a_file_that_is_not_toml_is_an_error() {
    let (_directory, file) = file();
    fs::write(file.path(), "[terminal\n").unwrap();
    assert!(file.load().is_err());
}
