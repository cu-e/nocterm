use super::*;
use gpui_kit::px;

pub(super) fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

fn assert_first_open_frame(reduced: bool, cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(reduced));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.clear_composer_context(cx);
        thread.send("active".into(), cx);
        thread.send("short queued message".into(), cx);
    });
    settle(&f, cx);
    let viewport = cx
        .update_window(f.handle, |_, window, cx| {
            let height = window.find("agent-transcript-area").bounds().size.height;
            window.click("queue-toggle", cx);
            height
        })
        .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.simulate_next_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        // Preserve normal cached-view invalidation: render_frame() forces a refresh.
        window.draw(cx).clear(cx);
        let reveal = window.find("agent-queue-reveal");
        let rows = window.find("agent-queue-rows");
        assert!(
            reveal.visible(),
            "the first post-click frame must show the list"
        );
        assert!(rows.bounds().size.height > px(20.));
        assert!((reveal.bounds().size.height - rows.bounds().size.height).abs() <= px(1.));
        let remaining = window.find("agent-transcript-area").bounds().size.height;
        assert!((viewport - remaining - reveal.bounds().size.height).abs() <= px(1.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_queue_opens_at_full_height_in_its_first_frame(cx: &mut TestAppContext) {
    assert_first_open_frame(false, cx);
}

#[gpui_kit::test]
fn reduced_motion_opens_the_queue_without_a_measurement_frame(cx: &mut TestAppContext) {
    assert_first_open_frame(true, cx);
}

#[gpui_kit::test]
fn expanded_queue_fits_one_and_two_short_messages_without_blank_height(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.clear_composer_context(cx);
        thread.send("active".into(), cx);
        thread.send("short".into(), cx);
    });
    open_queue(&f, cx);
    settle(&f, cx);
    let one = cx
        .update_window(f.handle, |_, window, _| {
            let content = window.find("agent-queue-rows").bounds().size.height;
            let reveal = window.find("agent-queue-reveal").bounds().size.height;
            assert!(
                content > px(20.) && content < px(100.),
                "unexpected single-row content {content:?}"
            );
            assert!(
                (reveal - content).abs() <= px(1.),
                "blank queue height: {reveal:?} vs actual {content:?}"
            );
            content
        })
        .unwrap();
    thread.update(cx, |thread, cx| {
        thread.send("another short message".into(), cx)
    });
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        let content = window.find("agent-queue-rows").bounds().size.height;
        let reveal = window.find("agent-queue-reveal").bounds().size.height;
        assert!(
            content > one && content < px(150.),
            "unexpected two-row content {content:?}"
        );
        assert!((reveal - content).abs() <= px(1.));
    })
    .unwrap();
    // Removing an item also shrinks to the new intrinsic height.
    thread.update(cx, |thread, cx| {
        let ids = thread.queued_prompt_ids().skip(1).collect::<Vec<_>>();
        for id in ids {
            thread.remove_queued(id, None, cx);
        }
        cx.notify();
    });
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        assert!((window.find("agent-queue-reveal").bounds().size.height - one).abs() <= px(1.));
    })
    .unwrap();
}

#[gpui_kit::test]
fn wrapped_text_images_and_attachments_are_measured_then_capped(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread
            .accept_composer_images(vec![prompt_image()], cx)
            .unwrap();
        thread.replace_composer_context(vec![Attachment::Group("Production servers".into())], cx);
        thread.send(
            "A long queued instruction that must wrap on this narrow chat panel. ".repeat(80),
            cx,
        );
    });
    open_queue(&f, cx);
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, _| {
        let content = window.find("agent-queue-rows").bounds();
        let reveal = window.find("agent-queue-reveal").bounds();
        let scroll = window.find("agent-queue-scroll").bounds();
        assert!(
            content.size.height > px(240.),
            "wrapped content was not measured: {content:?}"
        );
        assert!((reveal.size.height - px(240.)).abs() <= px(1.));
        assert!((scroll.size.height - px(240.)).abs() <= px(1.));
        assert!(content.size.width <= reveal.size.width);
    })
    .unwrap();
}

#[gpui_kit::test]
fn collapsed_queue_skips_hidden_row_and_image_rendering(cx: &mut TestAppContext) {
    let codec = image_codec();
    let f = fixture_with_width(
        cx,
        32.,
        workspace_resolver(),
        crate::ImageDeps::new(codec.clone(), codec.clone()),
    );
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread
            .accept_composer_images(vec![prompt_image()], cx)
            .unwrap();
        thread.send("hidden queue".into(), cx);
    });
    settle(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("queue-toggle").is_some());
        assert!(window.try_find("agent-queue-rows").is_none());
        assert!(window.try_find("agent-queue-reveal").is_none());
    })
    .unwrap();
    assert!(
        !codec
            .requests()
            .iter()
            .any(|request| matches!(request, nocterm_ai::testing::ImageRequest::Thumbnail(..))),
        "collapsed queue must not prepare hidden thumbnails"
    );
    open_queue(&f, cx);
    settle(&f, cx);
    assert!(
        codec
            .requests()
            .iter()
            .any(|request| matches!(request, nocterm_ai::testing::ImageRequest::Thumbnail(..))),
        "opening queue must prepare its images"
    );
}

#[gpui_kit::test]
fn reopening_the_queue_keeps_its_thumbnail_and_needs_no_animation_ticks(cx: &mut TestAppContext) {
    let codec = image_codec();
    let f = fixture_with_width(
        cx,
        18.,
        workspace_resolver(),
        crate::ImageDeps::new(codec.clone(), codec.clone()),
    );
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        thread
            .accept_composer_images(vec![prompt_image()], cx)
            .unwrap();
        thread.send("image in the queue".into(), cx);
    });
    settle(&f, cx);
    for open in [true, false, true] {
        cx.update_window(f.handle, |_, window, cx| window.click("queue-toggle", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(f.handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            if open {
                let reveal = window.find("agent-queue-reveal");
                let height = window.find("agent-queue-rows").bounds().size.height;
                assert!(reveal.visible());
                assert!((reveal.bounds().size.height - height.min(px(240.))).abs() <= px(1.));
            } else {
                assert!(window.try_find("agent-queue-rows").is_none());
            }
        })
        .unwrap();
        cx.run_until_parked();
    }
    let prepared = codec
        .requests()
        .iter()
        .filter(|request| matches!(request, nocterm_ai::testing::ImageRequest::Thumbnail(..)))
        .count();
    assert_eq!(
        prepared, 1,
        "reopening must reuse the existing image preview"
    );
}
