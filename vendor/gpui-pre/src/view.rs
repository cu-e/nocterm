use crate::{
    AnyElement, AnyEntity, AnyWeakEntity, App, AvailableSpace, Bounds, ContentMask, Context,
    Element, ElementId, Entity, EntityId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, PaintIndex, Pixels, PrepaintStateIndex, Render, RenderOnce, Size, Style,
    StyleRefinement, TextStyle, WeakEntity,
};
use crate::{Empty, Window};
use anyhow::Result;
use collections::FxHashSet;
use refineable::Refineable;
use std::mem;
use std::{
    any::{TypeId, type_name},
    fmt,
    ops::Range,
};

/// A dynamically-typed view handle that can be downcast to a specific `Entity<V>`.
///
/// This is the type-erased counterpart to [`ViewElement`]: it holds an entity plus
/// a function pointer to its render, and is itself a [`View`], so embedding it as an
/// element goes through the same [`ViewElement`] machinery as any other view.
#[derive(Clone, Debug)]
pub struct AnyView {
    entity: AnyEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl<V: Render> From<Entity<V>> for AnyView {
    fn from(value: Entity<V>) -> Self {
        AnyView {
            entity: value.into_any(),
            render: any_view::render::<V>,
        }
    }
}

impl AnyView {
    /// Embed this view as a cached [`ViewElement`] laid out at `style`.
    ///
    /// The rendered subtree is recycled from the previous frame unless
    /// [Context::notify] was called on the backing entity since it was rendered
    /// (or [Window::refresh] is called, which ignores caching).
    pub fn cached(self, style: StyleRefinement) -> ViewElement<AnyView> {
        ViewElement::new(self).cached(style)
    }

    /// Nocterm: [`Self::cached`] for a view sized by its content along the axes `style`
    /// leaves auto. A notified view lays its content out; a frame that reuses it keeps
    /// the size its content last laid out at.
    pub fn cached_to_content(self, style: StyleRefinement) -> ViewElement<AnyView> {
        ViewElement::new(self).cached_to_content(style)
    }

    /// Nocterm: calls `on_notify` whenever the view's entity notifies, while it lives,
    /// for an owner that draws something of the view without its type, such as a
    /// title, from a cached view of its own.
    pub fn observe(
        &self,
        cx: &mut App,
        mut on_notify: impl FnMut(&mut App) + 'static,
    ) -> crate::Subscription {
        let entity = self.entity.downgrade();
        cx.new_observer(
            self.entity.entity_id(),
            Box::new(move |cx| {
                if entity.upgrade().is_some() {
                    on_notify(cx);
                    true
                } else {
                    false
                }
            }),
        )
    }

    /// Convert this to a weak handle.
    pub fn downgrade(&self) -> AnyWeakView {
        AnyWeakView {
            entity: self.entity.downgrade(),
            render: self.render,
        }
    }

    /// Convert this to a [Entity] of a specific type.
    /// If this handle does not contain a view of the specified type, returns itself in an `Err` variant.
    pub fn downcast<T: 'static>(self) -> Result<Entity<T>, Self> {
        match self.entity.downcast() {
            Ok(entity) => Ok(entity),
            Err(entity) => Err(Self {
                entity,
                render: self.render,
            }),
        }
    }

    /// Gets the [TypeId] of the underlying view.
    pub fn entity_type(&self) -> TypeId {
        self.entity.entity_type
    }

    /// The [`EntityId`] of this view.
    pub fn entity_id(&self) -> EntityId {
        self.entity.entity_id()
    }
}

impl PartialEq for AnyView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl Eq for AnyView {}

/// `AnyView` is the type-erased [`View`]: its `render` is a function pointer rather
/// than a concrete type, but it participates in the reactive graph exactly like any
/// other view via [`ViewElement`].
impl View for AnyView {
    fn entity_id(&self) -> Option<EntityId> {
        Some(self.entity.entity_id())
    }

    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        (self.render)(&self, window, cx)
    }
}

impl<V: 'static + Render> IntoElement for Entity<V> {
    type Element = ViewElement<Entity<V>>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }

    #[inline(never)]
    fn into_any_element(self) -> AnyElement {
        self.into_element().into_any()
    }
}

impl IntoElement for AnyView {
    type Element = ViewElement<AnyView>;

    fn into_element(self) -> Self::Element {
        ViewElement::new(self)
    }
}

/// A weak, dynamically-typed view handle.
pub struct AnyWeakView {
    entity: AnyWeakEntity,
    render: fn(&AnyView, &mut Window, &mut App) -> AnyElement,
}

impl AnyWeakView {
    /// Upgrade to a strong `AnyView` handle, if the view is still alive.
    pub fn upgrade(&self) -> Option<AnyView> {
        let entity = self.entity.upgrade()?;
        Some(AnyView {
            entity,
            render: self.render,
        })
    }
}

impl<V: 'static + Render> From<WeakEntity<V>> for AnyWeakView {
    fn from(view: WeakEntity<V>) -> Self {
        AnyWeakView {
            entity: view.into(),
            render: any_view::render::<V>,
        }
    }
}

impl PartialEq for AnyWeakView {
    fn eq(&self, other: &Self) -> bool {
        self.entity == other.entity
    }
}

impl std::fmt::Debug for AnyWeakView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnyWeakView")
            .field("entity_id", &self.entity.entity_id)
            .finish_non_exhaustive()
    }
}

mod any_view {
    use crate::{AnyElement, AnyView, App, IntoElement, Render, Window};

    pub(crate) fn render<V: 'static + Render>(
        view: &AnyView,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let view = view.clone().downcast::<V>().unwrap();
        // Record the view's Render type name so the accessibility debug dump can
        // attribute nodes to the view that produced them.
        #[cfg(debug_assertions)]
        window
            .a11y
            .view_type_names
            .insert(view.entity_id(), std::any::type_name::<V>());
        view.update(cx, |view, cx| view.render(window, cx).into_any_element())
    }
}

/// A renderable that participates in GPUI's reactive graph — the unifying model
/// behind [`Render`] and [`RenderOnce`].
///
/// When `entity_id()` returns `Some`, that id becomes the view's identity: it gets
/// a unique element-id space (so internal `use_state` / `.id(..)` never collide
/// across siblings) and `cx.notify()` on that entity re-renders only this view's
/// subtree. `None` behaves like a stateless component.
///
/// You rarely implement `View` directly. `Entity<T: Render>` and any `T: RenderOnce`
/// get a blanket impl below; implement it by hand only when a component needs both
/// parent-supplied props *and* a backing entity for identity.
pub trait View: 'static + Sized {
    /// This view's identity, if it has one. A view typically holds the backing
    /// entity as a field and returns its [`EntityId`] here.
    ///
    /// The id becomes this view's [`ElementId`], so two views keyed on the same
    /// entity must not be rendered at the same position in the element tree
    /// (e.g. as siblings under the same parent): their internal element state
    /// (`use_state`, scroll offsets, etc.) would silently collide. Nesting is
    /// fine — the id is scoped by the parent path.
    fn entity_id(&self) -> Option<EntityId>;

    /// Render this view into an element tree, consuming `self`.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

/// A stateless component (`RenderOnce`) is a `View` with no identity.
impl<T: RenderOnce> View for T {
    fn entity_id(&self) -> Option<EntityId> {
        None
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        RenderOnce::render(self, window, cx)
    }
}

/// An entity that renders itself (`Render`) is a `View` keyed on its own id.
impl<T: Render> View for Entity<T> {
    fn entity_id(&self) -> Option<EntityId> {
        Some(Entity::entity_id(self))
    }

    #[inline]
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.update(cx, |this, cx| {
            Render::render(this, window, cx).into_any_element()
        })
    }
}

impl<T: Render> Entity<T> {
    /// Embed this entity as a cached [`ViewElement`] laid out at `style`.
    ///
    /// The rendered subtree is reused until the entity is notified (or the
    /// cached bounds / text style change). Caching requires a definite size:
    /// a cached view is laid out from `style` and is *not* measured from its
    /// contents. Use [`ViewElement::new`] (or `.child(entity)`) for the
    /// uncached case.
    #[track_caller]
    pub fn cached(self, style: StyleRefinement) -> ViewElement<Entity<T>> {
        ViewElement::new(self).cached(style)
    }

    /// Nocterm: [`Self::cached`] for a view sized by its content along the axes `style`
    /// leaves auto. A notified view lays its content out; a frame that reuses it keeps
    /// the size its content last laid out at.
    #[track_caller]
    pub fn cached_to_content(self, style: StyleRefinement) -> ViewElement<Entity<T>> {
        ViewElement::new(self).cached_to_content(style)
    }
}

/// The element type for [`View`] implementations. Wraps a `View` and hooks it
/// into layout, prepaint, and paint. Constructed via [`ViewElement::new`].
#[doc(hidden)]
pub struct ViewElement<V: View> {
    view: Option<V>,
    entity_id: Option<EntityId>,
    cached_style: Option<StyleRefinement>,
    /// Nocterm: the cached view is sized by its content along the auto axes of its style.
    to_content: bool,
    #[cfg(debug_assertions)]
    source: &'static core::panic::Location<'static>,
}

impl<V: View> ViewElement<V> {
    /// Wrap a [`View`] as an element.
    #[track_caller]
    pub fn new(view: V) -> Self {
        let entity_id = view.entity_id();
        ViewElement {
            entity_id,
            cached_style: None,
            to_content: false,
            view: Some(view),
            #[cfg(debug_assertions)]
            source: core::panic::Location::caller(),
        }
    }

    /// Enable caching of this view's rendered subtree, laid out at `style`.
    /// The composer supplies the layout style because caching skips rendering
    /// the contents to measure them.
    ///
    /// Crate-private on purpose: caching is only sound for entity-backed views,
    /// where [`Context::notify`] is the contract that busts the cache. A stateless
    /// view has no such contract, so a frozen subtree could never be invalidated.
    /// Reach this through [`Entity::cached`] or [`AnyView::cached`], which are
    /// entity-backed by construction.
    pub(crate) fn cached(mut self, style: StyleRefinement) -> Self {
        self.cached_style = Some(style);
        self
    }

    /// Nocterm: [`Self::cached`], sized by the content along the auto axes of `style`.
    pub(crate) fn cached_to_content(mut self, style: StyleRefinement) -> Self {
        self.cached_style = Some(style);
        self.to_content = true;
        self
    }

    /// Nocterm: the axes a view cached to its content takes from its content.
    fn content_axes(&self) -> Option<Size<bool>> {
        let style = self.cached_style.as_ref().filter(|_| self.to_content)?;
        Some(Size {
            width: style.size.width.is_none(),
            height: style.size.height.is_none(),
        })
    }
}

/// The layout state of a [`ViewElement`].
#[doc(hidden)]
pub struct ViewLayout {
    element: Option<AnyElement>,
    /// Nocterm: set when a view cached to its content laid its content out this frame,
    /// with the entities its render read.
    fresh: Option<FxHashSet<EntityId>>,
}

impl<V: View> IntoElement for ViewElement<V> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct ViewElementState {
    /// Nocterm: a view cached to its content was drawn at a size its content does not
    /// fit, so the next frame lays its content out again.
    misfit: bool,
    /// Nocterm: the draw that last painted this view. Its ranges index the frame
    /// drawn last only if that draw painted it; a reused ancestor copies a nested
    /// view's output without visiting it, which leaves its ranges stale.
    painted_in: u64,
    /// Nocterm: the cached views prepainted inside this one, at any depth. Reusing this
    /// view copies their output without visiting them, so their ranges move with it.
    nested: Vec<GlobalElementId>,
    prepaint_range: Range<PrepaintStateIndex>,
    paint_range: Range<PaintIndex>,
    cache_key: ViewElementCacheKey,
    accessed_entities: FxHashSet<EntityId>,
}

struct ViewElementCacheKey {
    bounds: Bounds<Pixels>,
    content_mask: ContentMask<Pixels>,
    text_style: TextStyle,
}

impl<V: View> Element for ViewElement<V> {
    type RequestLayoutState = ViewLayout;
    type PrepaintState = Option<AnyElement>;

    fn id(&self) -> Option<ElementId> {
        self.entity_id.map(ElementId::View)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        #[cfg(debug_assertions)]
        return Some(self.source);

        #[cfg(not(debug_assertions))]
        return None;
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(entity_id) = self.entity_id {
            // Stateful path: create a reactive boundary.
            let view = &mut self.view;
            request_layout_view(
                entity_id,
                id,
                self.cached_style.as_ref(),
                self.to_content,
                window,
                cx,
                &mut |window, cx| view.take().unwrap().render(window, cx).into_any_element(),
            )
        } else {
            // Stateless path: isolate subtree via type name (no entity identity).
            let (layout_id, element) =
                request_layout_component(type_name::<V>(), window, cx, &mut |window, cx| {
                    self.view
                        .take()
                        .unwrap()
                        .render(window, cx)
                        .into_any_element()
                });
            (
                layout_id,
                ViewLayout {
                    element,
                    fresh: None,
                },
            )
        }
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let content_axes = self.content_axes();
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            prepaint_view(
                entity_id,
                global_id,
                bounds,
                (&mut layout.element, layout.fresh.take(), content_axes),
                window,
                cx,
                &mut |window, cx| {
                    self.view
                        .take()
                        .unwrap()
                        .render(window, cx)
                        .into_any_element()
                },
            )
        } else {
            // Stateless path: just prepaint the element.
            prepaint_component(type_name::<V>(), &mut layout.element, window, cx)
        }
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        element: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(entity_id) = self.entity_id {
            // Stateful path.
            paint_view(
                entity_id,
                self.cached_style.is_some(),
                global_id,
                element,
                window,
                cx,
            );
        } else {
            // Stateless path: just paint the element.
            paint_component(std::any::type_name::<V>(), element, window, cx);
        }
    }
}

/// A view that renders nothing
pub struct EmptyView;

impl Render for EmptyView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[inline(never)]
fn request_layout_view(
    entity_id: EntityId,
    global_id: Option<&GlobalElementId>,
    cached_style: Option<&StyleRefinement>,
    to_content: bool,
    window: &mut Window,
    cx: &mut App,
    render: &mut dyn FnMut(&mut Window, &mut App) -> AnyElement,
) -> (LayoutId, ViewLayout) {
    window.with_rendered_view(entity_id, |window| {
        let caching_disabled = window.is_inspector_picking(cx);
        match cached_style {
            Some(style) if !caching_disabled => {
                let mut root_style = Style::default();
                root_style.refine(style);
                if to_content {
                    match reusable_size(entity_id, global_id, window) {
                        // Reused: the auto axes keep the size the content last took.
                        Some(size) => {
                            if style.size.width.is_none() {
                                root_style.size.width = size.width.into();
                            }
                            if style.size.height.is_none() {
                                root_style.size.height = size.height.into();
                            }
                        }
                        // Drawn again: the content lays itself out, as an uncached
                        // view's does, and its size is what later frames keep.
                        None => {
                            let (mut element, read) =
                                cx.detect_accessed_entities(|cx| render(window, cx));
                            let layout_id = element.request_layout(window, cx);
                            return (
                                layout_id,
                                ViewLayout {
                                    element: Some(element),
                                    fresh: Some(read),
                                },
                            );
                        }
                    }
                }
                let layout_id = window.request_layout(root_style, None, cx);
                (
                    layout_id,
                    ViewLayout {
                        element: None,
                        fresh: None,
                    },
                )
            }
            _ => {
                let mut element = render(window, cx);
                let layout_id = element.request_layout(window, cx);
                (
                    layout_id,
                    ViewLayout {
                        element: Some(element),
                        fresh: None,
                    },
                )
            }
        }
    })
}

/// Nocterm: the size a view cached to its content keeps in a frame that can reuse it:
/// the last draw painted it at a size its content fit, nothing notified it since and
/// the window is not refreshing.
fn reusable_size(
    entity_id: EntityId,
    global_id: Option<&GlobalElementId>,
    window: &mut Window,
) -> Option<Size<Pixels>> {
    if window.dirty_views.contains(&entity_id) || window.refreshing {
        return None;
    }
    let generation = window.draw_generation;
    let mut size = None;
    window.update_unvisited_element_state(global_id?, |state: &mut ViewElementState| {
        if state.painted_in + 1 == generation && !state.misfit {
            size = Some(state.cache_key.bounds.size);
        }
    });
    size
}

#[inline(never)]
fn request_layout_component(
    name: &'static str,
    window: &mut Window,
    cx: &mut App,
    render: &mut dyn FnMut(&mut Window, &mut App) -> AnyElement,
) -> (LayoutId, Option<AnyElement>) {
    window.with_id(ElementId::from(name), |window| {
        let mut element = render(window, cx);
        let layout_id = element.request_layout(window, cx);
        (layout_id, Some(element))
    })
}

#[inline(never)]
fn prepaint_view(
    entity_id: EntityId,
    global_id: Option<&GlobalElementId>,
    bounds: Bounds<Pixels>,
    (element, fresh, content_axes): (
        &mut Option<AnyElement>,
        Option<FxHashSet<EntityId>>,
        Option<Size<bool>>,
    ),
    window: &mut Window,
    cx: &mut App,
    render: &mut dyn FnMut(&mut Window, &mut App) -> AnyElement,
) -> Option<AnyElement> {
    window.set_view_id(entity_id);
    window.with_rendered_view(entity_id, |window| {
        if let Some(element) = element.take() {
            return match fresh {
                Some(read) => Some(prepaint_laid_out(element, read, global_id, bounds, window, cx)),
                None => {
                    let mut element = element;
                    element.prepaint(window, cx);
                    Some(element)
                }
            };
        }

        window.with_element_state::<ViewElementState, _>(
            global_id.unwrap(),
            |element_state, window| {
                let content_mask = window.content_mask();
                let text_style = window.text_style();

                if let Some(mut element_state) = element_state
                    && element_state.cache_key.bounds == bounds
                    && element_state.cache_key.content_mask == content_mask
                    && element_state.cache_key.text_style == text_style
                    && !window.dirty_views.contains(&entity_id)
                    && !window.refreshing
                    && element_state.painted_in + 1 == window.draw_generation
                {
                    let prepaint_start = window.prepaint_index();
                    window.reuse_prepaint(element_state.prepaint_range.clone());
                    cx.entities
                        .extend_accessed(&element_state.accessed_entities);
                    let prepaint_end = window.prepaint_index();
                    let previous = mem::replace(
                        &mut element_state.prepaint_range,
                        prepaint_start..prepaint_end,
                    );
                    for nested in &element_state.nested {
                        window.update_unvisited_element_state(
                            nested,
                            |state: &mut ViewElementState| {
                                state.prepaint_range = state
                                    .prepaint_range
                                    .start
                                    .shifted(&previous.start, &element_state.prepaint_range.start)
                                    ..state.prepaint_range.end.shifted(
                                        &previous.start,
                                        &element_state.prepaint_range.start,
                                    );
                            },
                        );
                    }
                    report_nested(window, global_id.unwrap(), &element_state.nested);

                    return (None, element_state);
                }

                // Nocterm: a cached view drawn again keeps the window's refresh state, so
                // the cached views nested in it are reused unless they or what they read
                // changed. Forcing them to draw (upstream) made one notified leaf redraw
                // every cached view under each of its cached ancestors.
                let prepaint_start = window.prepaint_index();
                window.cached_view_scopes.push(Vec::new());
                let (element, accessed_entities) = cx.detect_accessed_entities(|cx| {
                    let mut element = render(window, cx);
                    let size = element.layout_as_root(
                        Size::<AvailableSpace>::from(bounds.size),
                        window,
                        cx,
                    );
                    element.prepaint_at(bounds.origin, window, cx);
                    (element, size)
                });
                let (element, size) = element;
                // Nocterm: a view cached to its content drawn at a kept size its content
                // no longer fits lays its content out again next frame.
                let misfit = content_axes.is_some_and(|axes| {
                    (axes.width && size.width != bounds.size.width)
                        || (axes.height && size.height != bounds.size.height)
                });
                if misfit {
                    window.request_animation_frame();
                }
                let nested = window.cached_view_scopes.pop().unwrap_or_default();
                report_nested(window, global_id.unwrap(), &nested);

                let prepaint_end = window.prepaint_index();

                (
                    Some(element),
                    ViewElementState {
                        misfit,
                        painted_in: 0,
                        nested,
                        accessed_entities,
                        prepaint_range: prepaint_start..prepaint_end,
                        paint_range: PaintIndex::default()..PaintIndex::default(),
                        cache_key: ViewElementCacheKey {
                            bounds,
                            content_mask,
                            text_style,
                        },
                    },
                )
            },
        )
    })
}

/// Nocterm: prepaints a view cached to its content that laid its content out this
/// frame, recording what the next frames reuse.
fn prepaint_laid_out(
    mut element: AnyElement,
    read: FxHashSet<EntityId>,
    global_id: Option<&GlobalElementId>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let global_id = global_id.unwrap();
    window.with_element_state::<ViewElementState, _>(global_id, |_, window| {
        let content_mask = window.content_mask();
        let text_style = window.text_style();
        let prepaint_start = window.prepaint_index();
        window.cached_view_scopes.push(Vec::new());
        let ((), mut accessed_entities) = cx.detect_accessed_entities(|cx| {
            element.prepaint(window, cx);
        });
        accessed_entities.extend(read);
        let nested = window.cached_view_scopes.pop().unwrap_or_default();
        report_nested(window, global_id, &nested);
        let prepaint_end = window.prepaint_index();
        (
            element,
            ViewElementState {
                misfit: false,
                painted_in: 0,
                nested,
                accessed_entities,
                prepaint_range: prepaint_start..prepaint_end,
                paint_range: PaintIndex::default()..PaintIndex::default(),
                cache_key: ViewElementCacheKey {
                    bounds,
                    content_mask,
                    text_style,
                },
            },
        )
    })
}

/// Nocterm: records a prepainted cached view and those nested in it in the enclosing
/// cached view's scope, if any.
fn report_nested(window: &mut Window, global_id: &GlobalElementId, nested: &[GlobalElementId]) {
    if let Some(scope) = window.cached_view_scopes.last_mut() {
        scope.push(global_id.clone());
        scope.extend(nested.iter().cloned());
    }
}

#[inline(never)]
fn prepaint_component(
    name: &'static str,
    element: &mut Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    window.with_id(ElementId::from(name), |window| {
        element.as_mut().unwrap().prepaint(window, cx);
    });
    Some(element.take().unwrap())
}

#[inline(never)]
fn paint_view(
    entity_id: EntityId,
    cached: bool,
    global_id: Option<&GlobalElementId>,
    element: &mut Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) {
    window.with_rendered_view(entity_id, |window| {
        let caching_disabled = window.is_inspector_picking(cx);
        if cached && !caching_disabled {
            window.with_element_state::<ViewElementState, _>(
                global_id.unwrap(),
                |element_state, window| {
                    let mut element_state = element_state.unwrap();

                    let paint_start = window.paint_index();

                    let reused = element.is_none();
                    if let Some(element) = element {
                        element.paint(window, cx);
                    } else {
                        window.reuse_paint(element_state.paint_range.clone());
                    }

                    let paint_end = window.paint_index();
                    let previous =
                        mem::replace(&mut element_state.paint_range, paint_start..paint_end);
                    element_state.painted_in = window.draw_generation;
                    if reused {
                        let generation = window.draw_generation;
                        for nested in &element_state.nested {
                            window.update_unvisited_element_state(
                                nested,
                                |state: &mut ViewElementState| {
                                    state.paint_range = state
                                        .paint_range
                                        .start
                                        .shifted(&previous.start, &element_state.paint_range.start)
                                        ..state.paint_range.end.shifted(
                                            &previous.start,
                                            &element_state.paint_range.start,
                                        );
                                    state.painted_in = generation;
                                },
                            );
                        }
                    }

                    ((), element_state)
                },
            )
        } else {
            element.as_mut().unwrap().paint(window, cx);
        }
    });
}

#[inline(never)]
fn paint_component(
    name: &'static str,
    element: &mut Option<AnyElement>,
    window: &mut Window,
    cx: &mut App,
) {
    window.with_id(ElementId::Name(name.into()), |window| {
        element.as_mut().unwrap().paint(window, cx);
    });
}
