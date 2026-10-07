// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Native content, tab selection and close-control regressions.
use super::*;
/// The tab group's own frame has to be a flex column.
///
/// gpui's default display is Block, and block layout ignores a child's
/// `flex_grow`. With a plain `div()` frame the content region's `flex_1`
/// does nothing, `#tab-content` sizes to its content, and its only child
/// is the panel view positioned absolutely by `cached` — which
/// contributes no content height. The whole chain resolves to zero and
/// the dock draws a tab bar with nothing under it.
///
/// This asserts a dimension, which the project's testing guidance
/// discourages, because a zero-height content region is not a cosmetic
/// difference: it is the panel not rendering at all, and no behavioral
/// test in this crate can see it. `set_active` still fires, the layout
/// still round-trips, and the window still opens.
#[gpui::test]
fn the_panel_content_region_gets_the_height_below_the_tab_bar(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let height = Rc::new(Cell::new(px(0.)));
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    let measured = height.clone();
    cx.update(|window, cx| {
        let panel = MeasuredProbe::new(measured, cx);
        let layout = DockLayout::tabs().panel_view(panel_handle(panel), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let window_height = cx.update(|window, _| window.viewport_size().height);
    let content = height.get();
    assert!(
        content > px(0.),
        "the panel must receive height; it got {content:?} in a {window_height:?} window"
    );
    // The tab bar is 30px and the padded content region adds none for a
    // single tab, so the panel should get nearly the whole window.
    assert!(
        content > window_height - px(60.),
        "the panel should fill what the tab bar leaves; it got {content:?} \
             of {window_height:?}"
    );
}

/// A collapsed group is a strip of tabs with no content, and the actions
/// act on content. The old `TabPanel::bind_actions` gated them the same
/// way.
///
/// The bottom dock, not a side one: a closed left or right dock draws
/// nothing at all, so it would pass this whether or not the gate exists.
#[gpui::test]
fn a_collapsed_dock_ignores_the_zoom_action(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    let panel = cx.update(|window, cx| {
        let panel = Probe::new(cx);
        let layout = DockLayout::tabs().panel_view(panel_handle(panel.clone()), cx);
        area.update(cx, |area, cx| {
            area.set_dock(DockPlacement::Bottom, layout, window, cx);
            area.toggle_dock(DockPlacement::Bottom, window, cx);
        });
        panel
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        panel.read(cx).focus_handle(cx).focus(window, cx);
    });
    cx.run_until_parked();

    cx.dispatch_action(ToggleZoom);
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| area.read(cx).is_zoomed()),
        false,
        "a collapsed group installs no action handler"
    );
}

/// A panel that carries its own chrome declines the one-panel title bar
/// and gets the whole group.
#[gpui::test]
fn a_panel_without_a_title_bar_gets_the_whole_group(cx: &mut TestAppContext) {
    struct Chromeless {
        focus_handle: FocusHandle,
        height: Rc<Cell<Pixels>>,
    }

    impl gpui_base::dock::Panel for Chromeless {
        fn panel_name(&self) -> &'static str {
            "Chromeless"
        }
    }

    impl Panel for Chromeless {
        fn title_bar(&self, _: &App) -> bool {
            false
        }
    }

    impl EventEmitter<PanelEvent> for Chromeless {}

    impl Focusable for Chromeless {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.focus_handle.clone()
        }
    }

    impl Render for Chromeless {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let height = self.height.clone();
            div()
                .size_full()
                .on_prepaint(move |bounds, _, _| height.set(bounds.size.height))
        }
    }

    cx.update(|cx| crate::init(cx));

    let measure = |cx: &mut TestAppContext, chromeless: bool| -> Pixels {
        let height = Rc::new(Cell::new(px(0.)));
        let (area, cx) = cx.add_window_view(|window, cx| {
            let skin = DockSkin::new(cx);
            DockArea::new("skin", None, window, cx).with_renderer(skin)
        });
        let measured = height.clone();
        cx.update(|window, cx| {
            let panel: Arc<dyn gpui_base::dock::PanelView> = if chromeless {
                panel_handle(cx.new(|cx| Chromeless {
                    focus_handle: cx.focus_handle(),
                    height: measured,
                }))
            } else {
                panel_handle(MeasuredProbe::new(measured, cx))
            };
            let layout = DockLayout::tabs().panel_view(panel, cx);
            area.update(cx, |area, cx| area.set_center(layout, window, cx));
        });
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        height.get()
    };

    let with_title = measure(cx, false);
    let without = measure(cx, true);
    assert!(with_title > px(0.), "the probe must have been drawn");
    assert_eq!(
        without,
        with_title + px(30.),
        "the 30px the title bar took go to the panel instead"
    );
}

/// Panel with a chosen `closable` that records whether it was made active.
struct TabProbe {
    focus_handle: FocusHandle,
    closable: bool,
    activated: Rc<Cell<bool>>,
}

impl TabProbe {
    fn new(closable: bool, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
            closable,
            activated: Rc::new(Cell::new(false)),
        })
    }
}

impl gpui_base::dock::Panel for TabProbe {
    fn panel_name(&self) -> &'static str {
        "TabProbe"
    }

    fn closable(&self, _: &App) -> bool {
        self.closable
    }

    fn set_active(&mut self, active: bool, _: &mut Window, _: &mut Context<Self>) {
        if active {
            self.activated.set(true);
        }
    }
}

impl Panel for TabProbe {}

impl EventEmitter<PanelEvent> for TabProbe {}

impl Focusable for TabProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TabProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Render a two-tab group through the real [`DockSkin`] and report whether
/// the first tab drew a close button. The second tab is a non-closable
/// filler (a lone panel draws no tab bar) that never draws one, so the
/// probe is unambiguous.
fn drew_close_button(cx: &mut TestAppContext, closable: bool, enabled: bool) -> bool {
    cx.update(|cx| crate::init(cx));
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("skin", None, window, cx).with_renderer(renderer)
    });
    let skin = skin.expect("skin constructed with the area");
    if enabled {
        cx.update(|_, cx| skin.set_close_button_visible(true, cx));
    }
    cx.update(|window, cx| {
        let under_test = TabProbe::new(closable, cx);
        let filler = TabProbe::new(false, cx);
        let layout = DockLayout::tabs()
            .panel_view(panel_handle(under_test), cx)
            .panel_view(panel_handle(filler), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let button = cx.debug_bounds(CLOSE_BUTTON_SELECTOR);
    if enabled && closable {
        assert_eq!(
            button.as_ref().map(|bounds| bounds.size.width),
            Some(px(20.)),
            "the close button keeps the standard XS hover target"
        );
    }
    button.is_some()
}

/// A closable panel's tab carries a close (X) button.
#[gpui::test]
fn a_closable_panel_gets_a_close_button(cx: &mut TestAppContext) {
    assert!(
        drew_close_button(cx, true, true),
        "a closable panel's tab must draw a close button"
    );
}

/// A non-closable panel draws no close button (the gate is real).
#[gpui::test]
fn a_non_closable_panel_gets_no_close_button(cx: &mut TestAppContext) {
    assert!(
        !drew_close_button(cx, false, true),
        "a panel that reports itself non-closable must draw no close button"
    );
}

#[gpui::test]
fn close_buttons_are_off_by_default(cx: &mut TestAppContext) {
    assert!(!drew_close_button(cx, true, false));
}

#[gpui::test]
fn close_button_visibility_updates_after_the_skin_setting_changes(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init(cx));
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("skin", None, window, cx).with_renderer(renderer)
    });
    let skin = skin.expect("skin constructed with the area");
    cx.update(|window, cx| {
        let panel = TabProbe::new(true, cx);
        let filler = TabProbe::new(false, cx);
        area.update(cx, |area, cx| {
            area.set_center(
                DockLayout::tabs()
                    .panel_view(panel_handle(panel), cx)
                    .panel_view(panel_handle(filler), cx),
                window,
                cx,
            )
        });
    });

    for (visible, expected) in [(false, false), (true, true), (false, false)] {
        cx.update(|_, cx| skin.set_close_button_visible(visible, cx));
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.debug_bounds(CLOSE_BUTTON_SELECTOR).is_some(), expected);
    }
}

/// Clicking a non-active tab's close button removes that panel by id and
/// does not select it (`stop_propagation`). The displayed tab is a
/// non-closable filler, so the only close button is the one under test.
#[gpui::test]
fn clicking_a_close_button_removes_only_that_tab(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init(cx));
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("skin", None, window, cx).with_renderer(renderer)
    });
    cx.update(|_, cx| skin.unwrap().set_close_button_visible(true, cx));

    let (closable_id, filler_id, activated) = cx.update(|window, cx| {
        let filler = TabProbe::new(false, cx);
        let closable = TabProbe::new(true, cx);
        let activated = closable.read(cx).activated.clone();
        let filler_handle = panel_handle(filler);
        let closable_handle = panel_handle(closable);
        let ids = (closable_handle.panel_id(cx), filler_handle.panel_id(cx));
        // Filler at ix 0 is the displayed tab; the closable tab at ix 1 is
        // never active, so its close button is the only one drawn.
        let layout = DockLayout::tabs()
            .panel_view(filler_handle, cx)
            .panel_view(closable_handle, cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
        (ids.0, ids.1, activated)
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert!(
        cx.update(|_, cx| area.read(cx).panel(closable_id).is_some()),
        "the closable tab starts owned by the area"
    );

    let button = cx
        .debug_bounds(CLOSE_BUTTON_SELECTOR)
        .expect("the non-active closable tab draws a close button");
    cx.simulate_mouse_down(button.center(), MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(button.center(), MouseButton::Left, Modifiers::none());
    cx.run_until_parked();

    assert!(
        cx.update(|_, cx| area.read(cx).panel(closable_id).is_none()),
        "the close click removes exactly the tab it belongs to"
    );
    assert!(
        cx.update(|_, cx| area.read(cx).panel(filler_id).is_some()),
        "the displayed filler tab is untouched"
    );
    assert!(
        !activated.get(),
        "stop_propagation keeps the close click from also selecting the tab"
    );
}

/// A collapsed group offers no close button, though the same group does
/// while open. Before/after collapse isolates `!collapsed`; nothing else
/// changes.
#[gpui::test]
fn a_collapsed_group_draws_no_close_button(cx: &mut TestAppContext) {
    cx.update(|cx| crate::init(cx));
    let mut skin = None;
    let (area, cx) = cx.add_window_view(|window, cx| {
        let renderer = DockSkin::new(cx);
        skin = Some(renderer.clone());
        DockArea::new("skin", None, window, cx).with_renderer(renderer)
    });
    cx.update(|_, cx| skin.unwrap().set_close_button_visible(true, cx));
    // Two closable panels so the group is draggable (not on its last
    // visible panel) and offers close buttons while open.
    cx.update(|window, cx| {
        let a = TabProbe::new(true, cx);
        let b = TabProbe::new(true, cx);
        let layout = DockLayout::tabs()
            .panel_view(panel_handle(a), cx)
            .panel_view(panel_handle(b), cx);
        area.update(cx, |area, cx| {
            area.set_dock(DockPlacement::Bottom, layout, window, cx);
            if !area.is_dock_open(DockPlacement::Bottom) {
                area.toggle_dock(DockPlacement::Bottom, window, cx);
            }
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(CLOSE_BUTTON_SELECTOR).is_some(),
        "an open group with two closable panels offers a close button"
    );

    // Collapse the bottom dock: its strip stays clickable, but the close
    // buttons on it must not.
    cx.update(|window, cx| {
        area.update(cx, |area, cx| {
            area.toggle_dock(DockPlacement::Bottom, window, cx)
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(CLOSE_BUTTON_SELECTOR).is_none(),
        "a collapsed group must not offer a close button"
    );
}
