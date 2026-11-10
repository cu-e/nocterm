//! Phase and byte observations never pretend to be overall pull completion.
use gpui_kit::{
    IntoElement,
    base::TestSupportExt as _,
    component::{ActiveTheme as _, Sizable as _, progress::Progress, v_flex},
    div,
    prelude::*,
};
use nocterm_project_providers::SetupUpdate;

pub(crate) fn render(
    id: &'static str,
    update: Option<&SetupUpdate>,
    pending: bool,
    cx: &gpui_kit::App,
) -> impl IntoElement + use<> {
    let phase = update.map_or("Preparing workspace", SetupUpdate::label);
    let transfer = update.and_then(|update| update.transfer.as_ref());
    let percent = transfer.and_then(|transfer| {
        transfer
            .total_bytes
            .map(|total| transfer.current_bytes as f32 / total as f32 * 100.)
    });
    let caption = transfer.map_or_else(
        || format!("{phase}…"),
        |transfer| {
            let subject = transfer.subject.as_deref().unwrap_or("Download");
            match transfer.total_bytes {
                Some(total) => format!(
                    "{subject}: {} / {} bytes ({:.0}%)",
                    transfer.current_bytes,
                    total,
                    percent.unwrap_or(0.)
                ),
                None => format!("{subject}: {} bytes downloaded", transfer.current_bytes),
            }
        },
    );
    v_flex()
        .id(id)
        .test_support()
        .aria_label(caption.clone())
        .flex_1()
        .min_w_0()
        .gap_1()
        .when(pending, |view| {
            view.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .truncate()
                    .child(caption.clone()),
            )
            .child(
                Progress::new(gpui_kit::SharedString::from(format!("{id}-bar")))
                    .xsmall()
                    .accessibility_label(caption)
                    .value(percent.unwrap_or(0.))
                    .loading(percent.is_none()),
            )
        })
}
