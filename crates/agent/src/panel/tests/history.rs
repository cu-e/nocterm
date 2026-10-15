use super::*;
use crate::thread::AgentThread;

#[gpui_kit::test]
fn chats_are_saved_restored_into_a_new_panel_and_resume_their_session(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    new_chat(&f, cx);
    let session = cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
        thread.read(cx).session().clone().unwrap()
    });
    cx.run_until_parked();
    f.events
        .try_send(AgentEvent::Session(acp::SessionNotification::new(
            session.clone(),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(acp::ContentBlock::Text(
                acp::TextContent::new("answer"),
            ))),
        )))
        .unwrap();
    cx.run_until_parked();
    let finish = f.commands.pending.lock().unwrap().take().unwrap();
    finish
        .send(acp::PromptResponse::new(acp::StopReason::EndTurn))
        .unwrap();
    cx.run_until_parked();

    let saved = f.repository.chats();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].session_id.as_deref(), Some(&*session.0));
    assert!(matches!(
        saved[0].entries.as_slice(),
        [nocterm_ai::thread::Entry::User(_), nocterm_ai::thread::Entry::Agent(answer)] if answer == "answer"
    ));
    // The terminal context sent with the prompt is not part of the chat.
    let text = String::from_utf8(f.repository.encoded(&saved[0].id).unwrap()).unwrap();
    assert!(
        !text.contains(nocterm_ai::context::TERMINAL_RULES),
        "{text}"
    );

    // As after a restart: the previous process has closed before another document resumes it.
    f.panel.update(cx, |panel, cx| {
        panel
            .current()
            .unwrap()
            .update(cx, |thread, cx| thread.release_resources(cx));
    });
    cx.run_until_parked();
    // A new panel shows the saved chat, not yet connected.
    reinstall_history(&mut f, saved.clone(), cx);
    let panel = f.panel.clone();
    cx.run_until_parked();
    let thread = cx.update(|cx| {
        let panel = panel.read(cx);
        assert_eq!(panel.chat_documents().len(), 1);
        panel.chat_documents().next().unwrap().clone()
    });
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.dormant());
        assert_eq!(thread.transcript().entries.len(), 2);
        assert!(thread.session().is_none());
    });
    assert!(f.commands.restores.lock().unwrap().is_empty());

    // Opening it reopens the agent's session instead of starting another.
    let sessions = f.commands.sessions.load(Ordering::SeqCst);
    cx.update(|cx| activate_thread(&thread, &f.installation.installed, cx));
    cx.run_until_parked();
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(session.clone(), false)]
    );
    assert_eq!(f.commands.sessions.load(Ordering::SeqCst), sessions);
    cx.update(|cx| assert_eq!(thread.read(cx).session().as_ref(), Some(&session)));

    let id = cx.update(|cx| thread.read(cx).document().chat_id().to_owned());
    cx.update(|cx| f.installation.installed.runtime().delete_chat(id, cx));
    cx.run_until_parked();
    assert!(f.repository.chats().is_empty());
}
/// Sends `text` in the open chat and lets the agent answer `answer`.
#[gpui_kit::test]
fn empty_chats_are_dropped_when_another_starts_and_restart_keeps_the_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    new_chat(&f, cx);
    cx.update(|cx| assert_eq!(f.panel.read(cx).chat_documents().len(), 1));
    exchange(&f, "check the disk", "Use df -h.", cx);
    new_chat(&f, cx);
    cx.update(|cx| assert_eq!(f.panel.read(cx).chat_documents().len(), 2));

    // A chat whose agent exited is restarted in place, as the same chat.
    let (old, chat) = cx.update(|cx| {
        let panel = f.panel.read(cx);
        let old = panel.chat_documents().next().unwrap().clone();
        let chat = old.read(cx).document().chat_id().to_owned();
        (old, chat)
    });
    let id = old.entity_id();
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.open_thread(id, cx);
            panel
                .current()
                .unwrap()
                .update(cx, |thread, cx| thread.fail("Agent exited", cx));
        });
        window.render_frame(cx);
        window.click("agent-restart", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        // The empty chat was dropped too: nothing was said in it.
        assert_eq!(panel.chat_documents().len(), 1);
        let thread = panel.current().unwrap().read(cx);
        assert_ne!(panel.current().unwrap().entity_id(), id);
        assert_eq!(thread.document().chat_id().to_owned(), chat);
        assert_eq!(thread.title(), "check the disk");
        assert!(!thread.ended());
    });
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn history_searches_pins_renames_and_forks_chats(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "check disk usage", "Use df -h.", cx);
    let disk = cx.update(|cx| f.panel.read(cx).current().unwrap());
    new_chat(&f, cx);
    let nginx_session = exchange(&f, "restart nginx", "Done.", cx);
    let nginx = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let row = |thread: &Entity<crate::thread::AgentThread>| {
        ("history-thread", thread.entity_id().as_u64())
    };

    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        if window.try_find("agent-history-column").is_none() {
            window.click("agent-history", cx);
            window.render_frame(cx);
        }
        replace_textbox("agent-history-search", "DF -H", window, cx);
        window.render_frame(cx);
        // Messages are searched as well as titles, ignoring case.
        assert!(window.try_find(row(&disk)).is_some());
        assert!(window.try_find(row(&nginx)).is_none());
        replace_textbox("agent-history-search", "nothing like it", window, cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-history-no-match").is_some());
        replace_textbox("agent-history-search", "", window, cx);
        window.render_frame(cx);
        // The newest chat comes first, until the older one is pinned.
        assert!(
            window.find(row(&nginx)).bounds().origin.y < window.find(row(&disk)).bounds().origin.y
        );
        f.panel
            .update(cx, |panel, cx| panel.set_pinned(disk.entity_id(), true, cx));
        window.render_frame(cx);
        assert!(
            window.find(row(&disk)).bounds().origin.y < window.find(row(&nginx)).bounds().origin.y
        );

        f.panel.update(cx, |panel, cx| {
            panel.start_rename(disk.entity_id(), window, cx)
        });
        window.render_frame(cx);
        replace_textbox(
            ("history-rename", disk.entity_id().as_u64()),
            "  Disks  ",
            window,
            cx,
        );
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(row(&disk)).label(), Some("Disks"));
    })
    .unwrap();
    let saved = f.repository.chats();
    assert_eq!(saved[0].name.as_deref(), Some("Disks"));
    assert!(saved[0].pinned);

    // A fork starts with the chat's messages, in a copy of its session.
    cx.update_window(f.handle, |_, window, cx| {
        f.panel.update(cx, |panel, cx| {
            panel.fork_thread(nginx.entity_id(), window, cx);
            let fork = panel.current().unwrap();
            activate_thread(&fork, &f.installation.installed, cx)
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let panel = f.panel.read(cx);
        assert_eq!(panel.chat_documents().len(), 3);

        let fork = panel.current().unwrap().read(cx);
        assert_eq!(fork.title(), "restart nginx (fork)");
        assert_eq!(fork.transcript().entries.len(), 2);
        assert_eq!(
            fork.session().clone(),
            Some(acp::SessionId::new(format!("fork-of-{nginx_session}")))
        );
        assert_ne!(
            fork.document().chat_id(),
            nginx.read(cx).document().chat_id().to_owned()
        );
    });
    assert_eq!(
        *f.commands.restores.lock().unwrap(),
        vec![(nginx_session, true)]
    );
    assert_eq!(f.repository.chats().len(), 3);
}

#[test]
fn history_relative_time_boundaries() {
    use std::time::Duration;
    assert_eq!(
        crate::panel::widgets::relative_prompt_time(None),
        "No requests yet"
    );
    for (seconds, expected) in [
        (0, "Just now"),
        (59, "Just now"),
        (60, "1 minute ago"),
        (120, "2 minutes ago"),
        (3599, "59 minutes ago"),
        (3600, "1 hour ago"),
        (7200, "2 hours ago"),
        (86400, "1 day ago"),
        (172800, "2 days ago"),
    ] {
        assert_eq!(
            crate::panel::widgets::relative_prompt_time(Some(Duration::from_secs(seconds))),
            expected
        );
    }
}

fn assert_unaccepted_prompts_leave_no_history(
    thread: &mut AgentThread,
    cx: &mut gpui_kit::Context<AgentThread>,
) {
    assert!(thread.history_entries().is_empty());
    thread.send("  ".into(), cx);
    assert!(thread.history_entries().is_empty());
    thread.begin_composer_edit(Vec::new(), thread.composer().attachments().to_vec(), cx);
    thread.send("Queued before the session is ready".into(), cx);
    assert_eq!(thread.composer().queue().len(), 1);
    let ids = thread.queued_prompt_ids().collect::<Vec<_>>();
    for id in ids {
        thread.remove_queued(id, None, cx);
    }
    assert!(thread.history_entries().is_empty());
    thread.end_composer_edit(false, cx);
}

#[gpui_kit::test]
fn history_records_only_accepted_prompts_and_keeps_model_snapshot(cx: &mut TestAppContext) {
    let f = fixture_with_width(cx, 18., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            assert_unaccepted_prompts_leave_no_history(thread, cx);
            let mut option = acp::SessionConfigOption::select(
                "model",
                "Agent model",
                "first",
                vec![acp::SessionConfigSelectOption::new("first", "First model")],
            );
            option.category = Some(acp::SessionConfigOptionCategory::Model);
            report_control(
                thread,
                acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(vec![option])),
                cx,
            );
            thread.send("First request".into(), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|cx| {
        let thread = f.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| {
            let model = thread.model();
            assert_eq!(model.as_deref(), Some("First model"));
            assert_eq!(thread.transcript().entries.len(), 1);
            report_control(
                thread,
                acp::SessionUpdate::ConfigOptionUpdate(acp::ConfigOptionUpdate::new(Vec::new())),
                cx,
            );
            thread.send("Rejected while busy".into(), cx);
            assert_eq!(thread.transcript().entries.len(), 1);
            assert_eq!(thread.model(), model);
            report_control(
                thread,
                acp::SessionUpdate::SessionInfoUpdate(
                    serde_json::from_value(serde_json::json!({"title":"Updated title"})).unwrap(),
                ),
                cx,
            );
            thread.stop(cx);
            thread.send("Rejected while stopping".into(), cx);
            assert_eq!(thread.transcript().entries.len(), 1);
        });
    });
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id().as_u64();
        window.render_frame(cx);
        let row = window.find(("history-thread", id)).bounds();
        let delete = window.find(("delete-thread", id)).bounds();
        let history = window.find("agent-history-list").bounds();
        assert!(row.size.width > history.size.width * 0.85);
        // The whole row is one target, its buttons included.
        assert!(row.origin.x < delete.origin.x && delete.right() <= row.right());
        assert_eq!(
            window.find(("history-thread", id)).label(),
            Some("Updated title")
        );
        assert!(window.try_find("agent-history-column").is_some());
        window.click(("history-thread", id), cx);
        window.render_frame(cx);
        assert!(
            window.try_find("agent-history-column").is_some(),
            "history stays beside the chat"
        );
        window.click("agent-history", cx);
        window.render_frame(cx);
        assert!(window.try_find("agent-history-column").is_none());
        window.render_frame(cx);
        assert!(window.try_find("agent-history-column").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn history_starts_open_beside_the_first_chat(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        let id = f.panel.read(cx).current().unwrap().entity_id().as_u64();
        window.render_frame(cx);
        assert!(window.find(("history-thread", id)).visible());
        window.click("agent-history", cx);
        window.render_frame(cx);
        assert!(window.try_find(("history-thread", id)).is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn visible_history_refreshes_relative_time_and_hidden_history_stops_waking(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "timestamped question", "answer", cx);
    show_history(&f, cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().entity_id().as_u64());
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(("history-when", id)).label(), Some("Just now"));
    })
    .unwrap();
    let wakes = Rc::new(std::cell::Cell::new(0));
    let observed = wakes.clone();
    let _subscription =
        cx.update(|cx| cx.observe(&f.panel, move |_, _| observed.set(observed.get() + 1)));
    f._directory.metadata.timestamp_is(121);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    assert!(
        wakes.get() > 0,
        "visible history clock must request a redraw"
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("history-when", id)).label(),
            Some("2 minutes ago")
        );
        window.click("agent-history", cx);
        window.render_frame(cx);
        assert!(window.try_find(("history-when", id)).is_none());
    })
    .unwrap();
    cx.run_until_parked();
    wakes.set(0);
    let timestamp_reads = f._directory.metadata.requests().timestamps;
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    assert_eq!(wakes.get(), 0, "hidden history must stop its display clock");
    assert_eq!(f._directory.metadata.requests().timestamps, timestamp_reads);
}

fn second_history_window(
    f: &Fixture,
    cx: &mut TestAppContext,
) -> (AnyWindowHandle, Entity<AgentPanel>) {
    let mut panel = None;
    let (handle, _) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(
                    nocterm_workspace::Deps::new(f.installation.layout.clone()),
                    window,
                    cx,
                );
                let owner = cx.entity().downgrade();
                let view = cx.new(|cx| {
                    AgentPanel::new(
                        f._directory.host(owner, f.installation.installed.clone()),
                        window,
                        cx,
                    )
                });
                workspace.set_right_panel(view.clone(), window, cx);
                workspace.set_right_panel_available(true, window, cx);
                workspace.toggle_right_panel(window, cx);
                panel = Some(view);
                workspace
            })
        })
        .unwrap()
    });
    let panel = panel.unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(1800.), gpui_kit::px(900.)));
        let focus = panel.read(cx).composer_focus(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.click("agent-new-thread", cx);
        window.render_frame(cx);
        window.click("new-agent-codex", cx);
    })
    .unwrap();
    cx.run_until_parked();
    (handle, panel)
}
fn history_width(handle: AnyWindowHandle, cx: &mut TestAppContext) -> gpui_kit::Pixels {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.find("agent-history-column").bounds().size.width
    })
    .unwrap()
}

fn settle_scheduled_history(
    handle: AnyWindowHandle,
    cx: &mut TestAppContext,
) -> Vec<(usize, gpui_kit::Pixels, gpui_kit::Pixels)> {
    let mut delivered = Vec::new();
    for _ in 0..4 {
        let callbacks = cx
            .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
            .unwrap();
        if callbacks == 0 {
            return delivered;
        }
        cx.run_until_parked();
        let widths = cx
            .update_window(handle, |_, window, cx| {
                window.draw(cx).clear(cx);
                (
                    window.find("agent-history-column").bounds().size.width,
                    window.find("agent-panel").bounds().size.width,
                )
            })
            .unwrap();
        delivered.push((callbacks, widths.0, widths.1));
        cx.run_until_parked();
    }
    let remaining = cx
        .update_window(handle, |_, window, cx| window.simulate_next_frame(cx))
        .unwrap();
    assert_eq!(remaining, 0, "history settling must stop requesting frames");
    delivered
}

#[gpui_kit::test]
fn shared_history_width_refreshes_painted_windows_without_scalar_echo(cx: &mut TestAppContext) {
    use gpui_kit::{px, size};
    let f = fixture_with_width(cx, 60., usage_source(), workspace_resolver(), image_deps());
    let layout = &f.installation.layout;
    cx.update(|cx| cx.set_reduce_motion(true));
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(size(px(1800.), px(900.)));
        window.render_frame(cx);
    })
    .unwrap();
    let (other, _panel) = second_history_window(&f, cx);
    let _ = history_width(f.handle, cx);
    let _ = history_width(other, cx);
    let changes = Rc::new(std::cell::Cell::new(0));
    let seen = changes.clone();
    let _subscription = cx.update(|cx| {
        layout.observe_size("agent.history", cx, move |_, _| seen.set(seen.get() + 1))
    });
    cx.update(|cx| layout.set("agent.history", px(360.), cx).unwrap());
    cx.run_until_parked();
    for handle in [f.handle, other] {
        assert!((history_width(handle, cx) - px(360.)).abs() <= px(1.));
        for _ in 0..3 {
            assert!((history_width(handle, cx) - px(360.)).abs() <= px(1.));
        }
    }
    for handle in [f.handle, other] {
        settle_scheduled_history(handle, cx);
        assert!(settle_scheduled_history(handle, cx).is_empty());
    }
    assert_eq!(
        changes.get(),
        1,
        "rendered width adoption must not echo a scalar change"
    );
    cx.update_window(other, |_, window, cx| {
        window.dispatch_action(Box::new(nocterm_workspace::SwapSides), cx);
        window.render_frame(cx);
        window.click("agent-history", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.update(|cx| layout.set("agent.history", px(420.), cx).unwrap());
    cx.run_until_parked();
    cx.update_window(other, |_, window, cx| {
        window.click("agent-history", cx);
        window.render_frame(cx);
    })
    .unwrap();
    let delivered = settle_scheduled_history(other, cx);
    let other_width = cx
        .update_window(other, |_, window, _| {
            window.find("agent-history-column").bounds().size.width
        })
        .unwrap();
    let panel_width = cx
        .update_window(other, |_, window, _| {
            window.find("agent-panel").bounds().size.width
        })
        .unwrap();
    assert!(
        (other_width - px(420.)).abs() <= px(1.),
        "reopened history {other_width:?}, panel {panel_width:?}, scheduled frames {delivered:?}"
    );
    assert!(
        !delivered.is_empty(),
        "shared refresh must schedule its own settling draw"
    );
    assert!(settle_scheduled_history(other, cx).is_empty());
    assert!((history_width(f.handle, cx) - px(420.)).abs() <= px(1.));
    assert_eq!(changes.get(), 2);
}

#[gpui_kit::test]
fn closing_one_window_keeps_the_surviving_history_layout_subscription(cx: &mut TestAppContext) {
    use gpui_kit::px;
    let f = fixture_with_width(cx, 60., usage_source(), workspace_resolver(), image_deps());
    new_chat(&f, cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        cx.set_quit_mode(gpui_kit::QuitMode::Explicit);
    });
    let (survivor, _panel) = second_history_window(&f, cx);
    let layout = f.installation.layout.clone();
    let removed = f.panel.downgrade();
    let _ = history_width(survivor, cx);
    cx.update_window(f.handle, |_, window, _| window.remove_window())
        .unwrap();
    cx.update(|_| drop(f));
    cx.run_until_parked();
    removed.assert_released();
    for width in [360., 420.] {
        cx.update(|cx| layout.set("agent.history", px(width), cx).unwrap());
        cx.run_until_parked();
        let _ = history_width(survivor, cx);
        settle_scheduled_history(survivor, cx);
        assert!((history_width(survivor, cx) - px(width)).abs() <= px(1.));
        assert!(settle_scheduled_history(survivor, cx).is_empty());
    }
}

fn divider_pointer(
    handle: AnyWindowHandle,
    position: gpui_kit::Point<gpui_kit::Pixels>,
    pressed: bool,
    cx: &mut TestAppContext,
) {
    use gpui_kit::InputEvent as _;
    cx.update_window(handle, |_, window, cx| {
        window.dispatch_event(
            gpui_kit::MouseMoveEvent {
                position,
                pressed_button: pressed.then_some(gpui_kit::MouseButton::Left),
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn shared_history_refresh_during_native_drag_keeps_later_pointer_events_safe(
    cx: &mut TestAppContext,
) {
    use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent, point, px, size};
    let f = fixture_with_width(cx, 60., usage_source(), workspace_resolver(), image_deps());
    let layout = &f.installation.layout;
    cx.update(|cx| cx.set_reduce_motion(true));
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.resize(size(px(1800.), px(900.)));
        window.render_frame(cx);
    })
    .unwrap();
    let (other, _panel) = second_history_window(&f, cx);
    let _ = history_width(other, cx);
    let origin = cx
        .update_window(f.handle, |_, window, cx| {
            window.render_frame(cx);
            let history = window.find("agent-history-column").bounds();
            let origin = point(history.left(), history.top() + px(100.));
            window.dispatch_event(
                MouseDownEvent {
                    position: origin,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            origin
        })
        .unwrap();
    let dragged = origin + point(px(-24.), px(0.));
    divider_pointer(f.handle, dragged, true, cx);
    cx.update_window(f.handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
    cx.update(|cx| {
        assert!(
            cx.has_active_drag(),
            "real divider drag must be active before the shared update"
        );
        layout.set("agent.history", px(380.), cx).unwrap();
    });
    cx.run_until_parked();
    // Exercise the previously painted listener before the refresh draw, then the
    // replacement listener and MouseUp after native panel slots are rebuilt.
    divider_pointer(f.handle, dragged + point(px(-1.), px(0.)), true, cx);
    let _ = history_width(f.handle, cx);
    divider_pointer(f.handle, dragged + point(px(-2.), px(0.)), true, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.dispatch_event(
            MouseUpEvent {
                position: dragged,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
    })
    .unwrap();
    assert!((history_width(f.handle, cx) - px(380.)).abs() <= px(1.));
    assert!((history_width(other, cx) - px(380.)).abs() <= px(1.));
    cx.update(|cx| layout.set("agent.history", px(1000.), cx).unwrap());
    cx.run_until_parked();
    let _ = history_width(f.handle, cx);
    assert!(!settle_scheduled_history(f.handle, cx).is_empty());
    let constrained = history_width(f.handle, cx);
    assert!(constrained <= px(640.));
    for _ in 0..4 {
        assert_eq!(history_width(f.handle, cx), constrained);
    }
    assert!(settle_scheduled_history(f.handle, cx).is_empty());
    assert_eq!(
        cx.update(|cx| layout.get("agent.history", cx)),
        Some(px(1000.))
    );
}

fn toggle_workspace_panel(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("toggle-right-panel", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}
fn advance_history_clock(seconds: u64, cx: &mut TestAppContext) {
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(seconds));
    cx.run_until_parked();
}
#[gpui_kit::test]
fn reopened_workspace_panel_keeps_its_clock_after_the_old_deadline(cx: &mut TestAppContext) {
    let f = fixture(cx);
    new_chat(&f, cx);
    exchange(&f, "timestamped question", "answer", cx);
    show_history(&f, cx);
    let id = cx.update(|cx| f.panel.read(cx).current().unwrap().entity_id().as_u64());
    advance_history_clock(10, cx);
    toggle_workspace_panel(&f, cx);
    advance_history_clock(5, cx);
    toggle_workspace_panel(&f, cx);
    let wakes = Rc::new(std::cell::Cell::new(0));
    let seen = wakes.clone();
    let _subscription = cx.update(|cx| cx.observe(&f.panel, move |_, _| seen.set(seen.get() + 1)));
    advance_history_clock(16, cx);
    assert_eq!(
        wakes.get(),
        0,
        "old clock deadline must not wake reopened history"
    );
    f._directory.metadata.timestamp_is(181);
    advance_history_clock(15, cx);
    assert!(
        wakes.get() > 0,
        "reopened history must retain its own display clock"
    );
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find(("history-when", id)).label(),
            Some("3 minutes ago")
        );
    })
    .unwrap();
}
