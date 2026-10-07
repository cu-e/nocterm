// Added by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Observe the native tab layout box, without adding a canvas in its padding box.
use super::BoundsObserver;
use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

pub(super) struct ObservedTab {
    tab: AnyElement,
    observer: Option<BoundsObserver>,
}

impl ObservedTab {
    pub(super) fn new(tab: AnyElement, observer: BoundsObserver) -> Self {
        Self {
            tab,
            observer: Some(observer),
        }
    }
}

impl IntoElement for ObservedTab {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ObservedTab {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.tab.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tab.prepaint(window, cx);
        if let Some(observer) = self.observer.take() {
            observer(bounds, window, cx);
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.tab.paint(window, cx);
    }
}
