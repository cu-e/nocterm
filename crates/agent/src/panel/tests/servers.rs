//! Saved servers: opening them from the attach menu, and agents connecting
//! to attached servers in the background.
use super::*;
use nocterm_workspace::ConnectionDirectory as _;
use std::{
    cell::{Cell, RefCell},
    time::Duration,
};

fn summary(id: &str, group: Option<&str>) -> nocterm_workspace::ConnectionSummary {
    nocterm_workspace::ConnectionSummary {
        id: id.to_owned().into(),
        name: id.to_owned().into(),
        group: group.map(|group| group.to_owned().into()),
        description: Default::default(),
        target: serde_json::from_value(
            serde_json::json!({"host":"example.test", "port":22, "user":"user"}),
        )
        .unwrap(),
        icon: None,
        flag: None,
    }
}

/// Opens sessions the way the connections feature does: by updating the
/// workspace it is handed.
struct OpeningDirectory {
    servers: Vec<nocterm_workspace::ConnectionSummary>,
    opened: Cell<usize>,
    background: RefCell<Vec<gpui_kit::EntityId>>,
    sign_in: Rc<SignIn>,
}
impl OpeningDirectory {
    fn terminal(&self, id: &str, cx: &mut Context<Workspace>) -> Entity<FakeTerminal> {
        let access = Rc::new(Access {
            executor: Default::default(),
            lease: Default::default(),
            at_prompt: Cell::new(true),
            sent: Default::default(),
            profile: RefCell::new(Some(id.to_owned().into())),
            sign_in: self.sign_in.clone(),
        });
        cx.new(|cx| FakeTerminal {
            access,
            focus: cx.focus_handle(),
        })
    }
}
impl nocterm_workspace::ConnectionDirectory for OpeningDirectory {
    fn connections(&self, _: &App) -> Vec<nocterm_workspace::ConnectionSummary> {
        self.servers.clone()
    }
    fn open(
        &self,
        id: &str,
        workspace: &gpui_kit::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.opened.set(self.opened.get() + 1);
        workspace
            .update(cx, |workspace, cx| {
                let item = self.terminal(id, cx);
                workspace.add_item(item, window, cx)
            })
            .is_ok()
    }
    fn open_background(
        &self,
        id: &str,
        workspace: &gpui_kit::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<gpui_kit::EntityId> {
        let item = workspace
            .update(cx, |workspace, cx| {
                let item = self.terminal(id, cx);
                workspace.add_background_item(item, window, cx)
            })
            .ok()?;
        self.background.borrow_mut().push(item);
        Some(item)
    }
}

fn install(f: &Fixture, cx: &mut TestAppContext) -> Rc<OpeningDirectory> {
    let directory = Rc::new(OpeningDirectory {
        servers: vec![summary("web", Some("prod")), summary("db", Some("prod"))],
        opened: Cell::new(0),
        background: RefCell::new(Vec::new()),
        sign_in: Default::default(),
    });
    cx.update(|cx| {
        f.workspace.update(cx, |workspace, _| {
            workspace.set_connection_directory(directory.clone())
        })
    });
    directory
}

#[gpui_kit::test]
fn open_in_the_attach_menu_opens_a_tab(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    new_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        assert!(window.try_find("Active sessions").is_some());
        assert!(window.try_find("Saved servers").is_some());
        window.click("open-connection-web", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(directory.opened.get(), 1);
        assert_eq!(f.workspace.read(cx).items().count(), 2);
        assert!(f.panel.read(cx).menu.is_none(), "the menu closes");
    });
}

#[gpui_kit::test]
fn agents_open_attached_offline_servers_in_the_background(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let (server, context) = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Group("prod".into()), cx);
            let servers = thread.offline_servers(cx);
            assert_eq!(servers.len(), 2, "both servers in the folder are offline");
            let web = servers
                .iter()
                .find(|(_, summary)| summary.id.as_ref() == "web")
                .unwrap()
                .0
                .server_id
                .clone();
            (web, thread.context(cx))
        })
    });
    assert!(context.contains("offline_servers"), "{context}");
    assert!(context.contains(&server));
    let (respond, response) = oneshot::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let registration = thread.registration().as_ref().unwrap().id;
            thread.handle_tool(
                BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id: registration,
                    call: nocterm_ai::TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal {
                        server_id: server.clone(),
                    }),
                    respond,
                },
                cx,
            );
            assert_eq!(thread.tools.len(), 1, "connecting asks first");
            thread.approve_tool(0, true, true, cx);
        })
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let answer = response.now_or_never().unwrap().unwrap().unwrap();
    let terminal = answer["terminal"]["id"].as_str().unwrap().to_owned();
    let background = directory.background.borrow()[0];
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert!(workspace.is_background(background));
        assert_eq!(workspace.items().count(), 1, "no tab was opened");
        thread.update(cx, |thread, cx| {
            assert_eq!(thread.background, vec![background]);
            assert!(thread.resolved(cx).iter().any(|(id, _, _)| *id == terminal));
            assert_eq!(thread.offline_servers(cx).len(), 1, "web is online now");
            // Detaching the folder ends the session the chat opened.
            thread.attach(Attachment::Group("prod".into()), cx);
        });
    });
    cx.run_until_parked();
    cx.update(|cx| assert!(!f.workspace.read(cx).is_background(background)));
}

#[gpui_kit::test]
fn a_background_session_can_be_shown_in_a_tab(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    new_chat(&f, cx);
    let item = cx
        .update_window(f.handle, |_, window, cx| {
            let workspace = f.workspace.downgrade();
            directory.open_background("db", &workspace, window, cx)
        })
        .unwrap()
        .unwrap();
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        window.click(format!("show-background-{item:?}"), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let workspace = f.workspace.read(cx);
        assert!(!workspace.is_background(item));
        assert!(workspace.items().any(|open| open.item_id() == item));
    });
}

#[gpui_kit::test]
fn a_locked_vault_is_unlocked_from_the_chat_without_a_tab(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    directory.sign_in.vault_locked.set(true);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Group("prod".into()), cx);
            thread.offline_servers(cx)[0].0.server_id.clone()
        })
    });
    let (respond, mut response) = oneshot::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let registration = thread.registration().as_ref().unwrap().id;
            thread.handle_tool(
                BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id: registration,
                    call: nocterm_ai::TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal {
                        server_id: server,
                    }),
                    respond,
                },
                cx,
            );
            thread.approve_tool(0, true, true, cx);
        })
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(response.try_recv().unwrap().is_none(), "still signing in");
    let unlocks = Rc::new(Cell::new(0));
    let counter = unlocks.clone();
    cx.update_window(f.handle, |_, window, cx| {
        assert_eq!(thread.read(cx).sign_ins.len(), 1);
        assert!(thread.read(cx).sign_ins[0].vault);
        assert_eq!(f.workspace.read(cx).items().count(), 1, "no tab was opened");
        window.render_frame(cx);
        assert!(window.try_find("agent-approvals").is_some());

        cx.on_action(move |_: &nocterm_workspace::UnlockVault, _| counter.set(counter.get() + 1));
        window.click(("vault-unlock", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        unlocks.get(),
        1,
        "the button asks to open the unlock dialog"
    );
    // Unlocking answers the session's prompt with the saved secret.
    directory.sign_in.vault_locked.set(false);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    let answer = response.try_recv().unwrap().unwrap().unwrap();
    assert!(answer["terminal"]["id"].is_string());
    cx.update(|cx| {
        assert!(thread.read(cx).sign_ins.is_empty());
        assert_eq!(f.workspace.read(cx).items().count(), 1, "still no tab");
    });
}

#[gpui_kit::test]
fn a_password_is_typed_in_the_chat_without_a_tab(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let directory = install(&f, cx);
    directory.sign_in.asks.set(true);
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let server = cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Group("prod".into()), cx);
            thread.offline_servers(cx)[0].0.server_id.clone()
        })
    });
    let (respond, mut response) = oneshot::channel();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let registration = thread.registration().as_ref().unwrap().id;
            thread.handle_tool(
                BridgeCall {
                    arguments: None,
                    display_token: None,
                    registration_id: registration,
                    call: nocterm_ai::TerminalCall::OpenTerminal(nocterm_ai::OpenTerminal {
                        server_id: server,
                    }),
                    respond,
                },
                cx,
            );
            thread.approve_tool(0, true, true, cx);
        })
    });
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(response.try_recv().unwrap().is_none(), "still signing in");
    let item = cx.update(|cx| thread.read(cx).sign_ins[0].item);
    cx.update_window(f.handle, |_, window, cx| {
        assert!(!thread.read(cx).sign_ins[0].vault);
        assert_eq!(f.workspace.read(cx).items().count(), 1, "no tab was opened");
        window.render_frame(cx);
        assert!(window.try_find(("vault-unlock", 0usize)).is_none());
        let input = f.panel.read(cx).sign_in_inputs[&item].input.clone();
        input.update(cx, |input, cx| input.set_value("hunter2", window, cx));
        window.click(("sign-in-submit", 0usize), cx);
    })
    .unwrap();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(*directory.sign_in.answers.borrow(), ["hunter2"]);
    let answer = response.try_recv().unwrap().unwrap().unwrap();
    assert!(answer["terminal"]["id"].is_string());
    cx.update(|cx| {
        assert!(thread.read(cx).sign_ins.is_empty());
        assert_eq!(f.workspace.read(cx).items().count(), 1, "still no tab");
    });
}

#[gpui_kit::test]
fn rows_attached_through_a_folder_do_not_toggle_on_their_own(cx: &mut TestAppContext) {
    let f = fixture(cx);
    install(&f, cx);
    // The chat's terminal is a session of `web`, filed under `prod`.
    *f.access.profile.borrow_mut() = Some("web".into());
    new_chat(&f, cx);
    let thread = cx.update(|cx| f.panel.read(cx).current().unwrap());
    let attachments = |cx: &mut gpui_kit::App| thread.read(cx).composer.attachments.clone();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Terminal(f.terminal), cx);
            assert!(thread.composer.attachments.is_empty());
        })
    });
    let terminal = format!("attach-{:?}", f.terminal);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        window.click("attach-group-prod", cx);
        window.render_frame(cx);
        let prod = vec![Attachment::Group("prod".into())];
        assert_eq!(attachments(cx), prod);
        // Its servers and their sessions show as attached, and a click
        // cannot leave a duplicate behind once the folder is detached.
        for row in ["attach-connection-db", terminal.as_str()] {
            window.click(row.to_owned(), cx);
            window.render_frame(cx);
            assert_eq!(attachments(cx), prod, "{row}");
        }
        window.click("attach-group-prod", cx);
        window.render_frame(cx);
        assert!(attachments(cx).is_empty());
        window.click("attach-connection-db", cx);
        window.render_frame(cx);
        assert_eq!(attachments(cx), [Attachment::Connection("db".into())]);
    })
    .unwrap();
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            assert!(thread.resolved(cx).is_empty(), "web is no longer attached");
            let servers = thread.offline_servers(cx);
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0].1.id.as_ref(), "db");
        })
    });
}

#[gpui_kit::test]
fn a_folder_without_servers_says_so(cx: &mut TestAppContext) {
    let f = fixture(cx);
    install(&f, cx);
    cx.update(|cx| {
        let labels = super::super::attachments::AttachmentLabels::new(&f.workspace.downgrade(), cx);
        assert_eq!(
            labels.label(&Attachment::Group("prod".into())),
            "Group prod"
        );
        // Renamed away, or emptied: the chat keeps the folder, the agent gets nothing.
        assert_eq!(
            labels.label(&Attachment::Group("homelab".into())),
            "Group homelab · no servers"
        );
        assert_eq!(
            labels.label(&Attachment::Connection("deleted".into())),
            "Unavailable connection"
        );
    });
}
