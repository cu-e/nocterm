use super::*;
fn snippet() -> Snippet {
    Snippet {
        name: "Deploy 日本語".into(),
        description: "Описание".into(),
        content: "#!/bin/bash\necho 'привет'\n\n".into(),
        ..Default::default()
    }
}
#[gpui_kit::test]
async fn persistence_round_trip_and_queued_updates(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deep/snippets.toml");
    let model = cx.new(|_| Snippets::new(Some(path.clone())));
    let first = snippet();
    let second = Snippet {
        name: "Second".into(),
        ..snippet()
    };
    let save1 = model.update(cx, |model, cx| model.save(first.clone(), None, cx));
    let save2 = model.update(cx, |model, cx| model.save(second.clone(), None, cx));
    save1.await.unwrap();
    save2.await.unwrap();
    let restored = Snippets::new(Some(path));
    assert_eq!(restored.library.snippets, [first, second]);
}
#[gpui_kit::test]
async fn corrupt_file_is_read_only_and_never_overwritten(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("snippets.toml");
    std::fs::write(&path, "snippets = =").unwrap();
    let model = cx.new(|_| Snippets::new(Some(path.clone())));
    assert!(!model.read_with(cx, |model, _| model.writable()));
    let task = model.update(cx, |model, cx| model.save(snippet(), None, cx));
    assert!(task.await.is_err());
    assert_eq!(std::fs::read_to_string(path).unwrap(), "snippets = =");
}
#[gpui_kit::test]
async fn failed_write_keeps_published_state(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let parent = directory.path().join("blocked");
    let model = cx.new(|_| Snippets::new(Some(parent.join("snippets.toml"))));
    std::fs::write(&parent, "file").unwrap();
    let task = model.update(cx, |model, cx| model.save(snippet(), None, cx));
    assert!(task.await.is_err());
    assert!(model.read_with(cx, |model, _| model.library.snippets.is_empty()));
}
#[gpui_kit::test]
async fn concurrent_stale_edits_rebase_on_persisted_state(cx: &mut gpui_kit::TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let model = cx.new(|_| Snippets::new(Some(directory.path().join("snippets.toml"))));
    let original = snippet();
    model
        .update(cx, |model, cx| model.save(original.clone(), None, cx))
        .await
        .unwrap();
    let first = Snippet {
        content: "first".into(),
        ..original.clone()
    };
    let second = Snippet {
        content: "second".into(),
        ..original.clone()
    };
    let one = model.update(cx, |model, cx| {
        model.save(first.clone(), Some(original.clone()), cx)
    });
    let two = model.update(cx, |model, cx| model.save(second, Some(original), cx));
    one.await.unwrap();
    assert!(two.await.is_err());
    assert_eq!(
        model.read_with(cx, |model, _| model.library.snippets.clone()),
        [first]
    );
}
