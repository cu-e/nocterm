//! Status bar widgets and the footer toggles the shell draws beside them.
use super::*;

/// A status bar view that tests find by its element id.
struct Marker(&'static str);
impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().id(self.0).test_support().child(self.0)
    }
}

fn marker(
    name: &'static str,
    created: Rc<Cell<usize>>,
) -> impl Fn(&Workspace, &mut Window, &mut Context<Workspace>) -> Entity<Marker> + 'static {
    move |_, _, cx| {
        created.set(created.get() + 1);
        cx.new(|_| Marker(name))
    }
}

#[gpui_kit::test]
fn a_widget_joins_the_status_bar_list_of_its_zone(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let created = Rc::new(Cell::new(0));
    // The trailing widget registers first, so the zones and not the order place the views.
    for (contribution, name) in [
        (RIGHT_WIDGET, "trailing-marker"),
        (WIDGET, "leading-marker"),
    ] {
        let registration = WidgetRegistration::new(contribution, marker(name, created.clone()));
        let registered = run(cx, handle, &workspace, |workspace, window, cx| {
            window.resize(size(px(1400.), px(700.)));
            workspace.register_widget(registration, window, cx)
        });
        assert_eq!(registered, Ok(()));
    }
    assert_eq!(created.get(), 2);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let leading = window.find("leading-marker").bounds();
        let trailing = window.find("trailing-marker").bounds();
        assert!(leading.right() <= trailing.left());
    })
    .unwrap();
    assert_eq!(
        created.get(),
        2,
        "rendering does not create the views again"
    );
    cx.update(|cx| assert!(workspace.read(cx).is_registered(&id(WIDGET))));
    let again = run(cx, handle, &workspace, |workspace, window, cx| {
        let registration = WidgetRegistration::new(WIDGET, marker("twice", created.clone()));
        workspace.register_widget(registration, window, cx)
    });
    assert_eq!(again, Err(RegistrationError::Duplicate(id(WIDGET))));
    assert_eq!(created.get(), 2, "a refused widget is not created");
}

#[gpui_kit::test]
fn the_local_terminal_descriptor_matches_its_footer_toggle(cx: &mut TestAppContext) {
    let (_, workspace) = fixture(cx);
    cx.update(|cx| {
        let (tooltip, icon) =
            toggle_presentation(workspace.read(cx), Toggle::LocalTerminal, cx).expect("drawn");
        assert_eq!(tooltip.as_ref(), LOCAL_TERMINAL.title);
        assert_eq!(icon, nocterm_ui::contribution_icon(LOCAL_TERMINAL.icon));
    });
}

#[gpui_kit::test]
fn a_widget_left_in_the_tray_is_registered_but_never_created(cx: &mut TestAppContext) {
    let (handle, workspace) = with_everything(cx);
    let created = Rc::new(Cell::new(0));
    let registered = run(cx, handle, &workspace, |workspace, window, cx| {
        window.resize(size(px(1400.), px(700.)));
        let registration = WidgetRegistration::new(TRAY_WIDGET, marker("tray", created.clone()));
        workspace.register_widget(registration, window, cx)
    });
    assert_eq!(registered, Ok(()));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("tray").is_none());
    })
    .unwrap();
    assert_eq!(created.get(), 0);
    cx.update(|cx| assert!(workspace.read(cx).is_registered(&id(TRAY_WIDGET))));
}

fn navigation_toggles(window: &mut Window) -> Vec<usize> {
    (0..8)
        .filter(|ix| {
            window
                .try_find(toggle_id(Toggle::Navigation(*ix)))
                .is_some()
        })
        .collect()
}

#[gpui_kit::test]
fn a_tray_panel_shows_its_toggle_at_the_inner_end_of_the_right_zone_while_it_asks(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = with_everything(cx);
    let panel = Rc::new(RefCell::new(None));
    let registered = register(cx, handle, &workspace, {
        let panel = panel.clone();
        PanelRegistration::new(TRAY_PANEL, move |_, _, cx| {
            let created = TestPanel::builder("tray-panel").title("Tray").build(cx);
            *panel.borrow_mut() = Some(created.clone());
            created
        })
    });
    assert_eq!(registered, Ok(()));
    let panel = panel.borrow().clone().expect("created at registration");
    let drawn = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.resize(size(px(1400.), px(700.)));
            window.render_frame(cx);
            navigation_toggles(window)
        })
        .unwrap()
    };
    let before = drawn(cx);
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_attention(Some("Look".into()), cx)));
    let asking = drawn(cx);
    assert_eq!(asking.len(), before.len() + 1);
    let added = *asking
        .iter()
        .find(|ix| !before.contains(ix))
        .expect("one new toggle");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let toggle = window.find(toggle_id(Toggle::Navigation(added))).bounds();
        let terminal = window.find(toggle_id(Toggle::LocalTerminal)).bounds();
        assert!(toggle.right() <= terminal.left());
    })
    .unwrap();
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_attention(None, cx)));
    assert_eq!(drawn(cx), before);
}

#[gpui_kit::test]
fn the_local_terminal_toggle_opens_a_terminal_for_the_selection_it_was_drawn_under(
    cx: &mut TestAppContext,
) {
    let (handle, workspace) = with_everything(cx);
    let opened = Rc::new(Cell::new(0));
    run(cx, handle, &workspace, |workspace, window, _| {
        window.resize(size(px(1400.), px(700.)));
        let opened = opened.clone();
        workspace.set_local_terminal_opener(move |_, _, _, _| opened.set(opened.get() + 1));
    });
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(toggle_id(Toggle::LocalTerminal), cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(opened.get(), 1);
}
