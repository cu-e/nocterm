//! Passive source content drawn at its measured size in GPUI's separate drag root.
use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    AnyElement, App, Bounds, Context, Pixels, Render, Size, TextStyle, Window,
    base::TestSupportExt as _, div, prelude::*,
};

type Content = dyn Fn(&mut Window, &mut App) -> AnyElement;

#[derive(Clone)]
struct Appearance {
    size: Size<Pixels>,
    typography: TextStyle,
}

/// Captured during prepaint, after layout and inherited typography are resolved.
#[derive(Clone, Default)]
pub struct DragSource(Rc<RefCell<Option<Appearance>>>);

impl DragSource {
    pub fn capture(&self, bounds: Bounds<Pixels>, window: &Window) {
        *self.0.borrow_mut() = Some(Appearance {
            size: bounds.size,
            typography: window.text_style(),
        });
    }

    pub fn preview(
        &self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> DragPreview {
        let appearance = self.0.borrow().clone().expect("a drag source was painted");
        DragPreview {
            size: appearance.size,
            typography: appearance.typography,
            content: Rc::new(content),
        }
    }
}

pub struct DragPreview {
    size: Size<Pixels>,
    typography: TextStyle,
    content: Rc<Content>,
}

impl Render for DragPreview {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .id("drag-preview")
            .test_support()
            .w(self.size.width)
            .h(self.size.height)
            .font_family(self.typography.font_family.clone())
            .font_features(self.typography.font_features.clone())
            .text_size(self.typography.font_size)
            .line_height(self.typography.line_height)
            .font_weight(self.typography.font_weight)
            .text_color(self.typography.color);
        root.text_style().font_style = Some(self.typography.font_style);
        root.child((self.content)(window, cx))
    }
}

#[cfg(test)]
mod tests;
