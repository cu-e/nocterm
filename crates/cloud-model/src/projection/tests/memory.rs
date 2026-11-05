//! Projection accounting measures retained content, independent of saved-history omission.
use super::*;
use nocterm_ai::model::content::{Content, Image};
#[test]
fn a_large_accepted_prompt_image_is_fully_charged_and_shared_by_later_pages() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let data = "a".repeat(1024 * 1024);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![prompt_content_event(
            &run,
            vec![Content::Image(Image::new(data.clone(), "image/png"))],
            CommandId::from_bytes([1; 16]),
        )]))
        .unwrap();
    assert!(projection.resident_bytes() >= data.len() * 2);
    let Entry::User(content) = &projection.transcript().entries[0] else {
        panic!("prompt");
    };
    let Content::Image(image) = &content[0] else {
        panic!("image");
    };
    let original = image.data.clone();
    projection
        .apply(&fixture.page(vec![text(&run, TextChannel::Assistant, "answer")]))
        .unwrap();
    let Entry::User(content) = &projection.transcript().entries[0] else {
        panic!("prompt");
    };
    let Content::Image(image) = &content[0] else {
        panic!("image");
    };
    assert_eq!(image.data.as_ptr(), original.as_ptr());
    assert!(projection.resident_bytes() >= data.len() * 2);
}
#[test]
fn changed_tool_and_streamed_rows_remain_inside_the_measured_projection() {
    let mut fixture = ProjectionFixture::new();
    let run = run_binding(1);
    let (intent, call, display) = tool(&run, 2);
    let mut projection = ConversationProjection::new(fixture.binding().clone());
    projection
        .apply(&fixture.page(vec![
            prompt(&run, 1),
            CoordinatorEvent::ToolIntent {
                intent: intent.clone(),
                call,
                display: display.clone(),
            },
            text(&run, TextChannel::Assistant, "prefix"),
        ]))
        .unwrap();
    let output = "output".repeat(4096);
    let observation = ToolObservation::failed(
        intent.id(),
        BoundedJson::new(json!({"output":output})).unwrap(),
        display,
        ToolEffectStatus::Unknown,
    );
    projection
        .apply(&fixture.page(vec![
            CoordinatorEvent::ToolResult {
                run: run.clone(),
                observation,
            },
            text(&run, TextChannel::Assistant, " suffix"),
        ]))
        .unwrap();
    let actual =
        nocterm_ai::history::retained_value_bytes(&projection.transcript().entries).unwrap();
    assert!(projection.resident_bytes() >= actual);
    assert!(
        matches!(projection.transcript().entries.last(),Some(Entry::Agent(value)) if value == "prefix suffix")
    );
}
