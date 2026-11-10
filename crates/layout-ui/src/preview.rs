//! The status bar as it will look, on one line, where it is edited: the left zone flush left,
//! the centre in the middle and the right zone flush right, drawn from the preview text of each
//! widget. A group has a background of its own, and the panel that opens with the layout is
//! marked with the accent.
//!
//! Every chip and group is a drag source and opens its menu, built when it opens, on a right
//! click. The whole preview takes a drop, where `drops` says it lands; while an item is held
//! over it, one accent line in a gap shows where it would go, or a tint the panel it would make
//! a group with. The line never wraps: when it is too wide the chips lose their text and keep
//! their icons, and when even those do not fit it scrolls sideways.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, Entity, FocusHandle, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Toggled, Window,
    base::ElementExt as _,
    component::{ActiveTheme as _, h_flex, menu::ContextMenuExt as _, tooltip::Tooltip},
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::{ContributionId, ContributionKind, ContributionRef, PluginSet, Zone};
use nocterm_ui::{DragSource, contribution_icon};
use nocterm_window_layout::{ResolvedGroup, ResolvedItem, ResolvedMember, Slot, Subject};

use crate::{
    about,
    commands::Commands,
    dnd::{self, Dragged, Landing},
    drops::{self, Drops, Holds, Spot},
    face::{self, BORDER, CHIP_HEIGHT, Face, Look, PILL_PADDING},
    fit::{Density, EMPTY, Fit, Spread},
    keys,
    menu::{Claim, MenuOf, Menus},
    snapshot::Snapshot,
    user_facts::SharedFacts,
};

/// How visible the item being dragged stays in its place.
const MOVING: f32 = 0.4;
/// How much a chip under the pointer darkens, or lightens in a dark theme.
const HOVER: f32 = 0.07;

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

/// One drawing of the preview, and the drag held over it.
struct Bar<'a> {
    preview: &'a Preview,
    density: Density,
    menus: Menus,
    /// Where the entries painted are recorded; `None` when the layout shown takes no drops.
    drops: Option<Entity<Drops>>,
    /// Where the item held over the preview would land.
    aimed: Option<Landing>,
    /// The item being dragged.
    moving: Option<Subject>,
}

/// Where a chip is, and the group it is drawn in, which opens the chip's menu for it.
struct Setting {
    spot: Spot,
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
}

impl Bar<'_> {
    fn snapshot(&self) -> &Snapshot {
        &self.preview.snapshot
    }

    fn zone(&self, zone: Zone, spread: Spread, cx: &App) -> AnyElement {
        let items = self.snapshot().layout().zone(zone);
        let name = name(zone);
        let place = Slot::Zone(zone);
        let gap_id = |index| move || format!("layout-preview-gap:{name}:{index}");
        let mut entries = Vec::with_capacity(2 * items.len() + 1);
        for (index, item) in items.iter().enumerate() {
            entries.push(self.gap(gap_id(index), &place, index, cx));
            entries.push(self.item(zone, index, item, cx));
        }
        entries.push(self.gap(gap_id(items.len()), &place, items.len(), cx));
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
        let row = match &self.drops {
            Some(drops) => {
                let drops = drops.clone();
                dnd::measured(row, BORDER, move |bounds, _, cx| {
                    drops.update(cx, |drops, _| drops.zone(zone, bounds));
                })
            }
            None => row,
        };
        match (zone, spread) {
            (Zone::Left, Spread::Centred) => row.flex_1().justify_start(),
            (Zone::Right, Spread::Centred) => row.flex_1().justify_end(),
            _ => row.flex_none(),
        }
        .into_any_element()
    }

    /// The room in front of the entry at `index` of `place`, or after the last one, which
    /// shows the line where a drop would land.
    fn gap(&self, id: impl FnOnce() -> String, place: &Slot, index: usize, cx: &App) -> AnyElement {
        let marked = self
            .aimed
            .as_ref()
            .is_some_and(|aimed| aimed.is_at(place, index));
        let gap = face::gap().when(marked, |gap| gap.child(face::line(cx)));
        if self.drops.is_some() {
            gap.id(SharedString::from(id()))
                .test_support()
                .into_any_element()
        } else {
            gap.into_any_element()
        }
    }

    fn item(&self, zone: Zone, index: usize, item: &ResolvedItem, cx: &App) -> AnyElement {
        let spot = |holds| Spot::new(Slot::Zone(zone), index, holds);
        match item {
            ResolvedItem::Widget(id) => {
                let setting = Setting {
                    spot: spot(Holds::Item),
                    host: None,
                };
                self.contribution(id, setting, cx)
            }
            ResolvedItem::Separator => self.separator(
                SharedString::from(format!("layout-preview-separator:{}:{index}", name(zone))),
                spot(Holds::Item),
                None,
                cx,
            ),
            ResolvedItem::Group(group) if group.is_solo() => {
                let Some(ResolvedMember::Panel(panel)) = group.members().first() else {
                    return div().into_any_element();
                };
                let setting = Setting {
                    spot: spot(Holds::Alone(panel.clone())),
                    host: None,
                };
                self.contribution(panel, setting, cx)
            }
            ResolvedItem::Group(group) => {
                self.group(group, spot(Holds::Group(group.id().clone())), cx)
            }
        }
    }

    fn contribution(&self, id: &ContributionId, setting: Setting, cx: &App) -> AnyElement {
        match self.preview.catalog.contribution(id) {
            Some(contribution) => self.chip(contribution, setting, cx),
            None => div().into_any_element(),
        }
    }

    /// A group: its panels on a background of their own, without a title.
    fn group(&self, group: &ResolvedGroup, spot: Spot, cx: &App) -> AnyElement {
        let subject = Subject::Group(group.id().clone());
        let selected = self.preview.selected.as_ref() == Some(&subject);
        let title: SharedString = self.snapshot().group_title(group).into();
        let claim = Claim::default();
        let members = self.members(group, &claim, cx);
        let empty = members.is_empty();
        let joined = empty
            && self
                .aimed
                .as_ref()
                .is_some_and(|aimed| aimed.is_at(&Slot::Group(group.id().clone()), 0));
        let (commands, picked) = (self.preview.commands.clone(), subject.clone());
        let theme = cx.theme();
        let pill = face::pill(empty, cx)
            .id(SharedString::from(format!(
                "layout-preview-group:{}",
                group.id()
            )))
            .aria_label(title.clone())
            .aria_selected(selected)
            .test_support()
            .bg(theme.secondary_active)
            .when(selected, |pill| pill.border_color(theme.primary))
            .when(self.moving.as_ref() == Some(&subject), |pill| {
                pill.opacity(MOVING)
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                // A release that ends a drag started on a member is the drop, not a click.
                if !cx.has_active_drag() {
                    pick(&commands, picked.clone(), window, cx);
                }
            })
            .when(empty, |pill| {
                let tooltip = title.clone();
                pill.child(face::title(title, cx))
                    .tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip.clone())).into())
            })
            .children(members)
            .when(joined, |pill| pill.child(face::tint(cx)));
        let pill = match &self.drops {
            Some(drops) => {
                let source = DragSource::default();
                let dragged = Dragged::group(group.id(), spot.from());
                let pill = painted(pill, spot, Some(source.clone()), drops, BORDER);
                let (snapshot, catalog) =
                    (self.preview.snapshot.clone(), self.preview.catalog.clone());
                let id = group.id().clone();
                dnd::source(pill, dragged, source, move || {
                    group_look(&snapshot, &catalog, &id)
                })
            }
            None => pill,
        };
        let (menus, own) = (self.menus.clone(), MenuOf::Item(subject));
        pill.context_menu(move |popup, window, cx| {
            menus.popup(&claim.take_or(&own), popup, window, cx)
        })
        .into_any_element()
    }

    /// The panels and separators of a group, with the gaps around them.
    fn members(&self, group: &ResolvedGroup, claim: &Claim, cx: &App) -> Vec<AnyElement> {
        let members = group.members();
        if members.is_empty() {
            return Vec::new();
        }
        let place = Slot::Group(group.id().clone());
        let gap_id = |index| move || format!("layout-preview-member-gap:{}:{index}", group.id());
        let mut entries = Vec::with_capacity(2 * members.len() + 1);
        for (member, entry) in members.iter().enumerate() {
            entries.push(self.gap(gap_id(member), &place, member, cx));
            let spot = Spot::new(place.clone(), member, Holds::Item);
            entries.push(match entry {
                ResolvedMember::Panel(id) => {
                    let host = Some(claim.clone());
                    self.contribution(id, Setting { spot, host }, cx)
                }
                ResolvedMember::Separator => self.separator(
                    SharedString::from(format!(
                        "layout-preview-member-separator:{}:{member}",
                        group.id()
                    )),
                    spot,
                    Some(claim),
                    cx,
                ),
            });
        }
        entries.push(self.gap(gap_id(members.len()), &place, members.len(), cx));
        entries
    }

    fn separator(
        &self,
        id: SharedString,
        spot: Spot,
        host: Option<&Claim>,
        cx: &App,
    ) -> AnyElement {
        let (place, index) = spot.from();
        let line = face::separator(cx).id(id).test_support();
        let line = match &self.drops {
            Some(drops) => painted(line, spot, None, drops, 0.),
            None => line,
        };
        with_menu(line, &self.menus, MenuOf::Separator(place, index), host)
    }

    /// A widget or a panel toggle. A widget shows its sample text while the density allows;
    /// otherwise the chip is its icon and the tooltip names it.
    fn chip(&self, contribution: ContributionRef<'_>, setting: Setting, cx: &App) -> AnyElement {
        let snapshot = self.snapshot();
        let id = contribution.id().clone();
        let subject = Subject::Contribution(id.clone());
        let selected = self.preview.selected.as_ref() == Some(&subject);
        let disabled = snapshot.is_disabled(&id, &self.preview.catalog);
        let panel = contribution.kind() == ContributionKind::Panel;
        let open = panel && snapshot.opens_with_layout(&id);
        let text = (self.density == Density::Full && !panel).then(|| {
            SharedString::from(
                contribution
                    .preview()
                    .unwrap_or(contribution.title())
                    .to_owned(),
            )
        });
        let face = Face::new(contribution_icon(contribution.icon()), text, open);
        let name: SharedString = if disabled {
            format!("{} (disabled)", contribution.title()).into()
        } else {
            contribution.title().to_owned().into()
        };
        let grouped = matches!(&self.aimed, Some(Landing::With(target)) if *target == id);
        let (commands, picked) = (self.preview.commands.clone(), subject.clone());
        let theme = cx.theme();
        let chip = face
            .clone()
            .fill(h_flex(), cx)
            .id(SharedString::from(format!("layout-preview:{id}")))
            .aria_label(name.clone())
            .aria_selected(selected)
            .when(panel, |chip| {
                chip.aria_toggled(if open { Toggled::True } else { Toggled::False })
            })
            .test_support()
            .when(selected, |chip| chip.border_color(theme.primary))
            .when(!open, |chip| {
                chip.hover(|style| style.bg(theme.foreground.opacity(HOVER)))
            })
            .when(disabled, |chip| chip.opacity(0.5))
            .when(self.moving.as_ref() == Some(&subject), |chip| {
                chip.opacity(MOVING)
            })
            .cursor_pointer()
            .on_click(move |_, window, cx| {
                // A release that ends a drag is the drop; the group around a panel is not
                // picked by the same click.
                if !cx.has_active_drag() {
                    cx.stop_propagation();
                    pick(&commands, picked.clone(), window, cx);
                }
            })
            .tooltip(about::tooltip(
                &self.preview.catalog,
                self.preview.facts.as_ref(),
                &id,
                name,
                open,
            ))
            .when(grouped, |chip| chip.child(face::tint(cx)));
        let chip = match &self.drops {
            Some(drops) => {
                let source = DragSource::default();
                let dragged = Dragged::contribution(contribution, Some(setting.spot.from()));
                let chip = painted(chip, setting.spot, Some(source.clone()), drops, BORDER);
                dnd::source(chip, dragged, source, move || Look::Chip(face.clone()))
            }
            None => chip,
        };
        with_menu(
            chip,
            &self.menus,
            MenuOf::Item(subject),
            setting.host.as_ref(),
        )
    }
}

/// What the ghost of the group `id` shows: its panels, or its title while it has none.
fn group_look(
    snapshot: &Snapshot,
    catalog: &PluginSet,
    id: &nocterm_plugin_model::GroupId,
) -> Look {
    let group = snapshot.group(id);
    let members = group.map_or_else(Vec::new, |group| {
        group
            .members()
            .iter()
            .map(|member| match member {
                ResolvedMember::Panel(panel) => catalog.contribution(panel).map(|contribution| {
                    let open = snapshot.opens_with_layout(panel);
                    Face::new(contribution_icon(contribution.icon()), None, open)
                }),
                ResolvedMember::Separator => None,
            })
            .collect()
    });
    let title = group
        .map(|group| snapshot.group_title(group))
        .unwrap_or_default();
    Look::Group {
        members,
        title: title.into(),
    }
}

/// `element` recording, each time it is painted, where it is as `spot` in `drops`, and
/// measuring itself for the ghost of `source`.
fn painted<E: ParentElement>(
    element: E,
    spot: Spot,
    source: Option<DragSource>,
    drops: &Entity<Drops>,
    border: f32,
) -> E {
    let drops = drops.clone();
    dnd::measured(element, border, move |bounds, window, cx| {
        if let Some(source) = &source {
            source.capture(bounds, window);
        }
        drops.update(cx, |drops, _| drops.spot(spot, bounds));
    })
}

/// Selects `subject` for the keys of the editor.
fn pick(commands: &Commands, subject: Subject, window: &mut Window, cx: &mut App) {
    commands.run(window, cx, move |section, window, cx| {
        section.pick(Some(subject), window, cx);
    });
}

/// `element` opening the menu `of` on a right click: by itself, or through the group it is
/// drawn in.
fn with_menu<E>(element: E, menus: &Menus, of: MenuOf, host: Option<&Claim>) -> AnyElement
where
    E: InteractiveElement + ParentElement + Styled + IntoElement + 'static,
{
    match host {
        Some(host) => {
            let host = host.clone();
            element
                .on_mouse_down(MouseButton::Right, move |_, _, _| host.claim(of.clone()))
                .into_any_element()
        }
        None => {
            let menus = menus.clone();
            element
                .context_menu(move |popup, window, cx| menus.popup(&of, popup, window, cx))
                .into_any_element()
        }
    }
}

fn name(zone: Zone) -> &'static str {
    match zone {
        Zone::Left => "left",
        Zone::Center => "center",
        Zone::Right => "right",
    }
}

impl<'a> Bar<'a> {
    /// A drawing of `preview` at `density`, with the drag held over it when the layout takes
    /// drops. A drag that ended since the last drawing is forgotten.
    fn new(preview: &'a Preview, density: Density, window: &mut Window, cx: &mut App) -> Self {
        let drops = preview
            .snapshot
            .editable()
            .then(|| window.use_keyed_state("layout-preview-drops", cx, |_, _| Drops::default()));
        let held = cx.has_active_drag();
        let (aimed, moving) = drops.as_ref().map_or((None, None), |drops| {
            drops.update(cx, |drops, _| {
                if !held {
                    drops.settle();
                }
                drops.repaint();
                (drops.aimed().cloned(), drops.moving().cloned())
            })
        });
        let menus = Menus::new(
            preview.snapshot.clone(),
            preview.catalog.clone(),
            preview.commands.clone(),
        );
        Self {
            preview,
            density,
            menus,
            drops,
            aimed,
            moving,
        }
    }

    /// The three zones on one line, as wide as the box, or as the bar needs when it scrolls.
    fn line(&self, spread: Spread, scroll: Option<Pixels>, cx: &App) -> AnyElement {
        let line = h_flex()
            .id("layout-preview-line")
            .test_support()
            .items_center()
            .gap_x_3()
            .when(spread == Spread::Packed, |line| line.justify_between())
            .child(self.zone(Zone::Left, spread, cx))
            .child(self.zone(Zone::Center, spread, cx))
            .child(self.zone(Zone::Right, spread, cx));
        match scroll {
            Some(width) => line.w(width).flex_none(),
            None => line.w_full(),
        }
        .into_any_element()
    }
}

impl RenderOnce for Preview {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let plan = self.fit.plan(&self.snapshot, &self.catalog);
        let scroll = plan.scroll();
        let bar = Bar::new(&self, plan.density(), window, cx);
        let (line, drops) = (bar.line(plan.spread(), scroll, cx), bar.drops.clone());
        let (fit, commands, keys) = (
            self.fit.clone(),
            self.commands.clone(),
            self.commands.clone(),
        );
        let theme = cx.theme();
        let preview = div()
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
            });
        match drops {
            Some(drops) => drops::target(preview, drops, self.commands.clone()),
            None => preview,
        }
    }
}
