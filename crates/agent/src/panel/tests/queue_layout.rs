use super::*;
use gpui_kit::px;

fn settle(f: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn expanded_queue_fits_one_and_two_short_messages_without_blank_height(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.attachments.clear();
        thread.send("active".into(), cx);
        thread.send("short".into(), cx);
    });
    f.panel.update(cx, |panel, cx| {
        panel.queue_open = true;
        cx.notify();
    });
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
        thread.queue.truncate(1);
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
    let f = fixture_with_width(cx, 18.);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::new(2, 2)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        thread.images = vec![nocterm_ai::images::PromptImage::validate(png.into_inner()).unwrap()];
        thread.attachments = vec![Attachment::Group("Production servers".into())];
        thread.send(
            "A long queued instruction that must wrap on this narrow chat panel. ".repeat(80),
            cx,
        );
    });
    f.panel.update(cx, |panel, cx| {
        panel.queue_open = true;
        cx.notify();
    });
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
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    thread.update(cx, |thread, cx| {
        thread.send("active".into(), cx);
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
}
