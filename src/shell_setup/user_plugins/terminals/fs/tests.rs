//! A plugin terminal's files: what the plugin is asked and how its answers become a file system.

use std::sync::{Arc, Mutex};

use futures::{StreamExt as _, channel::mpsc, executor::block_on, future::join};
use nocterm_plugin_api::{
    base64,
    capability::files::{FileAnswer, FileEntry, FileFailure, FileOp, MAX_FILE_BYTES},
};
use nocterm_session::{EntryKind, FsError, RemoteFs, fs::UploadMode};

use super::{Asked, PluginFs};

/// A plugin that answers each request with the next of `answers`, and the requests it was asked.
fn plugin(
    answers: Vec<FileAnswer>,
) -> (
    PluginFs,
    impl std::future::Future<Output = ()>,
    Arc<Mutex<Vec<FileOp>>>,
) {
    let (asks, mut asked) = mpsc::channel::<Asked>(8);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let serve = async move {
        let mut answers = answers.into_iter();
        while let Some(request) = asked.next().await {
            let (op, reply) = request.into_parts();
            log.lock().unwrap().push(op);
            match answers.next() {
                Some(answer) => {
                    let _ = reply.send(answer);
                }
                None => break,
            }
        }
    };
    (PluginFs::new(asks), serve, seen)
}

fn file(name: &str, directory: bool) -> FileEntry {
    FileEntry {
        name: name.into(),
        directory,
        size: (!directory).then_some(12),
        modified: Some(1_700_000_000),
    }
}

#[test]
fn a_listing_becomes_entries_and_a_failure_its_error() {
    let (fs, serve, seen) = plugin(vec![
        FileAnswer::Listed(vec![
            file("plugins", true),
            file("server.properties", false),
        ]),
        FileAnswer::Failed(FileFailure::NotFound),
    ]);
    let (listed, missing) = block_on(join(
        async {
            let listed = fs.read_dir("/").await;
            let missing = fs.read_dir("/gone").await;
            drop(fs);
            (listed, missing)
        },
        serve,
    ))
    .0;
    let listed = listed.unwrap();
    assert_eq!(listed[0].kind, EntryKind::Directory);
    assert_eq!(
        (listed[1].name.as_str(), listed[1].size),
        ("server.properties", Some(12))
    );
    assert!(matches!(missing, Err(FsError::NotFound { path }) if path == "/gone"));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [
            FileOp::List { path: "/".into() },
            FileOp::List {
                path: "/gone".into()
            }
        ]
    );
}

#[test]
fn a_path_with_parent_steps_is_refused_without_asking_the_plugin() {
    let (fs, serve, seen) = plugin(vec![FileAnswer::Done]);
    let refused = block_on(join(
        async {
            let refused = fs.remove_file("/a/../etc/passwd").await;
            drop(fs);
            refused
        },
        serve,
    ))
    .0;
    assert!(matches!(refused, Err(FsError::Other(_))));
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn a_download_hands_out_the_bytes_in_the_chunks_asked() {
    let bytes: Vec<u8> = (0..=255).collect();
    let (fs, serve, _) = plugin(vec![FileAnswer::Content(base64::encode(&bytes))]);
    let chunks = block_on(join(
        async {
            let mut download = fs.download("/world/level.dat").await.unwrap();
            let mut chunks = Vec::new();
            loop {
                let chunk = download.read(100).await.unwrap();
                if chunk.is_empty() {
                    break;
                }
                chunks.push(chunk);
            }
            drop(fs);
            chunks
        },
        serve,
    ))
    .0;
    assert_eq!(
        chunks.iter().map(Vec::len).collect::<Vec<_>>(),
        [100, 100, 56]
    );
    assert_eq!(chunks.concat(), bytes, "binary bytes come back whole");
}

#[test]
fn an_upload_is_written_whole_when_finished_and_a_new_name_lands_beside_it() {
    let (fs, serve, seen) = plugin(vec![FileAnswer::Done, FileAnswer::Done]);
    let published = block_on(join(
        async {
            let mut upload = fs
                .upload("/plugins/a.yml", UploadMode::Create)
                .await
                .unwrap();
            upload.write(b"motd=".to_vec()).await.unwrap();
            upload.write(vec![0xff, 0x00]).await.unwrap();
            let first = upload.finish().await.unwrap();
            let mut renamed = fs
                .upload("/plugins/a.yml", UploadMode::Replace)
                .await
                .unwrap();
            renamed.write(b"x".to_vec()).await.unwrap();
            let second = renamed.publish_as("b.yml".into()).await.unwrap();
            drop(fs);
            (first, second)
        },
        serve,
    ))
    .0;
    assert_eq!(
        published,
        ("/plugins/a.yml".into(), "/plugins/b.yml".into())
    );
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen[0],
        FileOp::Write {
            path: "/plugins/a.yml".into(),
            data: base64::encode(b"motd=\xff\x00"),
            create: true,
        }
    );
    assert!(
        matches!(&seen[1], FileOp::Write { path, create: false, .. } if path == "/plugins/b.yml")
    );
}

#[test]
fn an_upload_larger_than_a_plugin_file_is_refused_before_the_plugin_is_asked() {
    let (fs, serve, seen) = plugin(vec![FileAnswer::Done]);
    let refused = block_on(join(
        async {
            let mut upload = fs.upload("/big.bin", UploadMode::Create).await.unwrap();
            let refused = upload.write(vec![0; MAX_FILE_BYTES + 1]).await;
            drop(upload);
            drop(fs);
            refused
        },
        serve,
    ))
    .0;
    assert!(refused.is_err());
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn creating_a_directory_that_is_there_already_is_accepted() {
    let (fs, serve, _) = plugin(vec![
        FileAnswer::Failed(FileFailure::AlreadyExists),
        FileAnswer::Found(Some(file("plugins", true))),
        FileAnswer::Failed(FileFailure::AlreadyExists),
        FileAnswer::Found(Some(file("plugins", false))),
    ]);
    let (directory, plain_file) = block_on(join(
        async {
            let directory = fs.create_dir("/plugins").await;
            let plain_file = fs.create_dir("/plugins").await;
            drop(fs);
            (directory, plain_file)
        },
        serve,
    ))
    .0;
    assert!(directory.is_ok());
    assert!(matches!(plain_file, Err(FsError::AlreadyExists { .. })));
}

#[test]
fn a_plugin_that_never_answers_fails_the_request() {
    let (fs, serve, _) = plugin(Vec::new());
    let failed = block_on(join(
        async {
            let failed = fs.stat("/").await;
            drop(fs);
            failed
        },
        serve,
    ))
    .0;
    assert!(matches!(failed, Err(FsError::Other(_))));
}
