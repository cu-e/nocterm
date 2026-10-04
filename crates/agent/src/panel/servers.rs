//! The saved servers in the attach menu, laid out as in the sidebar: servers
//! outside folders first, then each folder with its servers under it.

use nocterm_workspace::ConnectionSummary;

/// One row of the attach menu's server list.
#[derive(Debug)]
pub(super) enum ServerRow<'a> {
    /// A folder: attaching it attaches every server in it.
    Folder(&'a str),
    Server {
        summary: &'a ConnectionSummary,
        /// Filed in the folder above.
        nested: bool,
    },
}

/// The rows for `summaries`, folders and servers sorted by name.
pub(super) fn server_rows(summaries: &[ConnectionSummary]) -> Vec<ServerRow<'_>> {
    let key = |name: &str| (name.to_lowercase(), name.to_owned());
    let mut servers: Vec<&ConnectionSummary> = summaries.iter().collect();
    servers.sort_by_cached_key(|summary| {
        (
            summary.group.as_deref().map(key),
            key(summary.name.as_ref()),
        )
    });
    let mut rows = Vec::with_capacity(servers.len());
    let mut folder: Option<&str> = None;
    for summary in servers {
        let group = summary.group.as_deref();
        if let Some(group) = group
            && folder != Some(group)
        {
            rows.push(ServerRow::Folder(group));
        }
        folder = group;
        rows.push(ServerRow::Server {
            summary,
            nested: group.is_some(),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(name: &str, group: Option<&str>) -> ConnectionSummary {
        ConnectionSummary {
            id: name.to_owned().into(),
            name: name.to_owned().into(),
            group: group.map(|group| group.to_owned().into()),
            description: Default::default(),
            target: serde_json::from_value(
                serde_json::json!({"host": "example.test", "port": 22, "user": "root"}),
            )
            .unwrap(),
            icon: None,
            flag: None,
        }
    }

    #[test]
    fn servers_outside_folders_come_first_then_each_folder_with_its_servers() {
        let summaries = [
            summary("proxmox", Some("homelab")),
            summary("luxVPC", None),
            summary("web", Some("Clients")),
            summary("ai-server", Some("homelab")),
            summary("backup", None),
        ];
        let rows: Vec<String> = server_rows(&summaries)
            .into_iter()
            .map(|row| match row {
                ServerRow::Folder(name) => format!("[{name}]"),
                ServerRow::Server { summary, nested } => {
                    format!("{}{}", if nested { "  " } else { "" }, summary.name)
                }
            })
            .collect();
        assert_eq!(
            rows,
            [
                "backup",
                "luxVPC",
                "[Clients]",
                "  web",
                "[homelab]",
                "  ai-server",
                "  proxmox",
            ]
        );
    }
}
