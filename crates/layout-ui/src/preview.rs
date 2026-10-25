//! The status bar as it will look, on one line: the left zone flush left, the centre in the
//! middle and the right zone flush right, drawn from the preview text of each widget.
//!
//! The line never wraps. When it is too wide for its box the chips lose their text and keep
//! their icons, and when even those do not fit the line scrolls sideways inside the box. Each
//! chip is both a drag source and a drop target.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window,
    base::ElementExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, h_flex, menu::ContextMenuExt as _, tooltip::Tooltip,
    },
    div,
    prelude::*,
};
use nocterm_plugin_model::{ContributionId, ContributionRef, PluginSet, Zone};
use nocterm_ui::contribution_icon;
use nocterm_window_layout::{ResolvedGroup, ResolvedItem, ResolvedMember, Subject};

use crate::{
    commands::Commands,
    dnd::{self, BOX, CHIP, Dragged, Landing},
    fit::{Density, Fit, Spread},
    menu::ItemMenu,
    parts::tag,
    snapshot::Snapshot,
};

#[derive(IntoElement)]
pub(crate) struct Preview {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginSet>,
    selected: Option<Subject>,
    fit: Fit,
    commands: Commands,
}

/// Where a chip sits and how it is drawn.
struct Setting {
    density: Density,
    /// Where a drop on the chip lands.
    landing: Landing,
}

impl Preview {
    pub(crate) fn new(
        snapshot: Rc<Snapshot>,
        catalog: Arc<PluginSet>,
        selected: Option<Subject>,
        fit: Fit,
        commands: Commands,
    ) -> Self {
        Self {
            snapshot,
            catalog,
            selected,
            fit,
            commands,
        }
    }

    fn zone(&self, zone: Zone, density: Density, spread: Spread, cx: &App) -> AnyElement {
        let items = self.snapshot.layout().zone(zone);
        let (name, title) = names(zone);
        let theme = cx.theme();
        let row = h_flex()
            .id(SharedString::from(format!("layout-preview-zone:{name}")))
            .test_support()
            .min_w_0()
            .gap_2()
            .px_1()
            .py_0p5()
            .items_center()
            .rounded(theme.radius)
            .when(items.is_empty(), |row| {
                row.child(
                    div()
                        .id(SharedString::from(format!("layout-preview-empty:{name}")))
                        .test_support()
                        .px_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_dashed()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(title),
                )
            })
            .children(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| self.item(zone, index, item, density, cx)),
            );
        let row = if self.snapshot.editable() {
            dnd::target(
                row,
                Landing::zone_area(zone, items.len()),
                true,
                self.commands.clone(),
            )
        } else {
            row
        };
        match (zone, spread) {
            (Zone::Left, Spread::Centred) => row.flex_1().justify_start(),
            (Zone::Right, Spread::Centred) => row.flex_1().justify_end(),
            _ => row.flex_none(),
        }
        .into_any_element()
    }

    fn item(
        &self,
        zone: Zone,
        index: usize,
        item: &ResolvedItem,
        density: Density,
        cx: &App,
    ) -> AnyElement {
        match item {
            ResolvedItem::Widget(id) => self.widget(id, zone, index, density, cx),
            ResolvedItem::Separator => bar(
                SharedString::from(format!(
                    "layout-preview-separator:{}:{index}",
                    names(zone).0
                )),
                cx,
            ),
            ResolvedItem::Group(group) => self.group(group, zone, index, density, cx),
        }
    }

    fn widget(
        &self,
        id: &ContributionId,
        zone: Zone,
        index: usize,
        density: Density,
        cx: &App,
    ) -> AnyElement {
        let Some(contribution) = self.catalog.contribution(id) else {
            return div().into_any_element();
        };
        let text = contribution.preview().unwrap_or(contribution.title());
        let setting = Setting {
            density,
            landing: Landing::in_zone(zone, index),
        };
        self.chip(contribution, Some(text), setting, cx)
    }

    fn group(
        &self,
        group: &ResolvedGroup,
        zone: Zone,
        index: usize,
        density: Density,
        cx: &App,
    ) -> AnyElement {
        let subject = Subject::Group(group.id().clone());
        let selected = self.selected.as_ref() == Some(&subject);
        let title = self
            .snapshot
            .groups()
            .iter()
            .find(|spec| spec.id == *group.id())
            .map_or_else(
                || SharedString::from(group.id().as_str().to_owned()),
                |spec| SharedString::from(spec.title),
            );
        let members = self.members(group, density, cx);
        let named = density == Density::Full || group.members().is_empty();
        let commands = self.commands.clone();
        let theme = cx.theme();
        let tooltip = title.clone();
        let line = h_flex()
            .id(SharedString::from(format!(
                "layout-preview-group:{}",
                group.id()
            )))
            .aria_label(title.clone())
            .test_support()
            .group(BOX)
            .relative()
            .min_w_0()
            .gap_1()
            .px_1()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let subject = subject.clone();
                commands.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .when(!named, |line| {
                line.tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip.clone())).into())
            })
            .when(named, |line| {
                line.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(title.clone()),
                )
            })
            .children(members);
        if !self.snapshot.editable() {
            return line.into_any_element();
        }
        let before = Landing::in_zone(zone, index);
        let landing = before
            .clone()
            .and_group_end(group.id().clone(), group.members().len());
        let line = line.child(dnd::marker(
            SharedString::from(format!("layout-preview-marker:group:{}", group.id())),
            BOX,
            true,
            before,
            cx,
        ));
        let line = dnd::target(line, landing, true, self.commands.clone());
        dnd::source(line, Dragged::group(group.id(), title, &self.snapshot)).into_any_element()
    }

    /// The panels and separators of a group's box.
    fn members(&self, group: &ResolvedGroup, density: Density, cx: &App) -> Vec<AnyElement> {
        group
            .members()
            .iter()
            .enumerate()
            .map(|(member, entry)| match entry {
                ResolvedMember::Panel(id) => match self.catalog.contribution(id) {
                    Some(contribution) => {
                        let setting = Setting {
                            density,
                            landing: Landing::in_group(group.id().clone(), member),
                        };
                        self.chip(contribution, None, setting, cx)
                    }
                    None => div().into_any_element(),
                },
                ResolvedMember::Separator => bar(
                    SharedString::from(format!(
                        "layout-preview-member-separator:{}:{member}",
                        group.id()
                    )),
                    cx,
                ),
            })
            .collect()
    }

    /// A widget or a panel toggle. `text` is a widget's sample, shown while the density allows;
    /// otherwise the chip is its icon and a tooltip names it.
    fn chip(
        &self,
        contribution: ContributionRef<'_>,
        text: Option<&str>,
        setting: Setting,
        cx: &App,
    ) -> AnyElement {
        let id = contribution.id().clone();
        let subject = Subject::Contribution(id.clone());
        let selected = self.selected.as_ref() == Some(&subject);
        let disabled = self.snapshot.is_disabled(&id, &self.catalog);
        let shown = text.filter(|_| setting.density == Density::Full);
        let empty_when_idle = shown.is_some() && contribution.preview().is_none();
        let commands = self.commands.clone();
        let menu = ItemMenu::new(&subject, &self.snapshot, &self.catalog, commands.clone());
        let theme = cx.theme();
        let name: SharedString = if disabled {
            format!("{} (disabled)", contribution.title()).into()
        } else {
            contribution.title().to_owned().into()
        };
        let chip = h_flex()
            .id(SharedString::from(format!("layout-preview:{id}")))
            .aria_label(contribution.title())
            .test_support()
            .group(CHIP)
            .relative()
            .occlude()
            .gap_1()
            .px_1p5()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .when(empty_when_idle, |chip| chip.border_dashed())
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .when(disabled, |chip| chip.opacity(0.5))
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let subject = subject.clone();
                commands.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .child(Icon::new(contribution_icon(contribution.icon())).small())
            .children(shown.map(|text| div().text_xs().child(text.to_owned())))
            .when(disabled && shown.is_some(), |chip| {
                chip.child(tag(format!("layout-preview-disabled:{id}"), "Disabled", cx))
            })
            .when(shown.is_none(), |chip| {
                chip.tooltip(move |_, cx| cx.new(|_| Tooltip::new(name.clone())).into())
            });
        let chip = if self.snapshot.editable() {
            let chip = chip.child(dnd::marker(
                SharedString::from(format!("layout-preview-marker:{id}")),
                CHIP,
                true,
                setting.landing.clone(),
                cx,
            ));
            let chip = dnd::target(chip, setting.landing, false, self.commands.clone());
            dnd::source(chip, Dragged::contribution(contribution, &self.snapshot))
        } else {
            chip
        };
        chip.context_menu(move |popup, window, cx| menu.popup(popup, window, cx))
            .into_any_element()
    }
}

fn names(zone: Zone) -> (&'static str, &'static str) {
    match zone {
        Zone::Left => ("left", "Left"),
        Zone::Center => ("center", "Centre"),
        Zone::Right => ("right", "Right"),
    }
}

fn bar(id: impl Into<gpui_kit::ElementId>, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .w(gpui_kit::px(1.))
        .h_4()
        .flex_shrink_0()
        .bg(cx.theme().border)
        .into_any_element()
}

impl RenderOnce for Preview {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let plan = self.fit.plan(&self.snapshot, &self.catalog);
        let (density, spread, scroll) = (plan.density(), plan.spread(), plan.scroll());
        let (fit, commands) = (self.fit.clone(), self.commands.clone());
        let theme = cx.theme();
        let line = h_flex()
            .id("layout-preview-line")
            .test_support()
            .items_center()
            .gap_x_4()
            .when(spread == Spread::Packed, |line| line.justify_between())
            .child(self.zone(Zone::Left, density, spread, cx))
            .child(self.zone(Zone::Center, density, spread, cx))
            .child(self.zone(Zone::Right, density, spread, cx));
        let line = match scroll {
            Some(width) => line.w(width).flex_none(),
            None => line.w_full(),
        };
        div()
            .id("layout-preview")
            .test_support()
            .w_full()
            .min_w_0()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("layout-preview-scroll")
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .when(scroll.is_some(), |frame| frame.overflow_x_scroll())
                    .child(line),
            )
            .on_prepaint(move |bounds, window, cx| {
                if fit.observe(bounds.size.width) {
                    window.defer(cx, move |window, cx| {
                        commands.run(window, cx, |_, _, cx| cx.notify());
                    });
                }
            })
    }
}
