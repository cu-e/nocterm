//! Prepared sidebar positions. Saved profiles stay in their service.
use super::*;
use gpui_kit::{ListAlignment, ListState, px};
use nocterm_connections_model::profiles::Profiles;
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
    /// `editable`: whether the list may be changed now, so its rows can be edited and moved.
    pub(super) fn prepare(
        &mut self,
        profiles: &Profiles,
        editable: bool,
        filter: &str,
        collapsed: &HashSet<String>,
        rename: Option<(&str, FocusHandle)>,
    ) {
        if !self.dirty {
            return;
        }
        let listing = Listing::new(profiles, filter);
        self.empty = profiles.is_empty();
        self.nothing_matches = listing.is_empty();
        let rows = listing.into_rows(profiles, editable, filter, collapsed);
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
}
impl<'a> Listing<'a> {
    fn new(profiles: &'a Profiles, filter: &str) -> Self {
        let (top, groups) = profiles.grouped(filter);
        Self { top, groups }
    }
    fn is_empty(&self) -> bool {
        self.top.is_empty() && self.groups.is_empty()
    }
    fn into_rows(
        self,
        profiles: &Profiles,
        editable: bool,
        filter: &str,
        collapsed: &HashSet<String>,
    ) -> Vec<Entry> {
        let indices: HashMap<_, _> = profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| (profile.id, index))
            .collect();
        let profile_row = |profile: &Profile, group| Entry::Profile {
            index: indices[&profile.id],
            id: profile.id,
            group,
            writable: editable,
        };
        let mut rows = vec![Entry::Ungrouped];
        rows.extend(
            self.top
                .into_iter()
                .map(|profile| profile_row(profile, None)),
        );
        rows.push(Entry::Append(None));
        for (name, members) in self.groups {
            let collapsed = filter.trim().is_empty() && collapsed.contains(&name);
            let group: Rc<str> = name.into();
            rows.push(Entry::Group(group.clone(), collapsed));
            if !collapsed {
                rows.extend(
                    members
                        .into_iter()
                        .map(|profile| profile_row(profile, Some(group.clone()))),
                );
                rows.push(Entry::Append(Some(group)));
            }
        }
        rows
    }
}
