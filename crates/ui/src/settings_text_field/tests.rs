//! Native input and public receipts retain original settings, literal drafts and admitted work.
use super::SettingsTextField;
use crate::{DesignTokens, TerminalSettings};
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Context, Entity, Render, Subscription, TestAppContext,
    TestSupportExt as _, Window, WindowOptions, component::v_flex, div, prelude::*,
    test::TestWindowExt as _,
};
use nocterm_settings::SettingsDocument;
use nocterm_store::{
    SettingsConfig, SettingsHandle, StoreError,
    test_support::{WriteLog, WriteRelease, gated_writer},
};

struct Form {
    field: SettingsTextField,
    _notification: Subscription,
}
impl Render for Form {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .child(self.field.input())
            .when_some(self.field.error(cx), |this, error| {
                this.child(
                    div()
                        .id("field-error")
                        .aria_label(error.clone())
                        .test_support()
                        .child(error),
                )
            })
    }
}

struct Fixture {
    window: AnyWindowHandle,
    view: Entity<Form>,
    field: SettingsTextField,
    settings: SettingsHandle,
    release: WriteRelease,
    writes: WriteLog<SettingsDocument>,
}

fn fixture(initial: Option<&str>, cx: &mut TestAppContext) -> Fixture {
    let (writer, release, writes) = gated_writer().into_parts();
    let (window, view, field, settings) = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = SettingsConfig::with_writer(Default::default(), writer)
            .with(TerminalSettings {
                font_family: initial.map(str::to_owned),
                ..Default::default()
            })
            .with(crate::AppearanceSettings::default())
            .install(cx);
        crate::init(DesignTokens::builtin(), settings.clone(), cx);
        let mut field = None;
        let supplied = settings.clone();
        let (window, view) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let input = SettingsTextField::new(
                supplied,
                "font",
                |document| {
                    document
                        .get::<TerminalSettings>()
                        .font_family
                        .clone()
                        .unwrap_or_default()
                },
                |document, text| {
                    document.update::<TerminalSettings>(|section| {
                        section.font_family = Some(text.to_owned())
                    });
                    Ok(())
                },
                window,
                cx,
            );
            field = Some(input.clone());
            cx.new(|cx| {
                let weak = cx.weak_entity();
                let original_window = window.window_handle();
                let notification = input.observe(cx, move |cx| {
                    let _ = original_window.update(cx, |_, _, cx| {
                        let _ = weak.update(cx, |_, cx| cx.notify());
                    });
                });
                Form {
                    field: input,
                    _notification: notification,
                }
            })
        })
        .unwrap();
        (window, view, field.unwrap(), settings)
    });
    cx.update_window(window, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    Fixture {
        window,
        view,
        field,
        settings,
        release,
        writes,
    }
}

fn draft(window: AnyWindowHandle, text: &str, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.click("font", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(text, cx);
        assert_eq!(window.find("font").value(), Some(text));
    })
    .unwrap();
    cx.run_until_parked();
}
fn enter(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.run_until_parked();
}
fn literal(window: AnyWindowHandle, expected: &str, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("font").value(), Some(expected));
    })
    .unwrap();
}
fn saved(settings: &SettingsHandle, cx: &mut TestAppContext) -> Option<String> {
    cx.update(|cx| settings.setting::<TerminalSettings>(cx).font_family.clone())
}
fn written(writes: &WriteLog<SettingsDocument>) -> Vec<Option<String>> {
    writes
        .lock()
        .unwrap()
        .iter()
        .map(|document| document.get::<TerminalSettings>().font_family.clone())
        .collect()
}
fn no_error(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("field-error").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn older_success_keeps_newer_draft_and_original_settings_after_another_installation(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    let other_fixture = fixture(None, cx);
    let other = other_fixture.settings.clone();
    draft(f.window, "second", cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    literal(f.window, "second", cx);
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("first"));
    assert_eq!(saved(&other, cx), None);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1200));
    cx.run_until_parked();
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("second"));
    assert_eq!(saved(&other, cx), None);
    assert_eq!(
        written(&f.writes),
        vec![Some("first".into()), Some("second".into())]
    );
}

#[gpui_kit::test]
fn older_failure_keeps_newer_native_draft_without_showing_a_stale_error(cx: &mut TestAppContext) {
    let f = fixture(None, cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    draft(f.window, "second", cx);
    f.release
        .send(Err(StoreError::backend(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ))))
        .unwrap();
    cx.run_until_parked();
    literal(f.window, "second", cx);
    no_error(f.window, cx);
    assert_eq!(saved(&f.settings, cx), None);
    enter(f.window, cx);
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("second"));
}

#[gpui_kit::test]
fn returning_to_confirmed_text_admits_rollback_behind_an_accepted_write(cx: &mut TestAppContext) {
    let f = fixture(Some("original"), cx);
    draft(f.window, "changed", cx);
    enter(f.window, cx);
    draft(f.window, "original", cx);
    enter(f.window, cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        written(&f.writes),
        vec![Some("changed".into()), Some("original".into())]
    );
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("original"));
    literal(f.window, "original", cx);
    no_error(f.window, cx);
}

#[gpui_kit::test]
fn rollback_after_failed_predecessor_is_an_ordered_semantic_noop(cx: &mut TestAppContext) {
    let f = fixture(Some("original"), cx);
    draft(f.window, "changed", cx);
    enter(f.window, cx);
    draft(f.window, "original", cx);
    enter(f.window, cx);
    f.release
        .send(Err(StoreError::backend(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied,
        ))))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(written(&f.writes), vec![Some("changed".into())]);
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("original"));
    cx.update(|cx| assert_eq!(f.settings.revision(cx), 0));
    no_error(f.window, cx);
}

#[gpui_kit::test]
async fn explicit_close_flush_is_admitted_before_capture_and_survives_view_loss(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    draft(f.window, "last", cx);
    cx.update_window(f.window, |_, window, cx| {
        f.field.flush_for_close(window, cx)
    })
    .unwrap();
    let work = cx.update(|cx| f.settings.capture_shutdown(cx));
    let weak = f.view.downgrade();
    cx.update_window(f.window, |_, window, _| window.remove_window())
        .unwrap();
    drop(f.view);
    drop(f.field);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
    let final_task = cx.background_executor.spawn(work);
    f.release.send(Ok(())).unwrap();
    final_task.await.unwrap();
    assert_eq!(
        written(&f.writes),
        vec![Some("first".into()), Some("last".into())]
    );
}

#[gpui_kit::test]
fn deactivation_admits_newer_literal_while_the_prior_receipt_is_pending(cx: &mut TestAppContext) {
    let f = fixture(None, cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    draft(f.window, "last", cx);
    cx.update_window(f.window, |_, window, cx| f.field.deactivate(window, cx))
        .unwrap();
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(
        written(&f.writes),
        vec![Some("first".into()), Some("last".into())]
    );
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("last"));
}

#[gpui_kit::test]
fn full_admission_error_keeps_the_literal_and_same_enter_retries_after_capacity_returns(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    draft(f.window, "0", cx);
    enter(f.window, cx);
    for index in 1..=nocterm_store::SETTINGS_CAPACITY {
        draft(f.window, &index.to_string(), cx);
        enter(f.window, cx);
    }
    draft(f.window, "overflow", cx);
    enter(f.window, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("field-error")
                .label()
                .unwrap()
                .contains("queue is full")
        );
    })
    .unwrap();
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    literal(f.window, "overflow", cx);
    assert_eq!(
        f.writes.lock().unwrap().len(),
        nocterm_store::SETTINGS_CAPACITY + 1
    );
    cx.update_window(f.window, |_, window, cx| window.press("enter", cx))
        .unwrap();
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("field-error")
                .label()
                .unwrap()
                .contains("queue is full"),
            "admission must not clear the old diagnostic before its matching receipt"
        );
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        f.writes.lock().unwrap().len(),
        nocterm_store::SETTINGS_CAPACITY + 2
    );
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("overflow"));
    no_error(f.window, cx);
}

#[gpui_kit::test]
fn canonical_publication_updates_native_text_without_a_feedback_write(cx: &mut TestAppContext) {
    let f = fixture(None, cx);
    draft(f.window, "  Mono  ", cx);
    enter(f.window, cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    literal(f.window, "Mono", cx);
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("Mono"));
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1200));
    cx.run_until_parked();
    enter(f.window, cx);
    assert_eq!(f.writes.lock().unwrap().len(), 1);
    no_error(f.window, cx);
}

#[gpui_kit::test]
async fn restoring_a_previously_saved_literal_after_an_external_change_is_a_new_intention(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    let external = cx.update(|cx| {
        f.settings
            .update::<TerminalSettings>(cx, |section| section.font_family = Some("external".into()))
    });
    cx.run_until_parked();
    external.await.unwrap();
    literal(f.window, "external", cx);
    draft(f.window, "first", cx);
    enter(f.window, cx);
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("first"));
    assert_eq!(
        written(&f.writes),
        vec![
            Some("first".into()),
            Some("external".into()),
            Some("first".into())
        ]
    );
    no_error(f.window, cx);
}

#[gpui_kit::test]
async fn closing_admission_keeps_the_new_literal_and_diagnostic_outside_the_captured_snapshot(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    draft(f.window, "accepted", cx);
    enter(f.window, cx);
    let work = cx.update(|cx| f.settings.capture_shutdown(cx));
    draft(f.window, "too late", cx);
    enter(f.window, cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("field-error")
                .label()
                .unwrap()
                .contains("persistence is closing")
        );
    })
    .unwrap();
    literal(f.window, "too late", cx);
    let task = cx.background_executor.spawn(work);
    f.release.send(Ok(())).unwrap();
    task.await.unwrap();
    assert_eq!(written(&f.writes), vec![Some("accepted".into())]);
    literal(f.window, "too late", cx);
    cx.update_window(f.window, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window
                .find("field-error")
                .label()
                .unwrap()
                .contains("persistence is closing")
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn publication_observer_can_restore_the_same_literal_before_receipt_presentation(
    cx: &mut TestAppContext,
) {
    let f = fixture(None, cx);
    let stage = std::rc::Rc::new(std::cell::Cell::new(0));
    let old_literal_visible = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = f.settings.clone();
    let window = f.window;
    let phase = stage.clone();
    let lag = old_literal_visible.clone();
    let _observer = cx.update(|cx| {
        f.settings.observe(cx, move |cx| {
            let current = observed
                .setting::<TerminalSettings>(cx)
                .font_family
                .as_deref();
            if phase.get() == 0 && current == Some("first") {
                phase.set(1);
                observed
                    .update::<TerminalSettings>(cx, |section| {
                        section.font_family = Some("external".into())
                    })
                    .detach();
            } else if phase.get() == 1 && current == Some("external") {
                phase.set(2);
                let lag = lag.clone();
                window
                    .update(cx, |_, window, cx| {
                        window.defer(cx, move |window, cx| {
                            window.render_frame(cx);
                            let still_original = window.find("font").value() == Some("first");
                            lag.set(still_original);
                            if !still_original {
                                window.click("font", cx);
                                window.press("ctrl-a", cx);
                                window.press("backspace", cx);
                                window.input("first", cx);
                            }
                            window.press("enter", cx);
                        });
                    })
                    .unwrap();
            }
        })
    });
    draft(f.window, "first", cx);
    enter(f.window, cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(stage.get(), 2);
    assert_eq!(
        saved(&f.settings, cx).as_deref(),
        Some("first"),
        "original literal remained visible after later publication: {}",
        old_literal_visible.get()
    );
    assert_eq!(
        written(&f.writes),
        vec![
            Some("first".into()),
            Some("external".into()),
            Some("first".into())
        ]
    );
    literal(f.window, "first", cx);
    no_error(f.window, cx);
}

#[gpui_kit::test]
fn a_retyped_same_literal_survives_old_receipt_until_its_debounce(cx: &mut TestAppContext) {
    let f = fixture(None, cx);
    let stage = std::rc::Rc::new(std::cell::Cell::new(0));
    let old_literal_visible = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = f.settings.clone();
    let window = f.window;
    let phase = stage.clone();
    let lag = old_literal_visible.clone();
    let _observer = cx.update(|cx| {
        f.settings.observe(cx, move |cx| {
            let current = observed
                .setting::<TerminalSettings>(cx)
                .font_family
                .as_deref();
            if phase.get() == 0 && current == Some("first") {
                phase.set(1);
                observed
                    .update::<TerminalSettings>(cx, |section| {
                        section.font_family = Some("external".into())
                    })
                    .detach();
            } else if phase.get() == 1 && current == Some("external") {
                phase.set(2);
                let lag = lag.clone();
                window
                    .update(cx, |_, window, cx| {
                        window.defer(cx, move |window, cx| {
                            window.render_frame(cx);
                            let still_original = window.find("font").value() == Some("first");
                            lag.set(still_original);
                            window.click("font", cx);
                            window.press("ctrl-a", cx);
                            window.press("backspace", cx);
                            window.input("first", cx);
                        });
                    })
                    .unwrap();
            }
        })
    });
    draft(f.window, "first", cx);
    enter(f.window, cx);
    f.release.send(Ok(())).unwrap();
    cx.run_until_parked();
    assert_eq!(stage.get(), 2);
    assert!(
        old_literal_visible.get(),
        "schedule must run before old receipt presentation"
    );
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("external"));
    assert_eq!(
        written(&f.writes),
        vec![Some("first".into()), Some("external".into())]
    );
    literal(f.window, "first", cx);
    no_error(f.window, cx);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(saved(&f.settings, cx).as_deref(), Some("first"));
    assert_eq!(
        written(&f.writes),
        vec![
            Some("first".into()),
            Some("external".into()),
            Some("first".into())
        ]
    );
    literal(f.window, "first", cx);
    no_error(f.window, cx);
}
