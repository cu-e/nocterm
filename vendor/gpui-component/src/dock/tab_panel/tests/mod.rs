// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.

use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Modifiers, MouseButton,
    Pixels, Render, TestAppContext, VisualTestContext,
};
use gpui_base::dock::{DockArea, DockAreaRenderer, DockLayout, DockPlacement, PanelEvent};

use super::*;
use crate::dock::{
    DockSkin, Panel, panel_handle,
    test_support::{HideableProbe, MeasuredProbe},
};

struct Probe {
    focus_handle: FocusHandle,
}

impl Probe {
    fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl gpui_base::dock::Panel for Probe {
    fn panel_name(&self) -> &'static str {
        "Probe"
    }
}

impl Panel for Probe {}
impl EventEmitter<PanelEvent> for Probe {}

impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Draws nothing, but runs the real gates the skin's tab bar runs and
/// records what they decided for every group it was asked to draw.
#[derive(Default)]
struct Recorded {
    /// One entry per tab: whether its tab would start a drag.
    draggable: Vec<bool>,
}

struct Recorder {
    log: Rc<RefCell<Recorded>>,
}

impl TabGroupRenderer for Recorder {
    fn render_tab_bar(&self, group: &TabGroupContext, _: &mut Window, cx: &mut App) -> AnyElement {
        let mut log = self.log.borrow_mut();
        for ix in 0..group.panels().len() {
            log.draggable.push(tab_drag(group, ix, cx).is_some());
        }
        Empty.into_any_element()
    }
}

impl DockAreaRenderer for Recorder {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("recorder").size_full()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(Recorder {
            log: self.log.clone(),
        })
    }
}

fn recording_area(
    cx: &mut TestAppContext,
) -> (
    Entity<DockArea>,
    Rc<RefCell<Recorded>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let log = Rc::new(RefCell::new(Recorded::default()));
    let renderer = Rc::new(Recorder { log: log.clone() });
    let (area, cx) = cx.add_window_view(|window, cx| {
        DockArea::new("skin", None, window, cx).with_renderer(renderer)
    });
    (area, log, cx)
}

/// The gate carried forward from the old `TabPanel::render`, which wrapped
/// `on_drag` in `.when(state.draggable, ..)`. Base does not enforce it:
/// `TabGroupContext::drag_panel` answers for any tab in range.
#[gpui::test]
fn the_last_group_in_a_dock_offers_no_drag(cx: &mut TestAppContext) {
    let (area, log, cx) = recording_area(cx);

    cx.update(|window, cx| {
        let layout = DockLayout::tabs().panel_view(panel_handle(Probe::new(cx)), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    log.borrow_mut().draggable.clear();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert_eq!(
        log.borrow().draggable,
        vec![false],
        "the only visible panel in the dock has nowhere to go, so its tab \
             must not start a drag"
    );
}

#[gpui::test]
fn a_group_beside_another_offers_a_drag(cx: &mut TestAppContext) {
    let (area, log, cx) = recording_area(cx);

    cx.update(|window, cx| {
        let layout = DockLayout::h_split()
            .child(
                DockLayout::tabs().panel_view(panel_handle(Probe::new(cx)), cx),
                None,
            )
            .child(
                DockLayout::tabs().panel_view(panel_handle(Probe::new(cx)), cx),
                None,
            );
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    log.borrow_mut().draggable.clear();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    assert_eq!(
        log.borrow().draggable,
        vec![true, true],
        "each group has somewhere to go, so both tabs start a drag"
    );
}

/// A panel that allows zooming and asks for the control in the toolbar.
/// `Probe`'s default is `PanelControl::Menu`, which draws no button.
struct ToolbarZoomProbe {
    focus_handle: FocusHandle,
}

impl ToolbarZoomProbe {
    fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl gpui_base::dock::Panel for ToolbarZoomProbe {
    fn panel_name(&self) -> &'static str {
        "ToolbarZoomProbe"
    }
}

impl Panel for ToolbarZoomProbe {
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        Some(PanelControl::Toolbar)
    }
}

impl EventEmitter<PanelEvent> for ToolbarZoomProbe {}

impl Focusable for ToolbarZoomProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ToolbarZoomProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A panel that offers no zoom control but leaves base's `zoomable`
/// default alone. Withholding the control is meant to be enough.
struct NoControlProbe {
    focus_handle: FocusHandle,
}

impl NoControlProbe {
    fn new(cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus_handle: cx.focus_handle(),
        })
    }
}

impl gpui_base::dock::Panel for NoControlProbe {
    fn panel_name(&self) -> &'static str {
        "NoControlProbe"
    }
}

impl Panel for NoControlProbe {
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        None
    }
}

impl EventEmitter<PanelEvent> for NoControlProbe {}

impl Focusable for NoControlProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NoControlProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A panel that says "never zoom" in base's half but still names a place
/// for the control in this crate's half.
struct UnzoomableProbe {
    focus_handle: FocusHandle,
}

impl gpui_base::dock::Panel for UnzoomableProbe {
    fn panel_name(&self) -> &'static str {
        "UnzoomableProbe"
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for UnzoomableProbe {
    fn zoom_control(&self, _: &App) -> Option<PanelControl> {
        Some(PanelControl::Toolbar)
    }
}

impl EventEmitter<PanelEvent> for UnzoomableProbe {}

impl Focusable for UnzoomableProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for UnzoomableProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// Draw one panel through the real [`DockSkin`] and report whether its tab
/// bar offered a zoom control.
///
/// The real skin, not a recorder: the bug this guards is `render_toolbar`
/// asking only half the question, so a test that calls `zoom_control`
/// itself would pass with the bug in place.
fn drew_zoom_control(
    cx: &mut TestAppContext,
    panel: impl FnOnce(&mut App) -> Arc<dyn gpui_base::dock::PanelView>,
) -> bool {
    cx.update(|cx| {
        crate::init(cx);
    });
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    cx.update(|window, cx| {
        let layout = DockLayout::tabs().panel_view(panel(cx), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.debug_bounds(ZOOM_CONTROL_SELECTOR).is_some()
}

/// The two halves of the old `zoomable()` can now disagree, and base is
/// the one that decides. A control drawn against base's refusal is dead:
/// pressing it does nothing.
#[gpui::test]
fn a_panel_base_will_not_zoom_gets_no_zoom_control(cx: &mut TestAppContext) {
    let drew = drew_zoom_control(cx, |cx| {
        panel_handle(cx.new(|cx| UnzoomableProbe {
            focus_handle: cx.focus_handle(),
        }))
    });

    assert!(
        !drew,
        "the panel names a place for the control, but base refuses the zoom"
    );
}

/// The other half: a panel that allows zoom and asks for a toolbar control
/// gets one drawn. Without this the test above would also pass a skin that
/// never draws a zoom control at all.
#[gpui::test]
fn a_zoomable_panel_gets_its_zoom_control(cx: &mut TestAppContext) {
    let drew = drew_zoom_control(cx, |cx| panel_handle(ToolbarZoomProbe::new(cx)));

    assert!(
        drew,
        "a zoomable panel asking for a toolbar control gets one"
    );
}

/// The centre and the bottom dock share the centre column, and both get
/// height.
///
/// `DockSkin::center_frame` is a flex column holding the centre's root
/// split and the bottom dock, and the centre's split frame sits between
/// them. This pins that the frame carries a size at all: strip
/// `split_frame` of both `size_full` and `flex_1` and every panel here
/// measures zero. It does not pin *which* of the two does the work —
/// either alone passes.
#[gpui::test]
fn the_centre_and_the_bottom_dock_share_the_column(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let centre = Rc::new(Cell::new(px(0.)));
    let bottom = Rc::new(Cell::new(px(0.)));
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    let (centre_probe, bottom_probe) = (centre.clone(), bottom.clone());
    cx.update(|window, cx| {
        let centre_panel = MeasuredProbe::new(centre_probe, cx);
        let bottom_panel = MeasuredProbe::new(bottom_probe, cx);
        area.update(cx, |area, cx| {
            // A split inside a split, so the *nested* `split_frame` — the
            // one that sits inside a `resizable_panel` — is exercised too,
            // not only the centre's root.
            area.set_center(
                DockLayout::v_split().child(
                    DockLayout::h_split().child(
                        DockLayout::tabs().panel_view(panel_handle(centre_panel), cx),
                        None,
                    ),
                    None,
                ),
                window,
                cx,
            );
            area.set_dock(
                DockPlacement::Bottom,
                DockLayout::tabs().panel_view(panel_handle(bottom_panel), cx),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Bottom, px(200.), window, cx);
        });
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let window_height = cx.update(|window, _| window.viewport_size().height);
    assert!(
        centre.get() > px(0.),
        "the centre panel must receive height; it got {:?}",
        centre.get()
    );
    assert!(
        bottom.get() > px(0.),
        "the bottom dock's panel must receive height; it got {:?}",
        bottom.get()
    );
    assert!(
        centre.get() < window_height - px(150.),
        "the centre must give the 200px bottom dock its share; the centre \
             got {:?} of {window_height:?}",
        centre.get()
    );
}

/// Whichever slots of a split are hidden, the drawn ones fill it.
///
/// `render_node` pins every slot but one to a fixed size and lets the
/// remaining one absorb whatever the container has spare. Picking that
/// slot by tree position alone picks a hidden one whenever the trailing
/// container's panels are all hidden — nothing draws there, nothing
/// grows, and the split stops short of its frame, showing a band of the
/// frame's own background under the last visible panel. Hiding a slot is
/// an everyday event: a panel that is only meaningful for some symbols
/// answers `visible` with `false` for the rest.
///
/// Every subset is covered because the defect is positional: only the
/// cases that hide the trailing slot fail, and a test that hid one fixed
/// slot would pass against a fix that only special-cased that slot.
#[gpui::test]
fn a_split_fills_its_container_whichever_slots_are_hidden(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let heights: Vec<Rc<Cell<Pixels>>> = (0..3).map(|_| Rc::new(Cell::new(px(0.)))).collect();
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    let slots = heights.clone();
    let probes = cx.update(|window, cx| {
        let probes: Vec<_> = slots
            .iter()
            .map(|height| HideableProbe::new(height.clone(), cx))
            .collect();
        area.update(cx, |area, cx| {
            area.set_dock(
                DockPlacement::Right,
                probes.iter().zip([260., 320., 200.]).fold(
                    DockLayout::v_split(),
                    |split, (probe, size)| {
                        split.child(
                            DockLayout::tabs().panel_view(panel_handle(probe.clone()), cx),
                            Some(px(size)),
                        )
                    },
                ),
                window,
                cx,
            );
            area.set_dock_size(DockPlacement::Right, px(380.), window, cx);
        });
        probes
    });
    cx.run_until_parked();
    let draw = |cx: &mut VisualTestContext| {
        cx.update(|window, _| window.refresh());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
    };
    draw(cx);

    let dock_height = cx.update(|window, _| window.viewport_size().height);
    // Each slot spends a tab bar out of its height and the probe under it
    // measures the rest, so the drawn slots account for the whole dock
    // once one tab bar per drawn slot is added back.
    let drawn: Pixels = heights.iter().map(|height| height.get()).sum();
    let bar = (dock_height - drawn) / 3.;
    assert!(
        bar > px(0.) && bar < px(60.),
        "the three slots fill the dock to begin with, one tab bar each; \
             that leaves {bar:?} per slot of {dock_height:?}"
    );

    // Every subset except "all three hidden", which gives the whole node
    // up to *its* parent and so has no container of its own to fill.
    for hidden in 1..0b111u8 {
        let shown = (0..3).filter(|slot| hidden & (1 << slot) == 0);
        cx.update(|_, cx| {
            for (slot, probe) in probes.iter().enumerate() {
                // A sentinel, so a slot that stopped drawing is not read
                // as one that kept the height it had.
                heights[slot].set(px(-1.));
                probe.update(cx, |probe, cx| {
                    probe.set_visible(hidden & (1 << slot) == 0, cx)
                });
            }
        });
        cx.run_until_parked();
        draw(cx);

        let mut count = 0;
        let mut total = px(0.);
        for slot in shown {
            assert_ne!(
                heights[slot].get(),
                px(-1.),
                "hiding {hidden:03b}: slot {slot} is shown and must draw"
            );
            count += 1;
            total += heights[slot].get();
        }
        let empty = dock_height - total - bar * count as f32;
        assert!(
            empty.abs() < px(1.),
            "hiding {hidden:03b}: the drawn slots must take the hidden \
                 ones' space between them; they left {empty:?} of \
                 {dock_height:?} empty"
        );
    }
}

/// The old dock installed `ToggleZoom` and `ClosePanel` on the tab panel
/// itself; base installs neither, so the skin's `frame` is the only place
/// the keybindings reach.
#[gpui::test]
fn the_zoom_action_reaches_the_group_through_the_skin(cx: &mut TestAppContext) {
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
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
        panel
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        panel.read(cx).focus_handle(cx).focus(window, cx);
    });
    cx.run_until_parked();

    assert_eq!(cx.read(|cx| area.read(cx).is_zoomed()), false);
    cx.dispatch_action(ToggleZoom);
    cx.run_until_parked();
    assert_eq!(
        cx.read(|cx| area.read(cx).is_zoomed()),
        true,
        "the skin's frame is what carries the ToggleZoom handler"
    );

    cx.dispatch_action(ToggleZoom);
    cx.run_until_parked();
    assert_eq!(cx.read(|cx| area.read(cx).is_zoomed()), false);
}

/// Withholding the control withholds the whole affordance, keybinding
/// included. The two halves of the zoom question are asked in one place —
/// the skin's `frame` — so a doc claiming the action gets through anyway
/// would send a panel author looking for a second switch that does not
/// exist.
#[gpui::test]
fn the_zoom_action_refuses_a_panel_that_offers_no_control(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
    });
    let (area, cx) = cx.add_window_view(|window, cx| {
        let skin = DockSkin::new(cx);
        DockArea::new("skin", None, window, cx).with_renderer(skin)
    });

    let panel = cx.update(|window, cx| {
        let panel = NoControlProbe::new(cx);
        let layout = DockLayout::tabs().panel_view(panel_handle(panel.clone()), cx);
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
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
        "no control means no zoom, however the zoom was asked for"
    );
}

mod rendering;
