// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Toolbar and single-panel header presentation.
use super::*;

impl TabGroupSkin {
    /// The trailing controls: the panel's own buttons, the zoom affordance,
    /// and the ellipsis menu.
    pub(super) fn render_toolbar(
        &self,
        group: &TabGroupContext,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        if group.is_collapsed() {
            return div();
        }

        let zoomed = group.is_zoomed();
        let handle = group.active_panel().and_then(PanelHandle::of);
        let control = zoom_control(group, cx);
        let toolbar_zoom = control.is_some_and(|control| control.toolbar_visible());
        let menu_zoom = control.is_some_and(|control| control.menu_visible());
        let closable = group.is_closable();
        let buttons = handle.and_then(|handle| handle.toolbar_buttons(window, cx));
        let panel = handle.map(|handle| handle.panel());

        h_flex()
            .gap_1()
            .occlude()
            .when_some(buttons, |this, buttons| {
                this.children(
                    buttons
                        .into_iter()
                        .map(|button| button.xsmall().ghost().tab_stop(false)),
                )
            })
            .when_some(
                match (zoomed, toolbar_zoom) {
                    (true, _) => Some(("zoom-out", IconName::Minimize, t!("Dock.Zoom Out"))),
                    (false, true) => Some(("zoom-in", IconName::Maximize, t!("Dock.Zoom In"))),
                    (false, false) => None,
                },
                |this, (id, icon, tooltip)| {
                    this.child(
                        Button::new(id)
                            .icon(icon)
                            .xsmall()
                            .ghost()
                            .tab_stop(false)
                            .tooltip_with_action(tooltip, &ToggleZoom, None)
                            .selected(zoomed)
                            // Whether this button was drawn is the whole of
                            // the `zoom_control` decision, and there is no
                            // other way to ask a drawn tree about it. A no-op
                            // outside test builds; see `debug_selector`.
                            .debug_selector(|| ZOOM_CONTROL_SELECTOR.to_string())
                            .on_click({
                                let group = group.clone();
                                move |_, window, cx| group.toggle_zoom(window, cx)
                            }),
                    )
                },
            )
            .when(
                handle.is_none_or(|handle| handle.menu_visible(cx)),
                |toolbar| {
                    toolbar.child(
                        Button::new("menu")
                            .icon(IconName::Ellipsis)
                            .xsmall()
                            .ghost()
                            .tab_stop(false)
                            .dropdown_menu(move |menu, window, cx| {
                                menu.when_some(panel.clone(), |menu, panel| {
                                    panel.dropdown_menu(menu, window, cx)
                                })
                                .separator()
                                .menu_with_disabled(
                                    match zoomed {
                                        true => t!("Dock.Zoom Out"),
                                        false => t!("Dock.Zoom In"),
                                    },
                                    Box::new(ToggleZoom),
                                    !menu_zoom,
                                )
                                .when(closable, |menu| {
                                    menu.separator()
                                        .menu(t!("Dock.Close"), Box::new(ClosePanel))
                                })
                            })
                            .anchor(Anchor::TopRight),
                    )
                },
            )
    }

    /// The one-panel title bar: no tabs, just the title and the controls.
    pub(super) fn render_title(
        &self,
        group: &TabGroupContext,
        ix: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let panel = &group.panels()[ix];
        let left_button = self.dock_toggle_button(DockPlacement::Left, group, cx);
        let bottom_button = self.dock_toggle_button(DockPlacement::Bottom, group, cx);
        let right_button = self.dock_toggle_button(DockPlacement::Right, group, cx);
        let has_leading = left_button.is_some() || bottom_button.is_some();
        let handle = PanelHandle::of(panel);
        let title_style = handle.and_then(|handle| handle.title_style(cx));
        let drag = tab_drag(group, ix, cx);
        let source = PaintedSource::default();
        let free = handle.and_then(|handle| handle.free_header_content(window, cx));
        let has_free = free.is_some();

        h_flex()
            .justify_between()
            .h(px(30.))
            .py_2()
            .pl_3()
            .pr_2()
            .when(left_button.is_some(), |this| this.pl_2())
            .when(right_button.is_some(), |this| this.pr_2())
            .when_some(title_style, |this, style| {
                this.bg(style.background).text_color(style.foreground)
            })
            .when(has_leading, |this| {
                this.child(
                    h_flex()
                        .flex_shrink_0()
                        .mr_1()
                        .gap_1()
                        .children(left_button)
                        .children(bottom_button),
                )
            })
            .child(
                title_visual(panel, window, cx)
                    .when(has_free, |title| title.flex_none())
                    .on_prepaint({
                        let source = source.clone();
                        move |bounds, window, _| source.capture(bounds, window)
                    })
                    .when_some(drag, |this, drag| {
                        this.on_drag(drag, {
                            let panel = panel.clone();
                            move |drag, offset, _, cx| {
                                source.start(drag, offset, PreviewContent::Title(panel.clone()), cx)
                            }
                        })
                    }),
            )
            .children(free.map(|content| div().flex_1().min_w_0().h_full().child(content)))
            .children(handle.and_then(|handle| handle.title_suffix(window, cx)))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .ml_1()
                    .gap_1()
                    .child(self.render_toolbar(group, window, cx))
                    .children(right_button),
            )
            .into_any_element()
    }
}
