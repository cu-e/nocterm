//! A sectioned page creates its sections when it first draws them, passes every hide and close
//! on once, and scrolls to the section an anchor names.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, ScrollHandle,
    StatefulInteractiveElement as _, Styled as _, TestAppContext, Window, WindowOptions,
    component::v_flex, div, px, size, test::TestWindowExt as _,
};

use crate::{
    DesignTokens, EmbeddedPageSpec, SettingsPage, SettingsPageContext, SettingsPageFeedback,
    SettingsPageSpec,
};

/// What a page was told, shared with the test.
#[derive(Clone, Default)]
struct Told {
    created: Rc<Cell<usize>>,
    flushed: Rc<Cell<usize>>,
    hidden: Rc<Cell<usize>>,
    closed: Rc<Cell<usize>>,
    revealed: Rc<RefCell<Vec<String>>>,
}

fn bump(counter: &Cell<usize>) {
    counter.set(counter.get() + 1);
}

struct Probe {
    told: Told,
    anchors: &'static [&'static str],
    focus: FocusHandle,
}

impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().h(px(400.)).child("content")
    }
}

impl SettingsPage for Probe {
    fn flush(&mut self, _: &mut Window, _: &mut Context<Self>) {
        bump(&self.told.flushed);
    }

    fn on_deactivate(&mut self, _: &mut Window, _: &mut Context<Self>) {
        bump(&self.told.hidden);
    }

    fn on_close(&mut self, _: &mut Window, _: &mut Context<Self>) {
        bump(&self.told.closed);
    }

    fn reveal(&mut self, anchor: &str, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.told.revealed.borrow_mut().push(anchor.to_owned());
        self.anchors.contains(&anchor)
    }
}

fn probe(told: &Told, anchors: &'static [&'static str], cx: &mut App) -> Entity<Probe> {
    bump(&told.created);
    cx.new(|cx| Probe {
        told: told.clone(),
        anchors,
        focus: cx.focus_handle(),
    })
}

/// The window shows the page in a short scrolling column, as the Settings host does.
struct Host {
    page: AnyView,
    scroll: ScrollHandle,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("host-scroll")
            .h(px(300.))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(self.page.clone())
    }
}

struct Fixture {
    window: AnyWindowHandle,
    page: Box<dyn crate::SettingsPageHandle>,
    scroll: ScrollHandle,
    primary: Told,
    layout: Told,
}

fn open(cx: &mut TestAppContext) -> Fixture {
    let (primary, layout) = (Told::default(), Told::default());
    let scroll = ScrollHandle::new();
    let (primary_told, layout_told, host_scroll) =
        (primary.clone(), layout.clone(), scroll.clone());
    let (window, page) = cx.update(|cx| {
        gpui_kit::init(cx);
        let ready = crate::init(
            DesignTokens::builtin(),
            nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx),
            cx,
        );
        let spec = SettingsPageSpec::new("appearance", "Appearance", move |_, cx| {
            probe(&primary_told, &[], cx)
        })
        .with_sections(vec![EmbeddedPageSpec::new(
            "layout",
            "Status bar and panels",
            move |_, cx| probe(&layout_told, &["x"], cx),
        )]);
        let context =
            SettingsPageContext::new(ready.settings().clone(), SettingsPageFeedback::new(cx))
                .with_scroll(host_scroll);
        let made = Rc::new(RefCell::new(None));
        let kept = made.clone();
        let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            let page = spec.create(context.clone(), window, cx);
            let host = Host {
                page: page.view(),
                scroll: context.scroll().clone(),
            };
            *kept.borrow_mut() = Some(page);
            cx.new(|_| host)
        })
        .unwrap();
        let page = made.take().unwrap();
        (window, page)
    });
    cx.update_window(window, |_, window, _| window.activate_window())
        .unwrap();
    cx.simulate_window_resize(window, size(px(800.), px(300.)));
    cx.run_until_parked();
    Fixture {
        window,
        page,
        scroll,
        primary,
        layout,
    }
}

fn frames(fixture: &Fixture, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.update_window(fixture.window, |_, window, cx| {
            window.render_frame(cx);
            window.simulate_next_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn a_section_is_created_when_the_page_first_draws_it(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frames(&fixture, cx);
    assert_eq!(fixture.primary.created.get(), 1);
    assert_eq!(fixture.layout.created.get(), 1);
}

#[gpui_kit::test]
fn flush_hide_and_close_reach_the_page_and_each_section_once(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frames(&fixture, cx);
    cx.update_window(fixture.window, |_, window, cx| {
        fixture.page.flush(window, cx);
        fixture.page.deactivate(window, cx);
        fixture.page.close(window, cx);
    })
    .unwrap();
    for told in [&fixture.primary, &fixture.layout] {
        assert_eq!(told.flushed.get(), 1);
        assert_eq!(told.hidden.get(), 1);
        assert_eq!(told.closed.get(), 1);
    }
}

#[gpui_kit::test]
fn an_anchor_with_the_key_of_a_section_scrolls_to_it_and_passes_the_rest_on(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    frames(&fixture, cx);
    assert_eq!(fixture.scroll.offset().y, px(0.));
    let known = cx
        .update_window(fixture.window, |_, window, cx| {
            fixture.page.reveal("layout/x", window, cx)
        })
        .unwrap();
    frames(&fixture, cx);
    assert!(known);
    assert_eq!(*fixture.layout.revealed.borrow(), vec!["x".to_owned()]);
    assert!(
        fixture.scroll.offset().y < px(0.),
        "scrolled to the section"
    );
}

#[gpui_kit::test]
fn the_key_alone_reveals_the_section_and_an_unknown_inner_anchor_is_refused(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    frames(&fixture, cx);
    let (whole, unknown) = cx
        .update_window(fixture.window, |_, window, cx| {
            (
                fixture.page.reveal("layout", window, cx),
                fixture.page.reveal("layout/nope", window, cx),
            )
        })
        .unwrap();
    assert!(whole);
    assert!(!unknown);
}

#[gpui_kit::test]
fn an_anchor_no_section_owns_goes_to_the_page_itself(cx: &mut TestAppContext) {
    let fixture = open(cx);
    frames(&fixture, cx);
    let known = cx
        .update_window(fixture.window, |_, window, cx| {
            fixture.page.reveal("elsewhere", window, cx)
        })
        .unwrap();
    assert!(!known);
    assert_eq!(
        *fixture.primary.revealed.borrow(),
        vec!["elsewhere".to_owned()]
    );
    assert!(fixture.layout.revealed.borrow().is_empty());
}
