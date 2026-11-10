//! The entity that holds a plugin's view.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use gpui::{
    AnyElement, AnyView, App, Context, FocusHandle, IntoElement, Render, SharedString, Window,
};
use nocterm_plugin_api::{Event, Glyph, NodeKind, View, ViewError};

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
/// Draws a glyph in the colour and size of the text around it.
pub type IconFn = Rc<dyn Fn(Glyph) -> AnyElement>;
/// Builds the tooltip that names an icon button's action, as the application draws its own.
pub type TooltipFn = Rc<dyn Fn(SharedString, &mut Window, &mut App) -> AnyView>;
/// Draws the view again, after the user folded or unfolded one of its sections.
pub type RedrawFn = Rc<dyn Fn(&mut App)>;

/// Whether each foldable section of the view is folded, by its key. The user folds a section;
/// the plugin is not asked.
pub(crate) type Folds = Rc<RefCell<BTreeMap<String, bool>>>;

/// The application's side of a view.
#[derive(Clone)]
pub struct Hooks {
    pub icon: IconFn,
    pub event: EventFn,
    pub handles: FocusFn,
    pub tooltip: TooltipFn,
    pub redraw: RedrawFn,
    /// The font family of code blocks: the application's monospace font.
    pub mono: SharedString,
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
    folds: Folds,
    dev: bool,
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
            folds: Folds::default(),
            dev: false,
        }
    }

    /// Marks the view as one from a developer session: the label gains a DEV suffix and a bar
    /// item gains a DEV chip. The caller redraws the entity afterwards.
    pub fn mark_dev(&mut self) {
        self.dev = true;
    }

    /// Shows a view. One the host refuses becomes the fault chip and the error is returned.
    /// The caller redraws the entity afterwards.
    pub fn show(&mut self, view: View) -> Result<(), ViewError> {
        match Tree::new(view) {
            Ok(tree) => {
                self.keep_folds(&tree);
                self.content = Content::View(tree);
                Ok(())
            }
            Err(error) => {
                self.content = Content::Fault;
                Err(error)
            }
        }
    }

    /// Keeps the folds of the sections the new view still has, and starts each new one as the
    /// plugin asks; the map never holds more keys than the view.
    fn keep_folds(&self, tree: &Tree) {
        let mut folds = self.folds.borrow_mut();
        let mut kept = BTreeMap::new();
        for node in tree.nodes() {
            if let (Some(key), NodeKind::Section { folded, .. }) = (&node.key, &node.kind) {
                let folded = folds.get(key).copied().unwrap_or(*folded);
                kept.insert(key.clone(), folded);
            }
        }
        *folds = kept;
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

    /// "Plugin · ⟨name⟩", with " · DEV" in a developer session.
    pub fn label(&self) -> SharedString {
        label(&self.name, self.dev)
    }

    /// Whether the view is a developer session's.
    pub fn is_dev(&self) -> bool {
        self.dev
    }

    fn body(&self, cx: &mut App) -> AnyElement {
        match &self.content {
            Content::Empty => gpui::Empty.into_any_element(),
            Content::Fault => fault_chip(&self.style).into_any_element(),
            Content::View(tree) => {
                Painter::new(tree, &self.style, &self.hooks, &self.folds, self.kind, cx).roots()
            }
        }
    }
}

impl Render for PluginView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(cx);
        framed(self.kind, &self.style, self.dev, body)
    }
}
