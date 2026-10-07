// Added by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
use super::*;
use crate::{
    Icon, Theme, ThemeMode,
    dock::{DockSkin, Panel, TitleStyle, panel_handle},
    floating::FloatingCards,
    h_flex,
};
use gpui::{
    Entity, EventEmitter, FocusHandle, Focusable, Modifiers, MouseButton, TestAppContext,
    VisualTestContext, point,
};
use gpui_base::dock::{DockArea, DockLayout, PanelEvent};

const NAME: &str = "a long terminal name with native status and connection details";
const ALIAS: &str = "a long alias that must remain exactly as displayed";

struct Probe {
    focus: FocusHandle,
    alias: bool,
    title_background: Option<Hsla>,
}
impl Probe {
    fn new(alias: bool, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| Self {
            focus: cx.focus_handle(),
            alias,
            title_background: None,
        })
    }
}
impl gpui_base::dock::Panel for Probe {
    fn panel_name(&self) -> &'static str {
        "Drag appearance probe"
    }
    fn closable(&self, _: &App) -> bool {
        true
    }
}
impl Panel for Probe {
    fn title_style(&self, cx: &App) -> Option<TitleStyle> {
        self.title_background.map(|background| TitleStyle {
            background,
            foreground: cx.theme().foreground,
        })
    }
    fn tab_name(&self, _: &App) -> Option<gpui::SharedString> {
        self.alias.then(|| ALIAS.into())
    }
    fn tab_accent(&self, _: &App) -> Option<Hsla> {
        Some(gpui::rgb(0x336699).into())
    }
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .child(Icon::new(IconName::Check).small())
            .child(
                div()
                    .debug_selector(|| "drag-probe-title".into())
                    .child(NAME),
            )
    }
}
impl EventEmitter<PanelEvent> for Probe {}
impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

type CapturedDrag = Rc<RefCell<Option<DragPanel>>>;
struct Root {
    area: Entity<DockArea>,
    captured: CapturedDrag,
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let captured = self.captured.clone();
        div()
            .size_full()
            .on_drag_move::<DragPanel>(move |event, _, cx| {
                *captured.borrow_mut() = Some(event.drag(cx).clone())
            })
            .child(self.area.clone())
    }
}

fn fixture(
    cx: &mut TestAppContext,
    floating: bool,
    alias: bool,
    title: bool,
) -> (&mut VisualTestContext, CapturedDrag) {
    cx.update(|cx| {
        crate::init(cx);
        Theme::change(
            if floating {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
            None,
            cx,
        );
        if floating {
            cx.set_global(FloatingCards {
                gap: px(8.),
                radius: px(8.),
                canvas: cx.theme().background,
                surface: cx.theme().background,
                border: cx.theme().border,
                shadow: None,
            });
        }
    });
    let captured = CapturedDrag::default();
    let mut skin = None;
    let (root, cx) = cx.add_window_view(|window, cx| Root {
        area: cx.new(|cx| {
            let renderer = DockSkin::new(cx);
            skin = Some(renderer.clone());
            DockArea::new("drag-test", None, window, cx).with_renderer(renderer)
        }),
        captured: captured.clone(),
    });
    cx.update(|_, cx| skin.unwrap().set_close_button_visible(true, cx));
    let area = cx.update(|_, cx| root.read(cx).area.clone());
    cx.update(|window, cx| {
        let first = panel_handle(Probe::new(alias, cx));
        let second = panel_handle(Probe::new(false, cx));
        let layout = if title {
            DockLayout::h_split()
                .child(DockLayout::tabs().panel_view(first, cx), None)
                .child(DockLayout::tabs().panel_view(second, cx), None)
        } else {
            DockLayout::tabs()
                .panel_view(first, cx)
                .panel_view(second, cx)
        };
        area.update(cx, |area, cx| area.set_center(layout, window, cx));
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (cx, captured)
}

fn assert_native_preview(
    cx: &mut VisualTestContext,
    captured: CapturedDrag,
    selector: &'static str,
    has_close: bool,
) {
    let source = cx
        .debug_bounds(selector)
        .expect("source drag surface drawn");
    assert!(
        source.size.width > px(96.),
        "source is wider than the old fixed plate"
    );
    let label = cx.debug_bounds("drag-probe-title");
    let start = source.origin + point(px(18.), px(10.));
    cx.simulate_mouse_move(start, None, Modifiers::none());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let threshold = start + point(px(18.), px(0.));
    cx.simulate_mouse_move(threshold, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let preview = cx
        .debug_bounds("dock-native-drag-preview")
        .expect("native drag preview");
    assert_eq!(preview.size, source.size);
    assert_eq!(preview.origin, source.origin);
    if has_close {
        let close = cx
            .debug_bounds(CLOSE_BUTTON_SELECTOR)
            .expect("passive close suffix retained");
        assert!(close.left() > source.left() && close.right() <= source.right());
    }
    if let Some(label) = label {
        let preview_label = cx.debug_bounds("drag-probe-title").unwrap();
        assert_eq!(
            preview_label.size, label.size,
            "status/title typography retained"
        );
    }
    let destination = threshold + point(px(36.), px(25.));
    cx.simulate_mouse_move(destination, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let preview = cx.debug_bounds("dock-native-drag-preview").unwrap();
    assert_eq!(preview.origin, source.origin + point(px(36.), px(25.)));
    let payload = captured.borrow();
    let payload = payload
        .as_ref()
        .expect("unchanged dock drag payload observed through native drag movement");
    assert_eq!(payload.preview_size(), source.size);
    assert_eq!(payload.drag_offset(), threshold - source.origin);
    cx.simulate_mouse_up(destination, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn native_tab_preview_preserves_painted_size_status_alias_close_and_offset(
    cx: &mut TestAppContext,
) {
    let (cx, captured) = fixture(cx, false, false, false);
    assert_native_preview(cx, captured, "dock-tab-drag-source-0", true);
}

#[gpui::test]
fn native_floating_tab_preview_preserves_selected_alias_and_offset(cx: &mut TestAppContext) {
    let (cx, captured) = fixture(cx, true, true, false);
    assert_native_preview(cx, captured, "dock-tab-drag-source-0", true);
}

#[gpui::test]
fn native_inactive_tab_preview_preserves_painted_size_close_and_offset(cx: &mut TestAppContext) {
    let (cx, captured) = fixture(cx, false, false, false);
    assert_native_preview(cx, captured, "dock-tab-drag-source-1", true);
}

#[gpui::test]
fn preview_background_preserves_bar_card_and_custom_title_surfaces(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::init(cx);
        let panel = panel_handle(Probe::new(false, cx));
        let mut tab = TabVisual {
            panel: panel.clone(),
            ix: 0,
            has_leading: false,
            floating: false,
            selected: false,
            accent: None,
            close: true,
        };
        assert_eq!(
            PreviewContent::Tab(tab.clone()).background(cx),
            *cx.theme().tokens.tab_bar,
        );
        assert_eq!(
            PreviewContent::Title(panel.clone()).background(cx),
            *cx.theme().tokens.background,
        );
        let surface = gpui::rgb(0x254363).into();
        cx.set_global(FloatingCards {
            gap: px(8.),
            radius: px(8.),
            canvas: cx.theme().background,
            surface,
            border: cx.theme().border,
            shadow: None,
        });
        tab.floating = true;
        assert_eq!(PreviewContent::Tab(tab).background(cx), surface);
        assert_eq!(PreviewContent::Title(panel).background(cx), surface);
        let custom = gpui::rgb(0x614529).into();
        let panel = panel_handle(cx.new(|cx| Probe {
            focus: cx.focus_handle(),
            alias: false,
            title_background: Some(custom),
        }));
        assert_eq!(PreviewContent::Title(panel).background(cx), custom);
    });
}

#[gpui::test]
fn native_single_panel_title_preview_preserves_actual_title_surface(cx: &mut TestAppContext) {
    let (cx, captured) = fixture(cx, false, false, true);
    assert_native_preview(cx, captured, "dock-title-drag-source", false);
}
