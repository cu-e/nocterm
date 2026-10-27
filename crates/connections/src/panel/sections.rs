//! Connections shared through the account's organizations, one section each below the
//! personal list.
use super::*;
use crate::spaces::Spaces;
use nocterm_connections_model::replica::{ReplicaAccess, ReplicaScope, ReplicaSpace};

/// One organization and the connections of it that the filter lets through.
pub(super) struct Section {
    space: ReplicaSpace,
    members: Vec<Profile>,
}

/// The list as shown: the personal connections filed in folders, then the sections.
pub(super) struct Listing {
    pub(super) top: Vec<Profile>,
    pub(super) groups: Vec<(String, Vec<Profile>)>,
    pub(super) sections: Vec<Section>,
}

impl Listing {
    /// Moves the connections of each organization out of the folders into its section.
    /// Folders are the user's own way to file their connections, so a shared connection
    /// is listed flat. Without organizations the list is returned as it came.
    pub(super) fn new(
        spaces: &Spaces,
        top: Vec<Profile>,
        groups: Vec<(String, Vec<Profile>)>,
        cx: &App,
    ) -> Self {
        let mut sections: Vec<Section> = spaces
            .organizations()
            .map(|space| Section {
                space: space.clone(),
                members: Vec::new(),
            })
            .collect();
        if sections.is_empty() {
            return Self {
                top,
                groups,
                sections,
            };
        }
        let mut file = |profile: Profile| {
            let scope = spaces.scope(profile.id, cx);
            match sections.iter_mut().find(|s| s.space.scope == scope) {
                Some(section) => {
                    section.members.push(profile);
                    None
                }
                None => Some(profile),
            }
        };
        let top: Vec<Profile> = top.into_iter().filter_map(&mut file).collect();
        let groups = groups
            .into_iter()
            .map(|(name, members)| (name, members.into_iter().filter_map(&mut file).collect()))
            .filter(|(_, members): &(String, Vec<Profile>)| !members.is_empty())
            .collect();
        Self {
            top,
            groups,
            sections,
        }
    }

    /// Whether the filter leaves nothing to show.
    pub(super) fn is_empty(&self, filtering: bool) -> bool {
        self.top.is_empty()
            && self.groups.is_empty()
            && self
                .sections
                .iter()
                .all(|section| filtering && section.members.is_empty())
    }
}

fn space_id(scope: &ReplicaScope) -> String {
    match scope {
        ReplicaScope::Personal => "personal".to_owned(),
        ReplicaScope::Organization(id) => id.clone(),
    }
}

impl ConnectionsPanel {
    pub(super) fn render_section(
        &self,
        section: &Section,
        filtering: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = space_id(&section.space.scope);
        let writable = section.space.access == ReplicaAccess::Write;
        let theme = cx.theme();
        let header = h_flex()
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
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(section.space.name.clone()),
            )
            .when(!writable, |header| {
                header.child(
                    div()
                        .id(SharedString::from(format!(
                            "connections-space-{id}-readonly"
                        )))
                        .test_support()
                        .font_normal()
                        .child("read-only"),
                )
            });
        let muted = theme.muted_foreground;
        let mut list = v_flex().gap_px().child(header);
        for profile in &section.members {
            list = list.child(self.render_row(profile, writable, cx));
        }
        if section.members.is_empty() && !filtering {
            list = list.child(
                div()
                    .id(SharedString::from(format!("connections-space-{id}-empty")))
                    .test_support()
                    .mx_1()
                    .px_2()
                    .text_xs()
                    .text_color(muted)
                    .child("Nothing shared yet."),
            );
        }
        list
    }
}
