//! The status bar as it will look, on one line, where it is edited: the left zone flush left,
//! the centre in the middle and the right zone flush right, drawn from the preview text of each
//! widget.
//!
//! Every chip is a drag source and a drop target, and opens its menu on a right click. Between
//! the entries of a zone is a gap that takes a drop as well, so a panel can stand on its own in
//! front of another rather than join it. The line never wraps: when it is too wide the chips
//! lose their text and keep their icons, and when even those do not fit it scrolls sideways.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement as _, Styled, TestSupportExt as _, Window,
    base::ElementExt as _,
    component::{
        ActiveTheme as _, Icon, Sizable as _, h_flex, menu::ContextMenuExt as _, tooltip::Tooltip,
    },
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, ContributionRef, PluginSet, PluginTier, Zone,
};
use nocterm_ui::contribution_icon;
use nocterm_window_layout::{ResolvedGroup, ResolvedItem, ResolvedMember, Slot, Subject};

use crate::{
    commands::Commands,
    dnd::{self, CHIP, CHIP_HEIGHT, Dragged, GAP_WIDTH, Landing, PILL},
    fit::{Density, EMPTY, Fit, PILL_PADDING, Spread},
    keys,
    menu::{self, Claim, ItemMenu},
    snapshot::Snapshot,
    user_facts::SharedFacts,
};

#[derive(IntoElement)]
pub(crate) struct Preview {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginSet>,
    selected: Option<Subject>,
    facts: Option<SharedFacts>,
    fit: Fit,
    focus: FocusHandle,
    commands: Commands,
}

/// How a chip is drawn and what a drop on it does.
struct Setting {
    density: Density,
    /// Where a drop on the chip lands.
    landing: Landing,
    /// Whether a drop on the chip as a whole tints it: a panel on its own, which a dropped
    /// panel makes a group with.
    fills: bool,
    /// The group the chip is drawn in, which opens the chip's menu for it.
    host: Option<Claim>,
}

impl Preview {
    pub(crate) fn new(
        snapshot: Rc<Snapshot>,
        catalog: Arc<PluginSet>,
        selected: Option<Subject>,
        facts: Option<SharedFacts>,
        fit: Fit,
        focus: FocusHandle,
        commands: Commands,
    ) -> Self {
        Self {
            snapshot,
            catalog,
            selected,
            facts,
            fit,
            focus,
            commands,
        }
    }

    fn zone(&self, zone: Zone, density: Density, spread: Spread, cx: &App) -> AnyElement {
        let items = self.snapshot.layout().zone(zone);
        let name = name(zone);
        let mut entries = Vec::with_capacity(2 * items.len() + 1);
        for (index, item) in items.iter().enumerate() {
            entries.push(self.gap(zone, index, cx));
            entries.push(self.item(zone, index, item, density, cx));
        }
        entries.push(self.gap(zone, items.len(), cx));
        let row = h_flex()
            .id(SharedString::from(format!("layout-preview-zone:{name}")))
            .test_support()
            .min_w(px(EMPTY))
            .h(px(CHIP_HEIGHT + 2. * PILL_PADDING))
            .items_center()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .children(entries);
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

    /// The room in front of the entry at `index` of `zone`, or after the last one.
    fn gap(&self, zone: Zone, index: usize, cx: &App) -> AnyElement {
        if !self.snapshot.editable() {
            return div().flex_none().w(px(GAP_WIDTH)).into_any_element();
        }
        dnd::gap(
            SharedString::from(format!("layout-preview-gap:{}:{index}", name(zone))),
            Landing::in_zone(zone, index),
            self.commands.clone(),
            cx,
        )
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
            ResolvedItem::Widget(id) => {
                let setting = Setting {
                    density,
                    landing: Landing::in_zone(zone, index),
                    fills: false,
                    host: None,
                };
                self.contribution(id, setting, cx)
            }
            ResolvedItem::Separator => self.separator(
                SharedString::from(format!("layout-preview-separator:{}:{index}", name(zone))),
                Slot::Zone(zone),
                index,
                None,
                cx,
            ),
            ResolvedItem::Group(group) if group.is_solo() => {
                let Some(ResolvedMember::Panel(panel)) = group.members().first() else {
                    return div().into_any_element();
                };
                let setting = Setting {
                    density,
                    landing: Landing::beside(panel.clone(), zone, index),
                    fills: true,
                    host: None,
                };
                self.contribution(panel, setting, cx)
            }
            ResolvedItem::Group(group) => self.group(group, zone, index, density, cx),
        }
    }

    fn contribution(&self, id: &ContributionId, setting: Setting, cx: &App) -> AnyElement {
        match self.catalog.contribution(id) {
            Some(contribution) => self.chip(contribution, setting, cx),
            None => div().into_any_element(),
        }
    }

    /// A group: its panels on one filled background, without a title.
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
        let title: SharedString = self.snapshot.group_title(group).into();
        let claim = Claim::default();
        let members = self.members(group, density, &claim, cx);
        let empty = members.is_empty();
        let menu = ItemMenu::new(
            &subject,
            &self.snapshot,
            &self.catalog,
            self.commands.clone(),
        );
        let commands = self.commands.clone();
        let theme = cx.theme();
        let tooltip = title.clone();
        let pill = h_flex()
            .id(SharedString::from(format!(
                "layout-preview-group:{}",
                group.id()
            )))
            .aria_label(title.clone())
            .aria_selected(selected)
            .test_support()
            .group(PILL)
            .relative()
            .flex_none()
            .h(px(CHIP_HEIGHT + 2. * PILL_PADDING))
            .py(px(PILL_PADDING - 1.))
            .when(empty, |pill| pill.px(px(PILL_PADDING - 1.)))
            .items_center()
            .rounded(theme.radius)
            .bg(theme.secondary)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                gpui_kit::transparent_black()
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                let subject = subject.clone();
                commands.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .when(empty, |pill| {
                pill.child(
                    div()
                        .px_1p5()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(title.clone()),
                )
                .tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip.clone())).into())
            })
            .children(members);
        let pill = if self.snapshot.editable() {
            let landing = Landing::in_zone(zone, index)
                .and_group_end(group.id().clone(), group.members().len());
            let pill = pill.child(dnd::marker(
                SharedString::from(format!("layout-preview-marker:group:{}", group.id())),
                PILL,
                landing.clone(),
                cx,
            ));
            let pill = dnd::target(pill, landing, true, self.commands.clone());
            dnd::source(pill, Dragged::group(group.id(), title, &self.snapshot))
        } else {
            pill
        };
        pill.context_menu(move |popup, window, cx| claim.take_or(&menu).popup(popup, window, cx))
            .into_any_element()
    }

    /// The panels and separators of a group.
    fn members(
        &self,
        group: &ResolvedGroup,
        density: Density,
        claim: &Claim,
        cx: &App,
    ) -> Vec<AnyElement> {
        let members = group.members();
        if members.is_empty() {
            return Vec::new();
        }
        let mut entries = Vec::with_capacity(2 * members.len() + 1);
        for (member, entry) in members.iter().enumerate() {
            entries.push(self.member_gap(group, member, cx));
            entries.push(self.member(group, member, entry, density, claim, cx));
        }
        entries.push(self.member_gap(group, members.len(), cx));
        entries
    }

    /// The room in front of the member at `index` of `group`, or after the last one.
    fn member_gap(&self, group: &ResolvedGroup, index: usize, cx: &App) -> AnyElement {
        if !self.snapshot.editable() {
            return div().flex_none().w(px(GAP_WIDTH)).into_any_element();
        }
        dnd::gap(
            SharedString::from(format!("layout-preview-member-gap:{}:{index}", group.id())),
            Landing::in_group(group.id().clone(), index),
            self.commands.clone(),
            cx,
        )
    }

    fn member(
        &self,
        group: &ResolvedGroup,
        member: usize,
        entry: &ResolvedMember,
        density: Density,
        claim: &Claim,
        cx: &App,
    ) -> AnyElement {
        match entry {
            ResolvedMember::Panel(id) => {
                let setting = Setting {
                    density,
                    landing: Landing::in_group(group.id().clone(), member),
                    fills: false,
                    host: Some(claim.clone()),
                };
                self.contribution(id, setting, cx)
            }
            ResolvedMember::Separator => self.separator(
                SharedString::from(format!(
                    "layout-preview-member-separator:{}:{member}",
                    group.id()
                )),
                Slot::Group(group.id().clone()),
                member,
                Some(claim),
                cx,
            ),
        }
    }

    fn separator(
        &self,
        id: SharedString,
        place: Slot,
        index: usize,
        host: Option<&Claim>,
        cx: &App,
    ) -> AnyElement {
        let menu = ItemMenu::of(
            menu::separator(place, index, &self.snapshot),
            self.commands.clone(),
        );
        let line = div()
            .id(id)
            .test_support()
            .flex_none()
            .h(px(CHIP_HEIGHT))
            .px(px(2.))
            .flex()
            .items_center()
            .child(div().w(px(1.)).h(px(12.)).bg(cx.theme().border));
        with_menu(line, menu, host)
    }

    /// A widget or a panel toggle. A widget shows its sample text while the density allows;
    /// otherwise the chip is its icon and the tooltip names it.
    fn chip(&self, contribution: ContributionRef<'_>, setting: Setting, cx: &App) -> AnyElement {
        let id = contribution.id().clone();
        let subject = Subject::Contribution(id.clone());
        let selected = self.selected.as_ref() == Some(&subject);
        let disabled = self.snapshot.is_disabled(&id, &self.catalog);
        let text = contribution
            .preview()
            .unwrap_or(contribution.title())
            .to_owned();
        let shown = (setting.density == Density::Full
            && contribution.kind() == ContributionKind::Widget)
            .then_some(text);
        let commands = self.commands.clone();
        let menu = ItemMenu::new(&subject, &self.snapshot, &self.catalog, commands.clone());
        let theme = cx.theme();
        let name: SharedString = if disabled {
            format!("{} (disabled)", contribution.title()).into()
        } else {
            contribution.title().to_owned().into()
        };
        let about = self.about(contribution, name.clone(), cx);
        let chip = h_flex()
            .id(SharedString::from(format!("layout-preview:{id}")))
            .aria_label(name)
            .aria_selected(selected)
            .test_support()
            .group(CHIP)
            .relative()
            .flex_none()
            .h(px(CHIP_HEIGHT))
            .gap_1()
            .px_1p5()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                gpui_kit::transparent_black()
            })
            .hover(|style| style.bg(theme.secondary_hover))
            .when(disabled, |chip| chip.opacity(0.5))
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                // The group around a panel is not picked by the same click.
                cx.stop_propagation();
                let subject = subject.clone();
                commands.run(window, cx, move |section, window, cx| {
                    section.pick(Some(subject), window, cx);
                });
            })
            .child(Icon::new(contribution_icon(contribution.icon())).small())
            .children(shown.map(|text| div().text_xs().child(text)))
            .tooltip(move |_, cx| cx.new(|_| Tooltip::new(about.clone())).into());
        let chip = if self.snapshot.editable() {
            let chip = chip.child(dnd::marker(
                SharedString::from(format!("layout-preview-marker:{id}")),
                CHIP,
                setting.landing.clone(),
                cx,
            ));
            let chip = dnd::target(chip, setting.landing, setting.fills, self.commands.clone());
            dnd::source(chip, Dragged::contribution(contribution, &self.snapshot))
        } else {
            chip
        };
        with_menu(chip, menu, setting.host.as_ref())
    }

    /// The tooltip of a chip: its name and, for an item of a user plugin, what the host knows
    /// of the plugin, in the host's own words.
    fn about(
        &self,
        contribution: ContributionRef<'_>,
        name: SharedString,
        cx: &App,
    ) -> SharedString {
        let Some(plugin) = self
            .catalog
            .plugin_of(contribution.id())
            .filter(|plugin| plugin.tier() == PluginTier::User)
        else {
            return name;
        };
        let mut lines = vec![
            name.to_string(),
            format!("Plugin \u{b7} {} ({})", plugin.name(), plugin.id()),
        ];
        if let Some(facts) = self
            .facts
            .as_ref()
            .and_then(|facts| facts.facts(plugin.id(), cx))
        {
            lines.push(format!("Publisher: {} (unverified)", facts.publisher));
            lines.push(format!("Fingerprint: {}", facts.fingerprint));
            lines.push(if facts.capabilities.is_empty() {
                "Allowed: nothing".to_owned()
            } else {
                format!("Allowed: {}", facts.capabilities.join("; "))
            });
        }
        lines.join("\n").into()
    }
}

/// `element` opening `menu` on a right click: by itself, or through the group it is drawn in.
fn with_menu<E>(element: E, menu: Rc<ItemMenu>, host: Option<&Claim>) -> AnyElement
where
    E: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
{
    match host {
        Some(host) => {
            let host = host.clone();
            element
                .on_mouse_down(MouseButton::Right, move |_, _, _| host.claim(menu.clone()))
                .into_any_element()
        }
        None => element
            .context_menu(move |popup, window, cx| menu.popup(popup, window, cx))
            .into_any_element(),
    }
}

fn name(zone: Zone) -> &'static str {
    match zone {
        Zone::Left => "left",
        Zone::Center => "center",
        Zone::Right => "right",
    }
}

impl RenderOnce for Preview {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let plan = self.fit.plan(&self.snapshot, &self.catalog);
        let (density, spread, scroll) = (plan.density(), plan.spread(), plan.scroll());
        let (fit, commands, keys) = (
            self.fit.clone(),
            self.commands.clone(),
            self.commands.clone(),
        );
        let theme = cx.theme();
        let line = h_flex()
            .id("layout-preview-line")
            .test_support()
            .items_center()
            .gap_x_3()
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
            .track_focus(&self.focus)
            .on_key_down(move |event, window, cx| {
                if let Some(command) = keys::command(&event.keystroke) {
                    cx.stop_propagation();
                    keys.run(window, cx, move |section, _, cx| section.key(command, cx));
                }
            })
            .w_full()
            .min_w_0()
            .px_2()
            .py_1()
            .rounded(theme.radius_lg)
            .bg(theme.muted)
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
