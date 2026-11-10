//! Manual CPU/layout timing through the real queue button and normal cached draws.
//! Run with `cargo test -p nocterm-agent --lib queue_opening_frame_trace -- --ignored --nocapture`.
//! Optional NOCTERM_QUEUE_PROBE_{BYTES,COUNT,IMAGE,WARM,REDUCE_MOTION} select the workload.
//! Set NOCTERM_QUEUE_PROBE_NATIVE_TEXT=1 for real system font shaping.
//! Logical 16 ms frame steps are not GPU presentation or wall-clock input latency.
use super::*;
use gpui_kit::px;
use std::time::{Duration, Instant};

fn option(name: &str, default: usize) -> usize {
    std::env::var(format!("NOCTERM_QUEUE_PROBE_{name}"))
        .ok()
        .map(|value| {
            value
                .parse()
                .expect("queue probe option must be an integer")
        })
        .unwrap_or(default)
}

fn advance_frame(f: &Fixture, cx: &mut TestAppContext, advance: bool) {
    if advance {
        cx.executor().advance_clock(Duration::from_millis(16));
    }
    cx.update_window(f.handle, |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn settle_motion(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..20 {
        advance_frame(f, cx, true);
        cx.update_window(f.handle, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
#[ignore = "manual queue opening timing probe"]
fn queue_opening_frame_trace(cx: &mut TestAppContext) {
    if option("NATIVE_TEXT", 0) == 0 {
        measure_queue(cx);
        return;
    }
    let platform = gpui_kit::platform::current_platform(true);
    let mut native = TestAppContext::build_with_text_system(
        gpui_kit::TestDispatcher::new(0),
        Some("queue_opening_native_text"),
        platform.text_system(),
    );
    measure_queue(&mut native);
}

#[expect(
    clippy::print_stderr,
    reason = "explicitly requested manual queue timing diagnostics"
)]
fn measure_queue(cx: &mut TestAppContext) {
    let (bytes, count) = (option("BYTES", 32), option("COUNT", 1));
    let (image, warm, reduced) = (
        option("IMAGE", 0) != 0,
        option("WARM", 0) != 0,
        option("REDUCE_MOTION", 0) != 0,
    );
    let codec = image_codec();
    let f = fixture_with_width(
        cx,
        18.,
        workspace_resolver(),
        crate::ImageDeps::new(codec.clone(), codec.clone()),
    );
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(reduced));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.clear_composer_context(cx);
        thread.send("active".into(), cx);
        if image {
            thread
                .accept_composer_images(vec![prompt_image()], cx)
                .unwrap();
        }
        let mut text = "queued message ".repeat(bytes.div_ceil(15));
        text.truncate(bytes);
        for _ in 0..count {
            thread.send(text.clone(), cx);
        }
    });
    super::queue_layout::settle(&f, cx);
    if warm {
        open_queue(&f, cx);
        settle_motion(&f, cx);
        cx.update_window(f.handle, |_, window, cx| window.click("queue-toggle", cx))
            .unwrap();
        settle_motion(&f, cx);
    }
    eprintln!(
        "QUEUE_WORKLOAD {}",
        serde_json::json!({
            "bytes": bytes, "count": count, "image": image, "warm": warm, "reduced": reduced,
        })
    );
    trace_open(&f, codec.as_ref(), cx);
}

#[expect(
    clippy::print_stderr,
    reason = "explicitly requested manual queue timing diagnostics"
)]
fn trace_open(
    f: &Fixture,
    codec: &nocterm_ai::testing::ScriptedImageCodec,
    cx: &mut TestAppContext,
) {
    cx.update_window(f.handle, |_, window, cx| window.click("queue-toggle", cx))
        .unwrap();
    cx.run_until_parked();
    for frame in 0..20 {
        advance_frame(f, cx, frame != 0);
        let started = Instant::now();
        let (height, visible, rows) = cx
            .update_window(f.handle, |_, window, cx| {
                window.draw(cx).clear(cx);
                let reveal = window.try_find("agent-queue-reveal");
                (
                    reveal
                        .as_ref()
                        .map(|node| node.bounds().size.height / px(1.)),
                    reveal.as_ref().is_some_and(|node| node.visible()),
                    window
                        .try_find("agent-queue-rows")
                        .map(|node| node.bounds().size.height / px(1.)),
                )
            })
            .unwrap();
        cx.run_until_parked();
        eprintln!(
            "QUEUE_FRAME {}",
            serde_json::json!({
                "ms": frame * 16, "frame_us": started.elapsed().as_micros(),
                "height": height, "visible": visible, "rows": rows,
            })
        );
    }
    let thumbnails = codec
        .requests()
        .iter()
        .filter(|request| matches!(request, nocterm_ai::testing::ImageRequest::Thumbnail(..)))
        .count();
    eprintln!("QUEUE_THUMBNAILS {thumbnails}");
}
