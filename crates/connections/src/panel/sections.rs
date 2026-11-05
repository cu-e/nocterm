//! Prepared sidebar positions and organization headers. Saved profiles stay in their service.
use super::*;
use gpui_kit::{ListAlignment, ListState, px};
use nocterm_connections_model::{
    profiles::Profiles,
    replica::{ReplicaAccess, ReplicaScope, ReplicaSpace},
};
use std::{collections::HashMap, rc::Rc};

pub(super) enum Entry {
    Ungrouped,
    Profile {
        index: usize,
        id: ProfileId,
        group: Option<Rc<str>>,
        writable: bool,
    },
    Group(Rc<str>, bool),
    Append(Option<Rc<str>>),
    Space(Section),
    SpaceEmpty(String),
}

impl Entry {
    pub(super) fn render(
        &self,
        panel: &ConnectionsPanel,
        cx: &mut Context<ConnectionsPanel>,
    ) -> AnyElement {
        let row = match self {
            Self::Ungrouped => ungrouped(cx).into_any_element(),
            Self::Profile {
                index,
                id,
                group,
                writable,
            } => profile_row(*index, *id, group.as_ref(), *writable, panel, cx),
            Self::Group(group, collapsed) => panel
                .render_group_header(group, *collapsed, cx)
                .into_any_element(),
            Self::Append(group) => {
                let zone = panel
                    .render_append_zone(group.as_deref(), cx)
                    .into_any_element();
                match group {
                    Some(group) => div()
                        .id(SharedString::from(format!("group-members-{group}")))
                        .test_support()
                        .pl_3()
                        .child(zone)
                        .into_any_element(),
                    None => div().child(zone).into_any_element(),
                }
            }
            Self::Space(section) => section.render(cx).into_any_element(),
            Self::SpaceEmpty(id) => div()
                .id(SharedString::from(format!("connections-space-{id}-empty")))
                .test_support()
                .mx_1()
                .px_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Nothing shared yet.")
                .into_any_element(),
        };
        div().pb_px().child(row).into_any_element()
    }
}
fn ungrouped(cx: &mut Context<ConnectionsPanel>) -> impl IntoElement {
    h_flex()
        .id("connections-ungrouped")
        .test_support()
        .mx_1()
        .px_2()
        .py_1()
        .rounded(cx.theme().radius)
        .gap_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::Server).small())
        .child("Ungrouped")
        .drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
        .on_drop(cx.listener(|this, dragged: &DraggedProfile, window, cx| {
            this.move_profile(dragged, None, None, window, cx)
        }))
}
fn profile_row(
    index: usize,
    id: ProfileId,
    group: Option<&Rc<str>>,
    writable: bool,
    panel: &ConnectionsPanel,
    cx: &mut Context<ConnectionsPanel>,
) -> AnyElement {
    let profiles = panel.installed.connections().read(cx).profiles(cx);
    // A frame prepared before a deferred publication observation cannot display
    // a different record that moved into the same position.
    let Some(profile) = profiles
        .iter()
        .nth(index)
        .filter(|profile| profile.id == id)
    else {
        return div().into_any_element();
    };
    let visual = row::RowVisual::new(profile, &panel.installed, cx);
    let row = panel.render_row(visual, writable, cx).into_any_element();
    div()
        .when(group.is_some(), |row| row.pl_3())
        .when_some(group.cloned(), |row, group| {
            row.drag_over::<DraggedProfile>(|style, _, _, cx| style.bg(cx.theme().accent))
                .on_drop(
                    cx.listener(move |this, dragged: &DraggedProfile, window, cx| {
                        this.move_profile(dragged, Some(group.to_string()), None, window, cx)
                    }),
                )
        })
        .child(row)
        .into_any_element()
}

pub(super) struct ConnectionList {
    rows: Rc<Vec<Entry>>,
    scroll: ListState,
    dirty: bool,
    empty: bool,
    nothing_matches: bool,
}
impl Default for ConnectionList {
    fn default() -> Self {
        Self {
            rows: Rc::default(),
            scroll: ListState::new(0, ListAlignment::Top, px(100.)),
            dirty: true,
            empty: true,
            nothing_matches: false,
        }
    }
}
impl ConnectionList {
    pub(super) fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub(super) fn rows(&self) -> Rc<Vec<Entry>> {
        self.rows.clone()
    }
    pub(super) fn scroll(&self) -> ListState {
        self.scroll.clone()
    }
    pub(super) fn empty(&self) -> bool {
        self.empty
    }
    pub(super) fn nothing_matches(&self) -> bool {
        self.nothing_matches
    }
    pub(super) fn prepare(
        &mut self,
        profiles: &Profiles,
        spaces: &Spaces,
        filter: &str,
        collapsed: &HashSet<String>,
        rename: Option<(&str, FocusHandle)>,
        cx: &App,
    ) {
        if !self.dirty {
            return;
        }
        let listing = Listing::new(profiles, spaces, filter, cx);
        self.empty = profiles.is_empty() && !spaces.shared();
        self.nothing_matches = listing.is_empty(!filter.trim().is_empty());
        let rows = listing.into_rows(profiles, spaces, filter, collapsed, cx);
        let offset = self.scroll.logical_scroll_top();
        self.scroll.splice_focusable(
            0..self.rows.len(),
            rows.iter().map(|entry| match (entry, &rename) {
                (Entry::Group(group, _), Some((name, focus))) if group.as_ref() == *name => {
                    Some(focus.clone())
                }
                _ => None,
            }),
        );
        // Unmeasured rows otherwise contribute zero scroll extent. A height hint lets
        // native wheel/thumb jumps reach unseen rows without constructing the whole list.
        self.scroll = self.scroll.clone().with_uniform_item_height(px(64.));
        self.scroll.scroll_to(offset);
        self.rows = Rc::new(rows);
        self.dirty = false;
    }
}

/// A temporary borrowed partition, used only when semantic list inputs change.
struct Listing<'a> {
    top: Vec<&'a Profile>,
    groups: Vec<(String, Vec<&'a Profile>)>,
    sections: Vec<(Section, Vec<&'a Profile>)>,
}
impl<'a> Listing<'a> {
    fn new(profiles: &'a Profiles, spaces: &Spaces, filter: &str, cx: &App) -> Self {
        let (top, groups) = profiles.grouped(filter);
        let mut sections: Vec<_> = spaces
            .organizations()
            .map(|space| (Section::of(space, spaces.editable()), Vec::new()))
            .collect();
        let mut unreachable = Vec::new();
        let mut file = |profile: &'a Profile| match spaces.scope(profile.id, cx) {
            ReplicaScope::Personal => Some(profile),
            scope => {
                match sections
                    .iter_mut()
                    .find(|(section, _)| section.scope.as_ref() == Some(&scope))
                {
                    Some((_, members)) => members.push(profile),
                    None => unreachable.push(profile),
                }
                None
            }
        };
        let top = top.into_iter().filter_map(&mut file).collect();
        let groups = groups
            .into_iter()
            .filter_map(|(name, members)| {
                let listed = !members.is_empty();
                let rest: Vec<_> = members.into_iter().filter_map(&mut file).collect();
                (rest.is_empty() == !listed).then_some((name, rest))
            })
            .collect();
        if !unreachable.is_empty() {
            sections.push((Section::unavailable(), unreachable));
        }
        Self {
            top,
            groups,
            sections,
        }
    }
    fn is_empty(&self, filtering: bool) -> bool {
        self.top.is_empty()
            && self.groups.is_empty()
            && self
                .sections
                .iter()
                .all(|(_, members)| filtering && members.is_empty())
    }
    fn into_rows(
        self,
        profiles: &Profiles,
        spaces: &Spaces,
        filter: &str,
        collapsed: &HashSet<String>,
        cx: &App,
    ) -> Vec<Entry> {
        let indices: HashMap<_, _> = profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| (profile.id, index))
            .collect();
        let profile_row = |profile: &Profile, group, writable| Entry::Profile {
            index: indices[&profile.id],
            id: profile.id,
            group,
            writable,
        };
        let mut rows = vec![Entry::Ungrouped];
        rows.extend(
            self.top
                .into_iter()
                .map(|profile| profile_row(profile, None, spaces.writable(profile.id, cx))),
        );
        rows.push(Entry::Append(None));
        for (name, members) in self.groups {
            let collapsed = filter.trim().is_empty() && collapsed.contains(&name);
            let group: Rc<str> = name.into();
            rows.push(Entry::Group(group.clone(), collapsed));
            if !collapsed {
                rows.extend(members.into_iter().map(|profile| {
                    profile_row(
                        profile,
                        Some(group.clone()),
                        spaces.writable(profile.id, cx),
                    )
                }));
                rows.push(Entry::Append(Some(group)));
            }
        }
        for (section, members) in self.sections {
            let writable = section.writable;
            let id = section.id.clone();
            let empty = members.is_empty();
            rows.push(Entry::Space(section));
            rows.extend(
                members
                    .into_iter()
                    .map(|profile| profile_row(profile, None, writable)),
            );
            if empty && filter.trim().is_empty() {
                rows.push(Entry::SpaceEmpty(id));
            }
        }
        rows
    }
}

pub(super) struct Section {
    id: String,
    name: String,
    scope: Option<ReplicaScope>,
    writable: bool,
}
impl Section {
    fn of(space: &ReplicaSpace, editable: bool) -> Self {
        let id = match &space.scope {
            ReplicaScope::Personal => "personal".to_owned(),
            ReplicaScope::Organization(id) => id.clone(),
        };
        Self {
            id,
            name: space.name.clone(),
            scope: Some(space.scope.clone()),
            writable: editable && space.access == ReplicaAccess::Write,
        }
    }
    fn unavailable() -> Self {
        Self {
            id: "unavailable".into(),
            name: "Unavailable".into(),
            scope: None,
            writable: false,
        }
    }
    pub(super) fn render(&self, cx: &App) -> impl IntoElement + use<> {
        let id = &self.id;
        let theme = cx.theme();
        h_flex()
            .id(SharedString::from(format!("connections-space-{id}")))
            .test_support()
            .mx_1()
            .px_2()
            .py_1()
            .gap_1()
            .rounded(theme.radius)
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Layers).small())
            .child(div().flex_1().min_w_0().truncate().child(self.name.clone()))
            .when(!self.writable, |header| {
                header.child(
                    div()
                        .id(SharedString::from(format!(
                            "connections-space-{id}-readonly"
                        )))
                        .test_support()
                        .font_normal()
                        .child("read-only"),
                )
            })
    }
}
