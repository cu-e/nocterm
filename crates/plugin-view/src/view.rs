//! The entity that holds a plugin's view.

use std::rc::Rc;

use gpui::{AnyElement, App, Context, FocusHandle, IntoElement, Render, SharedString, Window};
use nocterm_plugin_api::{Event, PluginIcon, View, ViewError};

use crate::{
    FrameKind, ViewStyle,
    frame::{fault_chip, framed, label},
    paint::Painter,
    tree::Tree,
};

/// How a control was operated. It never carries a key, a character or a modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A click or a tap.
    Pointer,
    /// Enter or Space on a control that has focus.
    Key,
}

/// What a button or toggle reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interaction {
    pub event: Event,
    pub origin: Origin,
}

/// Receives what the user does to a control of the view.
pub type EventFn = Rc<dyn Fn(Interaction, &mut Window, &mut App)>;
/// The focus handle of the `n`th control of the view. The application keeps the handles, so
/// this crate never makes or moves focus.
pub type FocusFn = Rc<dyn Fn(usize, &mut App) -> FocusHandle>;
/// Draws a plugin icon in the colour of the text around it.
pub type IconFn = Rc<dyn Fn(PluginIcon) -> AnyElement>;

/// The application's side of a view.
#[derive(Clone)]
pub struct Hooks {
    pub icon: IconFn,
    pub event: EventFn,
    pub handles: FocusFn,
}

enum Content {
    Empty,
    View(Tree),
    Fault,
}

/// A plugin's view inside its frame.
pub struct PluginView {
    name: SharedString,
    kind: FrameKind,
    style: ViewStyle,
    hooks: Hooks,
    content: Content,
}

impl PluginView {
    pub fn new(
        name: impl Into<SharedString>,
        kind: FrameKind,
        style: ViewStyle,
        hooks: Hooks,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            style,
            hooks,
            content: Content::Empty,
        }
    }

    /// Shows a view. One the host refuses becomes the fault chip and the error is returned.
    /// The caller redraws the entity afterwards.
    pub fn show(&mut self, view: View) -> Result<(), ViewError> {
        match Tree::new(view) {
            Ok(tree) => {
                self.content = Content::View(tree);
                Ok(())
            }
            Err(error) => {
                self.content = Content::Fault;
                Err(error)
            }
        }
    }

    /// Shows the fault chip, for a plugin that failed to answer.
    pub fn fault(&mut self) {
        self.content = Content::Fault;
    }

    /// Draws with other colours.
    pub fn restyle(&mut self, style: ViewStyle) {
        self.style = style;
    }

    pub fn is_fault(&self) -> bool {
        matches!(self.content, Content::Fault)
    }

    /// "Plugin · ⟨name⟩".
    pub fn label(&self) -> SharedString {
        label(&self.name)
    }

    fn body(&self, cx: &mut App) -> AnyElement {
        match &self.content {
            Content::Empty => gpui::Empty.into_any_element(),
            Content::Fault => fault_chip(&self.style).into_any_element(),
            Content::View(tree) => {
                Painter::new(tree, &self.style, &self.hooks, self.kind, cx).roots()
            }
        }
    }
}

impl Render for PluginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(cx);
        framed(self.kind, &self.style, self.label(), body)
    }
}
