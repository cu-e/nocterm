//! Grouped attachment projection and explicit, independent server/group bindings.
use super::SnippetEditor;
use gpui_kit::{
    AnyElement, Context, SharedString,
    component::{
        ActiveTheme as _, Disableable as _, Icon, Sizable as _,
        button::{Button, ButtonVariants as _},
        checkbox::Checkbox,
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_ui::{IconName, form};
use nocterm_workspace::ConnectionSummary;

struct Folder {
    name: Option<SharedString>,
    servers: Vec<ConnectionSummary>,
}
fn project(connections: &[ConnectionSummary], groups: &[SharedString], query: &str) -> Vec<Folder> {
    let query = query.trim().to_lowercase();
    let mut names: Vec<_> = groups
        .iter()
        .cloned()
        .chain(connections.iter().filter_map(|server| server.group.clone()))
        .collect();
    names.sort();
    names.dedup();
    let names = names.into_iter().map(Some).chain(
        connections
            .iter()
            .any(|server| server.group.is_none())
            .then_some(None),
    );
    names
        .filter_map(|name| {
            let group_matches = name
                .as_ref()
                .is_some_and(|name| name.to_lowercase().contains(&query));
            let mut servers: Vec<_> = connections
                .iter()
                .filter(|server| server.group == name)
                .filter(|server| {
                    query.is_empty()
                        || group_matches
                        || server.name.to_lowercase().contains(&query)
                        || server.target.host.to_lowercase().contains(&query)
                })
                .cloned()
                .collect();
            servers.sort_by(|a, b| a.name.cmp(&b.name));
            (query.is_empty() || group_matches || !servers.is_empty())
                .then_some(Folder { name, servers })
        })
        .collect()
}
fn folder_id(group: Option<&str>) -> String {
    group.map_or_else(
        || "snippet-folder-unassigned".to_owned(),
        |name| format!("snippet-folder-group-{name}"),
    )
}
impl SnippetEditor {
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn attachments_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let directory = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).connection_directory());
        let connections = directory
            .as_ref()
            .map_or_else(Vec::new, |directory| directory.connections(cx));
        let groups = directory
            .as_ref()
            .map_or_else(Vec::new, |directory| directory.groups(cx));
        let folders = project(
            &connections,
            &groups,
            &self.attachment_search.read(cx).value(),
        );
        let no_matches =
            folders.is_empty() && !self.attachment_search.read(cx).value().trim().is_empty();
        let mut tree = v_flex().gap_1();
        for folder in folders {
            let key = folder.name.as_ref().map(ToString::to_string);
            let folded = self.folded_groups.contains(&key);
            let inherited = key
                .as_ref()
                .is_some_and(|name| self.draft.groups.contains(name));
            let label = folder.name.clone().unwrap_or_else(|| "Ungrouped".into());
            let count = folder.servers.len();
            let fold_key = key.clone();
            let mut heading = h_flex()
                .gap_2()
                .py_1()
                .child(
                    Button::new(SharedString::from(folder_id(key.as_deref())))
                        .ghost()
                        .xsmall()
                        .icon(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .tooltip(if folded {
                            "Expand group"
                        } else {
                            "Collapse group"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.folded_groups.remove(&fold_key) {
                                this.folded_groups.insert(fold_key.clone());
                            }
                            cx.notify();
                        })),
                )
                .child(Icon::new(IconName::FolderTree).small());
            if let Some(group) = key {
                heading = heading.child(
                    Checkbox::new(SharedString::from(format!("group-{group}")))
                        .small()
                        .label(label)
                        .checked(inherited)
                        .disabled(self.pending)
                        .tooltip("Attach to this group, including servers added to it later")
                        .on_click(cx.listener(move |this, checked, _, cx| {
                            this.set_binding(true, group.clone(), *checked, cx)
                        })),
                );
            } else {
                heading = heading.child(div().text_sm().child(label));
            }
            heading = heading.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(count.to_string()),
            );
            let mut branch = v_flex().child(heading);
            if !folded {
                for server in folder.servers {
                    let id = server.id.to_string();
                    let checked = self.draft.profiles.contains(&id);
                    let name = if inherited {
                        format!("{} · via group", server.name)
                    } else {
                        server.name.to_string()
                    };
                    branch = branch.child(
                        div().pl_8().py_1().child(
                            Checkbox::new(SharedString::from(format!("profile-{id}")))
                                .small()
                                .label(name)
                                .checked(checked)
                                .disabled(self.pending)
                                .tooltip(format!(
                                    "{} — attach directly to this server",
                                    server.target.host
                                ))
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    this.set_binding(false, id.clone(), *checked, cx)
                                })),
                        ),
                    );
                }
            }
            tree = tree.child(branch);
        }
        let missing_profiles: Vec<_> = self
            .draft
            .profiles
            .iter()
            .filter(|id| {
                !connections
                    .iter()
                    .any(|server| server.id.as_ref() == id.as_str())
            })
            .cloned()
            .collect();
        let missing_groups: Vec<_> = self
            .draft
            .groups
            .iter()
            .filter(|name| {
                !groups.iter().any(|group| group.as_ref() == name.as_str())
                    && !connections.iter().any(|server| {
                        server
                            .group
                            .as_ref()
                            .is_some_and(|group| group.as_ref() == name.as_str())
                    })
            })
            .cloned()
            .collect();
        let missing = !missing_profiles.is_empty() || !missing_groups.is_empty();
        let mut unavailable = v_flex().gap_2();
        for id in missing_profiles {
            unavailable = unavailable.child(
                Checkbox::new(SharedString::from(format!("profile-{id}")))
                    .small()
                    .label(format!("Unavailable server ({id})"))
                    .checked(true)
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, checked, _, cx| {
                        this.set_binding(false, id.clone(), *checked, cx)
                    })),
            );
        }
        for name in missing_groups {
            unavailable = unavailable.child(
                Checkbox::new(SharedString::from(format!("group-{name}")))
                    .small()
                    .label(format!("Unavailable group: {name}"))
                    .checked(true)
                    .disabled(self.pending)
                    .on_click(cx.listener(move |this, checked, _, cx| {
                        this.set_binding(true, name.clone(), *checked, cx)
                    })),
            );
        }
        v_flex().gap_3()
            .child(form::page_header("Attachments", Some("Group bindings include future servers. Direct server bindings are independent.".into()), cx))
            .child(Input::new(&self.attachment_search).small())
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{} direct server{}, {} group{} selected", self.draft.profiles.len(), if self.draft.profiles.len() == 1 { "" } else { "s" }, self.draft.groups.len(), if self.draft.groups.len() == 1 { "" } else { "s" })))
            .when(connections.is_empty() && groups.is_empty(), |page| page.child(form::note("No saved servers or groups. You can save this snippet without attachments.", cx)))
            .when(no_matches, |page| page.child(form::note("No matching servers or groups.", cx)))
            .child(tree)
            .when(missing, |page| page.child(div().pt_3().text_sm().child("Unavailable attachments")).child(form::note("Uncheck a binding to remove it. Missing bindings keep their scope.", cx)).child(unavailable))
            .into_any_element()
    }
}
#[cfg(test)]
mod tests;
