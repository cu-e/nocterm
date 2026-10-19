//! Real native rows/prompts keep the original Plain policy without common headers.
use super::*;
use crate::{
    history::physical::SnapshotBuilder,
    model::{
        content::MediaKind,
        json::deferred::DecodeLedger,
        native::{NativeList, NativeOwned},
        retention::{
            RetentionAuthority,
            document::{
                DocumentRetention,
                tests::{snapshot, text},
            },
        },
        tools::{ToolKind, ToolStatus},
    },
};
fn content(ledger: &DecodeLedger, value: &str) -> NativeOwned<Content> {
    crate::model::native::content::from_text(
        crate::model::native::content::text_content(
            text(ledger, value),
            NativeOwned::absent(ledger).unwrap(),
            NativeOwned::absent(ledger).unwrap(),
        )
        .unwrap(),
    )
}
#[test]
fn physical_rows_match_original_plain_components_without_source_metadata() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut rows = Vec::new();
    rows.push(crate::history::native::entries::agent(text(
        &ledger,
        "actual text with spare native capacity",
    )));
    let mut contents = NativeList::new(&ledger, 2).unwrap();
    contents.push(content(&ledger, "first user item")).unwrap();
    contents.push(content(&ledger, "second user item")).unwrap();
    rows.push(crate::history::native::entries::user(contents.finish()));
    let mut output = NativeList::new(&ledger, 1).unwrap();
    output
        .push(
            crate::model::native::tools::content_output(
                content(&ledger, "boxed content"),
                NativeOwned::absent(&ledger).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let mut locations = NativeList::new(&ledger, 1).unwrap();
    locations
        .push(
            crate::model::native::tools::location(
                crate::model::native::tools::path(text(&ledger, "/tmp/original-path-capacity"))
                    .unwrap(),
                Some(7),
                NativeOwned::absent(&ledger).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let header = crate::model::native::tools::header(
        crate::model::native::tools::tool_id(text(&ledger, "original-tool-id")).unwrap(),
        text(&ledger, "original title"),
        NativeOwned::supplied(text(&ledger, "literal name")),
        ToolKind::Execute,
        ToolStatus::Pending,
    )
    .unwrap();
    let tool = crate::model::native::tools::record(
        header,
        output.finish(),
        locations.finish(),
        NativeOwned::absent(&ledger).unwrap(),
        NativeOwned::absent(&ledger).unwrap(),
        NativeOwned::absent(&ledger).unwrap(),
        NativeOwned::absent(&ledger).unwrap(),
    )
    .unwrap();
    rows.push(crate::history::native::entries::tool_entry(tool));
    for native in rows {
        let original = crate::history::allocations::row(native.get()).unwrap();
        let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
        builder.append_row(native, 2).unwrap();
        live.replace(builder.finish(None).unwrap()).unwrap();
        let actual = live.snapshot().unwrap().rows().last().unwrap();
        assert_eq!(row(actual).unwrap(), original);
        assert_eq!(
            actual.cached_policy().unwrap(),
            original.max(crate::history::physical::wire::retained_row(actual).unwrap())
        );
    }
}
#[test]
fn physical_prompt_keeps_full_media_attachments_and_original_plain_formula() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let mut foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    live.replace(snapshot(&foreground, "seed")).unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let mut images = NativeList::new(&ledger, 1).unwrap();
    images
        .push(crate::model::native::content::from_media(
            crate::model::native::content::media(
                MediaKind::Image,
                text(&ledger, "full image"),
                text(&ledger, "image/png"),
                NativeOwned::absent(&ledger).unwrap(),
                NativeOwned::absent(&ledger).unwrap(),
                NativeOwned::absent(&ledger).unwrap(),
            )
            .unwrap(),
        ))
        .unwrap();
    let mut attachments = NativeList::new(&ledger, 1).unwrap();
    attachments
        .push(crate::history::native::document::connection(text(
            &ledger,
            "original context",
        )))
        .unwrap();
    let native = crate::history::native::document::prompt(
        17,
        text(&ledger, "original prompt"),
        images.finish(),
        attachments.finish(),
    )
    .unwrap();
    let original = crate::history::allocations::prompt(native.get()).unwrap();
    let mut queue = NativeList::new(&ledger, 1).unwrap();
    queue.push(native).unwrap();
    let composer = crate::history::native::document::composer(
        NativeOwned::absent(&ledger).unwrap(),
        queue.finish(),
        NativeList::new(&ledger, 0).unwrap().finish(),
    )
    .unwrap();
    let mut builder = SnapshotBuilder::from_role(&mut foreground, &live).unwrap();
    builder.replace_composer(composer).unwrap();
    live.replace(builder.finish(None).unwrap()).unwrap();
    assert_eq!(
        prompt(&live.snapshot().unwrap().composer().queue()[0]).unwrap(),
        original
    );
    let actual = &live.snapshot().unwrap().composer().queue()[0];
    assert_eq!(
        actual.cached_policy().unwrap(),
        original.max(crate::history::physical::wire::retained_prompt(actual).unwrap())
    );
}

fn original_resume(ledger: &DecodeLedger) -> NativeOwned<crate::history::ResumeDescriptor> {
    use crate::model::native::NativeChoices;
    let mut choices = NativeChoices::new(ledger, 2).unwrap();
    choices
        .push(
            text(ledger, "model"),
            crate::model::native::choices::text_value(text(ledger, "original-model")),
        )
        .unwrap();
    choices
        .push(
            text(ledger, "reasoning"),
            crate::model::native::choices::flag(true, ledger).unwrap(),
        )
        .unwrap();
    crate::history::native::document::resume(
        NativeOwned::supplied(text(ledger, "session-original-001")),
        NativeOwned::supplied(
            crate::model::native::tools::path(text(ledger, "/tmp/original-workdir")).unwrap(),
        ),
        choices.finish().unwrap(),
        NativeOwned::supplied(text(ledger, "mode-original")),
        false,
    )
    .unwrap()
}
fn original_composer(ledger: &DecodeLedger) -> NativeOwned<crate::history::ComposerData> {
    let mut queue = NativeList::new(ledger, 1).unwrap();
    queue
        .push(
            crate::history::native::document::prompt(
                17,
                text(ledger, "queued text"),
                NativeList::new(ledger, 0).unwrap().finish(),
                NativeList::new(ledger, 0).unwrap().finish(),
            )
            .unwrap(),
        )
        .unwrap();
    let mut attachments = NativeList::new(ledger, 1).unwrap();
    attachments
        .push(crate::history::native::document::group(text(
            ledger,
            "default context",
        )))
        .unwrap();
    crate::history::native::document::composer(
        NativeOwned::supplied(text(ledger, "original draft")),
        queue.finish(),
        attachments.finish(),
    )
    .unwrap()
}

#[test]
fn physical_shell_matches_original_document_after_component_subtraction() {
    let authority = RetentionAuthority::new(usize::MAX).unwrap();
    let (_, mut roles) = DocumentRetention::new(&authority).unwrap();
    let foreground = roles.take_foreground().unwrap();
    let mut live = roles.take_live().unwrap();
    let ledger =
        DecodeLedger::for_operation(&foreground, crate::history::MAX_RESIDENT_BYTES).unwrap();
    let resume = original_resume(&ledger);
    let metadata = crate::history::native::document::metadata(
        text(&ledger, "agent"),
        NativeOwned::supplied(text(&ledger, "original title")),
        NativeOwned::supplied(text(&ledger, "original name")),
        true,
        19,
        NativeOwned::supplied(text(&ledger, "original model")),
    )
    .unwrap();
    let mut rows = NativeList::new(&ledger, 1).unwrap();
    rows.push(crate::history::native::entries::agent(text(
        &ledger,
        "original row",
    )))
    .unwrap();
    let mut times = NativeList::new(&ledger, 5).unwrap();
    times
        .push(NativeOwned::scalar(7u64, &ledger).unwrap())
        .unwrap();
    let transcript =
        crate::history::native::document::transcript(rows.finish(), times.finish()).unwrap();
    let composer = original_composer(&ledger);
    let native = crate::history::native::NativeChat::new(
        text(&ledger, "f2f8f6ac-eae5-4dd7-b4bc-3713da3ed73c"),
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap()
    .into_document();
    let document = native.get();
    let mut original = crate::history::allocations::document(document).unwrap();
    for row in &document.transcript.entries {
        original -= crate::history::allocations::row(row).unwrap();
    }
    for prompt in &document.composer.queue {
        original -= crate::history::allocations::prompt(prompt).unwrap();
    }
    live.replace(
        SnapshotBuilder::from_document(&foreground, native)
            .unwrap()
            .finish(None)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(shell(live.snapshot().unwrap()).unwrap(), original);
}
