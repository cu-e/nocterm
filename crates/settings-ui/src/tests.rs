//! Host lifecycle is exercised with supplied pages; cross-owner native controls live in app GUI tests.
use super::*;
use gpui_kit::{Entity, TestAppContext, WindowOptions};
struct EmptyPage {
    focus: FocusHandle,
}
impl Focusable for EmptyPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for EmptyPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl nocterm_ui::SettingsPage for EmptyPage {}
#[gpui_kit::test]
fn supplied_builtin_pages_are_created_eagerly_in_navigation_order(cx: &mut TestAppContext) {
    use std::{cell::RefCell, rc::Rc};
    let created = Rc::new(RefCell::new(Vec::new()));
    let (_window, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = nocterm_ui::init(
            nocterm_ui::DesignTokens::builtin(),
            (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
            cx,
        );
        let builtins = ["appearance", "terminal", "local-shell", "ssh", "ai"].map(|id| {
            let created = created.clone();
            SettingsPageSpec::contextual(id, id, move |_, _, cx| {
                created.borrow_mut().push(id);
                cx.new(|cx| EmptyPage {
                    focus: cx.focus_handle(),
                })
            })
        });
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                SettingsView::with_pages(
                    test_catalog(builtins, Vec::new()),
                    Installed::new(ready.settings().clone()),
                    window,
                    cx,
                )
            })
        })
        .unwrap()
    });
    assert_eq!(
        &*created.borrow(),
        &["appearance", "terminal", "local-shell", "ssh", "ai"]
    );
    cx.update(|cx| assert_eq!(view.read(cx).selected_page_id(), "appearance"));
}

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::test::TestWindowExt as _;
use nocterm_ui::SettingsPage;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
struct HostFixture {
    installed: Installed,
    pages: SettingsCatalog,
}
fn host_fixture(cx: &mut App) -> HostFixture {
    gpui_kit::init(cx);
    let ready = nocterm_ui::init(
        nocterm_ui::DesignTokens::builtin(),
        (nocterm_store::SettingsConfig::in_memory(Default::default())).install(cx),
        cx,
    );
    let builtins = ["appearance", "terminal", "local-shell", "ssh", "ai"].map(|id| {
        SettingsPageSpec::contextual(id, id, |_, _, cx| {
            cx.new(|cx| EmptyPage {
                focus: cx.focus_handle(),
            })
        })
    });
    HostFixture {
        installed: Installed::new(ready.settings().clone()),
        pages: test_catalog(builtins, Vec::new()),
    }
}
fn with_guest(fixture: &HostFixture, guests: Vec<SettingsPageSpec>) -> SettingsCatalog {
    SettingsCatalog::new(
        fixture
            .pages
            .pages()
            .iter()
            .filter(|page| page.lifecycle() == Lifecycle::Eager)
            .cloned()
            .chain(guests.into_iter().map(SettingsPageRegistration::lazy))
            .collect(),
        "appearance",
    )
    .unwrap()
}
fn test_window_options() -> WindowOptions {
    WindowOptions::default()
}
struct GuestPage {
    input: Entity<InputState>,
    deactivations: Rc<Cell<usize>>,
    closures: Rc<Cell<usize>>,
}
impl Focusable for GuestPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.read(cx).focus_handle(cx)
    }
}
impl Render for GuestPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id("guest-page").child(Input::new(&self.input))
    }
}
impl SettingsPage for GuestPage {
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deactivations.set(self.deactivations.get() + 1);
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.closures.set(self.closures.get() + 1);
        self.on_deactivate(window, cx);
    }
}

#[gpui_kit::test]
#[expect(clippy::too_many_lines, reason = "predates the limit")]
fn guest_pages_are_lazy_reused_and_told_when_hidden_or_closed(cx: &mut TestAppContext) {
    let creations = Rc::new(Cell::new(0));
    let deactivations = Rc::new(Cell::new(0));
    let closures = Rc::new(Cell::new(0));
    let guest = Rc::new(RefCell::new(None::<Entity<GuestPage>>));
    let descriptor = SettingsPageSpec::new("vault", "Vault", {
        let creations = creations.clone();
        let deactivations = deactivations.clone();
        let closures = closures.clone();
        let guest = guest.clone();
        move |window, cx| {
            creations.set(creations.get() + 1);
            let entity = cx.new(|cx| GuestPage {
                input: cx.new(|cx| InputState::new(window, cx).masked(true)),
                deactivations: deactivations.clone(),
                closures: closures.clone(),
            });
            *guest.borrow_mut() = Some(entity.clone());
            entity
        }
    });
    let (handle, workspace, fixture) = cx.update(|cx| {
        let fixture = host_fixture(cx);
        let deps = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        let (handle, workspace) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(deps, window, cx);
                register(
                    &mut workspace,
                    with_guest(&fixture, vec![descriptor.clone()]),
                    fixture.installed.clone(),
                );
                workspace
            })
        })
        .unwrap();
        (handle, workspace, fixture)
    });
    let guest_value = |cx: &App| {
        guest
            .borrow()
            .as_ref()
            .unwrap()
            .read(cx)
            .input
            .read(cx)
            .value()
            .to_string()
    };
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            open_page(
                workspace,
                "",
                &with_guest(&fixture, vec![descriptor.clone()]),
                fixture.installed.clone(),
                window,
                cx,
            );
        });
        window.render_frame(cx);
        assert_eq!(creations.get(), 0, "guest must be lazy");
        window.within("settings-pages").click(1usize, cx);
        let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
        assert_eq!(settings.read(cx).selected_page_id(), "terminal");
        workspace.update(cx, |workspace, cx| {
            open_page(
                workspace,
                "vault",
                &with_guest(&fixture, vec![descriptor.clone()]),
                fixture.installed.clone(),
                window,
                cx,
            )
        });
        assert_eq!(workspace.read(cx).items().count(), 1);
        assert_eq!(creations.get(), 1);
        window.render_frame(cx);
        window.input("guest master draft", cx);
        assert_eq!(guest_value(cx), "guest master draft");
        window.within("settings-pages").click(0usize, cx);
        assert_eq!(deactivations.get(), 1);
        assert!(guest_value(cx).is_empty());
        assert_eq!(settings.read(cx).selected_page_id(), "appearance");
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
        workspace.update(cx, |workspace, cx| {
            open_page(
                workspace,
                "vault",
                &with_guest(&fixture, vec![descriptor.clone()]),
                fixture.installed.clone(),
                window,
                cx,
            )
        });
        assert_eq!(creations.get(), 1, "reuse the guest entity");
        window.render_frame(cx);
        window.input("closing guest draft", cx);
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        assert_eq!(closures.get(), 1);
        assert!(guest_value(cx).is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn open_settings_reuses_one_tab(cx: &mut TestAppContext) {
    let (handle, workspace) = cx.update(|cx| {
        let fixture = host_fixture(cx);
        let deps = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            let workspace = cx.new(|cx| {
                let mut workspace = Workspace::new(deps, window, cx);
                register(
                    &mut workspace,
                    fixture.pages.clone(),
                    fixture.installed.clone(),
                );
                workspace
            });
            window.focus(&workspace.read(cx).focus_handle(cx), cx);
            workspace
        })
        .unwrap()
    });
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(Box::new(OpenSettings), cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).items().count(), 1);
        let settings = workspace.read(cx).find_item::<SettingsView>().unwrap();
        assert!(settings.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
}

struct FlushPage {
    focus: FocusHandle,
    id: &'static str,
    events: Rc<RefCell<Vec<&'static str>>>,
}
impl Focusable for FlushPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for FlushPage {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl SettingsPage for FlushPage {
    fn flush(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.events.borrow_mut().push(self.id);
    }
    fn on_deactivate(&mut self, _: &mut Window, _: &mut Context<Self>) {
        if self.id == "guest" {
            self.events.borrow_mut().push("guest-hidden");
        }
    }
    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        if self.id == "guest" {
            self.events.borrow_mut().push("guest-closed");
        }
    }
}
fn flush_catalog(events: Rc<RefCell<Vec<&'static str>>>) -> SettingsCatalog {
    let builtins = ["appearance", "terminal", "local-shell", "ssh", "ai"].map(|id| {
        let events = events.clone();
        SettingsPageSpec::contextual(id, id, move |_, _, cx| {
            cx.new(|cx| FlushPage {
                focus: cx.focus_handle(),
                id,
                events: events.clone(),
            })
        })
    });
    let guest = SettingsPageSpec::new("guest", "Guest", move |_, cx| {
        cx.new(|cx| FlushPage {
            focus: cx.focus_handle(),
            id: "guest",
            events: events.clone(),
        })
    });
    test_catalog(builtins, vec![guest])
}
#[gpui_kit::test]
fn every_builtin_flushes_before_guest_deactivation_and_close(cx: &mut TestAppContext) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (handle, workspace) = cx.update(|cx| {
        let fixture = host_fixture(cx);
        let pages = flush_catalog(events.clone());
        let deps = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(deps, window, cx);
                open_page(
                    &mut workspace,
                    "guest",
                    &pages,
                    fixture.installed,
                    window,
                    cx,
                );
                workspace
            })
        })
        .unwrap()
    });
    events.borrow_mut().clear();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.within("settings-pages").click(0usize, cx);
        assert_eq!(
            &*events.borrow(),
            &[
                "appearance",
                "terminal",
                "local-shell",
                "ssh",
                "ai",
                "guest-hidden"
            ]
        );
        events.borrow_mut().clear();
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
        assert_eq!(
            &*events.borrow(),
            &[
                "appearance",
                "terminal",
                "local-shell",
                "ssh",
                "ai",
                "guest-closed"
            ]
        );
    })
    .unwrap();
}

fn test_catalog(builtins: [SettingsPageSpec; 5], guests: Vec<SettingsPageSpec>) -> SettingsCatalog {
    SettingsCatalog::new(
        builtins
            .into_iter()
            .map(SettingsPageRegistration::eager)
            .chain(guests.into_iter().map(SettingsPageRegistration::lazy))
            .collect(),
        "appearance",
    )
    .unwrap()
}

fn descriptor(id: &'static str, created: Rc<Cell<usize>>) -> SettingsPageSpec {
    SettingsPageSpec::new(id, id, move |_, cx| {
        created.set(created.get() + 1);
        cx.new(|cx| EmptyPage {
            focus: cx.focus_handle(),
        })
    })
}

#[test]
fn catalog_rejects_invalid_ids_and_defaults_before_invoking_any_factory() {
    use SettingsCatalogError::{DuplicateId, EmptyId, MissingDefault};
    let created = Rc::new(Cell::new(0));
    let eager = |id| SettingsPageRegistration::eager(descriptor(id, created.clone()));
    let lazy = |id| SettingsPageRegistration::lazy(descriptor(id, created.clone()));
    assert_eq!(
        SettingsCatalog::new(vec![], "").err(),
        Some(MissingDefault { id: "" })
    );
    assert_eq!(
        SettingsCatalog::new(vec![eager("")], "missing").err(),
        Some(EmptyId { index: 0 })
    );
    assert_eq!(
        SettingsCatalog::new(vec![eager("same"), lazy("same")], "missing").err(),
        Some(DuplicateId { id: "same" })
    );
    assert_eq!(
        SettingsCatalog::new(vec![eager("one"), lazy("")], "one").err(),
        Some(EmptyId { index: 1 })
    );
    assert_eq!(
        SettingsCatalog::new(vec![eager("one")], "absent").err(),
        Some(MissingDefault { id: "absent" })
    );
    assert!(SettingsCatalog::new(vec![eager("A"), lazy("a"), lazy(" ")], " ").is_ok());
    assert_eq!(
        created.get(),
        0,
        "validation and descriptor freezing must be side effect free"
    );
}

#[gpui_kit::test]
fn supplied_order_and_lazy_default_drive_navigation_without_creating_hidden_pages(
    cx: &mut TestAppContext,
) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let created = events.clone();
    let (handle, workspace) = cx.update(|cx| {
        let fixture = host_fixture(cx);
        let registration = |id, eager| {
            let created = created.clone();
            let spec = SettingsPageSpec::new(id, id, move |_, cx| {
                created.borrow_mut().push(id);
                cx.new(|cx| EmptyPage {
                    focus: cx.focus_handle(),
                })
            });
            if eager {
                SettingsPageRegistration::eager(spec)
            } else {
                SettingsPageRegistration::lazy(spec)
            }
        };
        let catalog = SettingsCatalog::new(
            vec![
                registration("hidden", false),
                registration("default", false),
                registration("last eager", true),
                registration("first eager", true),
            ],
            "default",
        )
        .unwrap();
        let deps = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(deps, window, cx);
                open_page(&mut workspace, "", &catalog, fixture.installed, window, cx);
                workspace
            })
        })
        .unwrap()
    });
    assert_eq!(&*events.borrow(), &["last eager", "first eager", "default"]);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx).find_item::<SettingsView>().unwrap();
        assert_eq!(view.read(cx).selected_page_id(), "default");
        window.within("settings-pages").click(3usize, cx);
        assert_eq!(view.read(cx).selected_page_id(), "first eager");
        window.within("settings-pages").click(0usize, cx);
        assert_eq!(view.read(cx).selected_page_id(), "hidden");
        assert_eq!(
            &*events.borrow(),
            &["last eager", "first eager", "default", "hidden"]
        );
        workspace.update(cx, |workspace, cx| workspace.close_item(0, window, cx));
    })
    .unwrap();
}

#[gpui_kit::test]
fn same_and_unknown_page_opens_preserve_selection_and_do_not_flush(cx: &mut TestAppContext) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let (handle, workspace, fixture, pages) = cx.update(|cx| {
        let fixture = host_fixture(cx);
        let pages = flush_catalog(events.clone());
        let deps = nocterm_workspace::Deps::new(nocterm_ui::LayoutHandle::in_memory(cx));
        let (handle, workspace) = gpui_kit::open_window(test_window_options(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(deps, window, cx);
                open_page(
                    &mut workspace,
                    "guest",
                    &pages,
                    fixture.installed.clone(),
                    window,
                    cx,
                );
                workspace
            })
        })
        .unwrap();
        (handle, workspace, fixture, pages)
    });
    events.borrow_mut().clear();
    cx.update_window(handle, |_, window, cx| {
        for id in ["guest", "missing", ""] {
            workspace.update(cx, |workspace, cx| {
                open_page(workspace, id, &pages, fixture.installed.clone(), window, cx)
            });
            let view = workspace.read(cx).find_item::<SettingsView>().unwrap();
            assert_eq!(view.read(cx).selected_page_id(), "guest");
        }
        assert!(events.borrow().is_empty());
    })
    .unwrap();
}
