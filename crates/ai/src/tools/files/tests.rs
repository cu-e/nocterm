//! File tools: their bounds, and their work on a terminal's filesystem.
use std::sync::Arc;

use futures::executor::block_on;
use nocterm_session::{RemoteFs, testing::InMemoryFs};
use serde_json::{Value, json};

use super::*;
use crate::{TerminalCall, TerminalCallError, ToolCapability, ToolFailure, ToolGroup};

fn files() -> InMemoryFs {
    let fs = InMemoryFs::default();
    fs.seed_file("/server.properties", "motd=Hi\nmax-players=20\npvp=true\n")
        .unwrap();
    fs.seed_file("/plugins/a.yml", "a: 1\n").unwrap();
    fs
}

fn run_on(fs: &InMemoryFs, call: TerminalCall, redact: bool) -> Result<Value, ToolFailure> {
    let shared: Arc<dyn RemoteFs> = Arc::new(fs.clone());
    block_on(run(call, shared, redact))
}

fn text(fs: &InMemoryFs, path: &str) -> String {
    String::from_utf8(fs.bytes(path).unwrap()).unwrap()
}

fn edit(old: &str, new: &str, replace_all: bool) -> TerminalCall {
    TerminalCall::EditFile(EditFile {
        terminal_id: "t1".into(),
        path: "/server.properties".into(),
        old_text: old.into(),
        new_text: new.into(),
        replace_all,
    })
}

#[test]
fn paths_name_one_place_below_the_top() {
    for path in ["/", "/a", "/a/b.txt", "/with space/ü"] {
        assert!(path_ok(path), "{path}");
    }
    for path in ["", "a", "/a/", "//a", "/a/../b", "/./a", "/a\nb", "~/a"] {
        assert!(!path_ok(path), "{path:?}");
    }
    let delete = |path: &str| {
        TerminalCall::DeletePath(DeletePath {
            terminal_id: "t1".into(),
            path: path.into(),
        })
        .validate()
    };
    assert_eq!(delete("/"), Err(TerminalCallError::FilePathRejected));
    assert_eq!(delete("/logs"), Ok(()));
    let moved = TerminalCall::MovePath(MovePath {
        terminal_id: "t1".into(),
        from: "/world".into(),
        to: "/world/inside".into(),
    });
    assert_eq!(moved.validate(), Err(TerminalCallError::FilePathRejected));
    assert_eq!(
        edit("same", "same", false).validate(),
        Err(TerminalCallError::EditRejected)
    );
}

#[test]
fn reading_is_a_read_and_every_change_needs_the_files_capability() {
    let read = TerminalCall::ReadFile(ReadFile {
        terminal_id: "t1".into(),
        path: "/a".into(),
        start_line: None,
        max_lines: None,
    });
    assert!(!read.writes());
    assert_eq!(read.capability(), ToolCapability::Read);
    assert_eq!(read.group(), ToolGroup::Files);
    let change = edit("a", "b", false);
    assert!(change.writes() && change.is_file_tool());
    assert_eq!(change.capability(), ToolCapability::Files);
    assert_eq!(change.terminal_id(), Some("t1"));
}

#[test]
fn a_listing_shows_folders_first_with_the_sizes_of_files() {
    let listed = run_on(
        &files(),
        TerminalCall::ListFiles(ListFiles {
            terminal_id: "t1".into(),
            path: "/".into(),
        }),
        false,
    )
    .unwrap();
    assert_eq!(
        listed["entries"],
        json!([
            {"name": "plugins", "type": "directory"},
            {"name": "server.properties", "type": "file", "size": 32}
        ])
    );
    assert_eq!(listed["truncated"], json!(false));
}

#[test]
fn a_file_is_read_a_range_of_lines_at_a_time() {
    let fs = files();
    let read = |start, max| {
        run_on(
            &fs,
            TerminalCall::ReadFile(ReadFile {
                terminal_id: "t1".into(),
                path: "/server.properties".into(),
                start_line: start,
                max_lines: max,
            }),
            false,
        )
        .unwrap()
    };
    let first = read(None, Some(2));
    assert_eq!(first["content"], json!("motd=Hi\nmax-players=20\n"));
    assert_eq!(first["total_lines"], json!(3));
    assert_eq!(first["next_start_line"], json!(3));
    let rest = read(Some(3), None);
    assert_eq!(rest["content"], json!("pvp=true\n"));
    assert!(rest.get("next_start_line").is_none());
}

#[test]
fn an_edit_replaces_exact_text_once_or_says_why_not() {
    let fs = files();
    let answer = run_on(&fs, edit("max-players=20", "max-players=40", false), false).unwrap();
    assert_eq!(answer["replacements"], json!(1));
    assert_eq!(
        text(&fs, "/server.properties"),
        "motd=Hi\nmax-players=40\npvp=true\n"
    );
    let missing = run_on(&fs, edit("max-players=20", "x", false), false).unwrap_err();
    assert!(missing.message().contains("not found"), "{missing}");
    let twice = run_on(&fs, edit("=", ":", false), false).unwrap_err();
    assert!(twice.message().contains("occurs 3 times"), "{twice}");
    let all = run_on(&fs, edit("=", ":", true), false).unwrap();
    assert_eq!(all["replacements"], json!(3));
    assert_eq!(
        text(&fs, "/server.properties"),
        "motd:Hi\nmax-players:40\npvp:true\n"
    );
}

#[test]
fn a_write_creates_a_file_and_replaces_one_only_when_asked() {
    let fs = files();
    let write = |path: &str, overwrite| {
        TerminalCall::WriteFile(WriteFile {
            terminal_id: "t1".into(),
            path: path.into(),
            content: "[]\n".into(),
            overwrite,
        })
    };
    run_on(&fs, write("/ops.json", false), false).unwrap();
    assert_eq!(text(&fs, "/ops.json"), "[]\n");
    let refused = run_on(&fs, write("/server.properties", false), false).unwrap_err();
    assert!(refused.message().contains("set overwrite"), "{refused}");
    run_on(&fs, write("/server.properties", true), false).unwrap();
    assert_eq!(text(&fs, "/server.properties"), "[]\n");
}

#[test]
fn hidden_secrets_are_never_written_back_but_the_rest_of_the_file_can_change() {
    let fs = InMemoryFs::default();
    fs.seed_file("/config.yml", "name: lobby\npassword: hunter2\n")
        .unwrap();
    let read = run_on(
        &fs,
        TerminalCall::ReadFile(ReadFile {
            terminal_id: "t1".into(),
            path: "/config.yml".into(),
            start_line: None,
            max_lines: None,
        }),
        true,
    )
    .unwrap();
    assert_eq!(read["redacted"], json!(true));
    let shown = read["content"].as_str().unwrap();
    assert!(!shown.contains("hunter2"), "{shown}");
    let rewrite = TerminalCall::WriteFile(WriteFile {
        terminal_id: "t1".into(),
        path: "/config.yml".into(),
        content: shown.replace("lobby", "hub"),
        overwrite: true,
    });
    assert!(run_on(&fs, rewrite, true).is_err());
    let rename = TerminalCall::EditFile(EditFile {
        terminal_id: "t1".into(),
        path: "/config.yml".into(),
        old_text: "name: lobby".into(),
        new_text: "name: hub".into(),
        replace_all: false,
    });
    run_on(&fs, rename, true).unwrap();
    assert_eq!(text(&fs, "/config.yml"), "name: hub\npassword: hunter2\n");
}

#[test]
fn folders_are_made_moved_and_deleted() {
    let fs = files();
    let call = |call| run_on(&fs, call, false);
    call(TerminalCall::MakeDirectory(MakeDirectory {
        terminal_id: "t1".into(),
        path: "/backups".into(),
    }))
    .unwrap();
    call(TerminalCall::MovePath(MovePath {
        terminal_id: "t1".into(),
        from: "/plugins/a.yml".into(),
        to: "/backups/a.yml".into(),
    }))
    .unwrap();
    assert_eq!(text(&fs, "/backups/a.yml"), "a: 1\n");
    let deleted = call(TerminalCall::DeletePath(DeletePath {
        terminal_id: "t1".into(),
        path: "/plugins".into(),
    }))
    .unwrap();
    assert_eq!(deleted["deleted"], json!("directory"));
    let gone = call(TerminalCall::DeletePath(DeletePath {
        terminal_id: "t1".into(),
        path: "/plugins".into(),
    }))
    .unwrap_err();
    assert!(gone.message().contains("no such file"), "{gone}");
}

#[test]
fn a_binary_file_is_refused_with_the_way_to_copy_it() {
    let fs = InMemoryFs::default();
    fs.seed_file("/world/level.dat", vec![0x1f, 0x8b, 0xff, 0x00])
        .unwrap();
    let refused = run_on(
        &fs,
        TerminalCall::ReadFile(ReadFile {
            terminal_id: "t1".into(),
            path: "/world/level.dat".into(),
            start_line: None,
            max_lines: None,
        }),
        false,
    )
    .unwrap_err();
    assert!(refused.message().contains("transfer_files"), "{refused}");
}
