// Added by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! The dock drag root reuses the source tab/title presentation at painted dimensions.
use super::{CLOSE_BUTTON_SELECTOR, PanelHandle, panel_title};
use crate::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    floating::FloatingCards,
    tab::Tab,
};
use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, Entity, Hsla, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Point, Render, Size, Stateful, Styled as _, TextStyle,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_base::{
    TestSupportExt as _,
    dock::{DragPanel, PanelView, TabGroupContext},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Clone)]
struct Appearance {
    size: Size<Pixels>,
    typography: TextStyle,
}

#[derive(Clone, Default)]
pub(super) struct PaintedSource(Rc<RefCell<Option<Appearance>>>);

impl PaintedSource {
    pub fn capture(&self, bounds: Bounds<Pixels>, window: &Window) {
        *self.0.borrow_mut() = Some(Appearance {
            size: bounds.size,
            typography: window.text_style(),
        });
    }

    pub fn start(
        &self,
        drag: &DragPanel,
        offset: Point<Pixels>,
        content: PreviewContent,
        cx: &mut App,
    ) -> Entity<DragPanelPreview> {
        cx.stop_propagation();
        let appearance = self
            .0
            .borrow()
            .clone()
            .expect("a dock drag source was painted");
        drag.set_drag_offset(offset);
        drag.set_preview_size(appearance.size);
        let background = content.background(cx);
        cx.new(|_| DragPanelPreview {
            appearance,
            content,
            background,
        })
    }
}

#[derive(Clone)]
pub(super) struct TabVisual {
    pub panel: Arc<dyn PanelView>,
    pub ix: usize,
    pub has_leading: bool,
    pub floating: bool,
    pub selected: bool,
    pub accent: Option<Hsla>,
    pub close: bool,
}

impl TabVisual {
    pub fn render(
        &self,
        group: Option<&TabGroupContext>,
        window: &mut Window,
        cx: &mut App,
    ) -> Tab {
        Tab::new()
            .ix(self.ix)
            .tab_bar_prefix(self.has_leading)
            .debug_selector(|| format!("dock-tab-drag-source-{}", self.ix))
            .when(self.floating, |tab| tab.floating())
            .selected(self.selected)
            .when_some(self.accent, |tab, accent| tab.accent(accent))
            .map(
                |tab| match PanelHandle::of(&self.panel).and_then(|handle| handle.tab_name(cx)) {
                    Some(name) => tab.child(name),
                    None => tab.child(panel_title(&self.panel, window, cx)),
                },
            )
            .when(self.close, |tab| {
                let close = Button::new(("close-tab", self.ix))
                    .icon(IconName::Close)
                    .xsmall()
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .foreground(cx.theme().secondary_foreground)
                            .hover(*cx.theme().tokens.secondary_hover)
                            .active(*cx.theme().tokens.secondary_active),
                    )
                    .ml(-px(8.))
                    .mr_2()
                    .tab_stop(false)
                    .debug_selector(|| CLOSE_BUTTON_SELECTOR.to_string())
                    .when_some(group.cloned(), |button, group| {
                        let panel_id = self.panel.panel_id(cx);
                        button.on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            group.close(panel_id, window, cx);
                        })
                    });
                tab.suffix(close)
            })
    }
}

pub(super) fn title_visual(
    panel: &Arc<dyn PanelView>,
    window: &mut Window,
    cx: &mut App,
) -> Stateful<Div> {
    div()
        .id("tab")
        .debug_selector(|| "dock-title-drag-source".to_string())
        .flex_1()
        .min_w_16()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .child(panel_title(panel, window, cx))
}

pub(super) enum PreviewContent {
    Tab(TabVisual),
    Title(Arc<dyn PanelView>),
}

impl PreviewContent {
    /// Transparent source content is painted over its enclosing bar or card.
    fn background(&self, cx: &App) -> Hsla {
        let surface = FloatingCards::get(cx).map(|cards| cards.surface);
        match self {
            Self::Tab(tab) if !tab.floating => *cx.theme().tokens.tab_bar,
            Self::Tab(_) => surface.unwrap_or(*cx.theme().tokens.tab_bar),
            Self::Title(panel) => PanelHandle::of(panel)
                .and_then(|handle| handle.title_style(cx))
                .map(|style| style.background)
                .or(surface)
                .unwrap_or(*cx.theme().tokens.background),
        }
    }
}

/// The passive appearance half of the toolkit's unchanged DragPanel payload.
pub struct DragPanelPreview {
    appearance: Appearance,
    content: PreviewContent,
    background: Hsla,
}

impl Render for DragPanelPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let size = self.appearance.size;
        let typography = &self.appearance.typography;
        let mut root = div()
            .id("drag-panel")
            .debug_selector(|| "dock-native-drag-preview".into())
            .test_support()
            .w(size.width)
            .h(size.height)
            .bg(self.background)
            .font_family(typography.font_family.clone())
            .font_features(typography.font_features.clone())
            .text_size(typography.font_size)
            .line_height(typography.line_height)
            .font_weight(typography.font_weight)
            .text_color(typography.color);
        root.text_style().font_style = Some(typography.font_style);
        let content: AnyElement = match &self.content {
            // Outside TabBar there is no sliding indicator: Tab paints its selected fill itself.
            PreviewContent::Tab(tab) => tab
                .render(None, window, cx)
                .w(size.width)
                .h(size.height)
                .into_any_element(),
            PreviewContent::Title(panel) => title_visual(panel, window, cx)
                .w_full()
                .h_full()
                .into_any_element(),
        };
        root.child(content)
    }
}

#[cfg(test)]
mod tests;
