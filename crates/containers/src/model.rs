//! What a listing found.

/// Where a container is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
    /// A state this crate does not know, as the engine named it.
    Other(String),
}

impl State {
    pub(crate) fn parse(state: &str) -> Self {
        match state.trim().to_ascii_lowercase().as_str() {
            "created" | "configured" | "initialized" => Self::Created,
            "running" | "up" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "removing" | "stopping" => Self::Removing,
            "exited" | "stopped" => Self::Exited,
            "dead" => Self::Dead,
            _ => Self::Other(state.trim().to_owned()),
        }
    }

    /// Whether its processes run, paused or not: what stopping applies to.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Restarting)
    }
}

/// One container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: State,
    /// The engine's own summary, such as "Exited (0) 3 days ago".
    pub status: String,
    /// The Compose project it belongs to, if any.
    pub project: Option<String>,
    /// Its service within that project.
    pub service: Option<String>,
}

/// One image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub id: String,
    /// `None` for an untagged (dangling) image.
    pub repository: Option<String>,
    pub tag: Option<String>,
    /// Human-readable, such as "187MB".
    pub size: String,
    /// How long ago it was built, when the engine says.
    pub created: String,
}

impl Image {
    /// `repository:tag`, or the short id of an untagged image.
    pub fn name(&self) -> String {
        match (&self.repository, &self.tag) {
            (Some(repository), Some(tag)) => format!("{repository}:{tag}"),
            (Some(repository), None) => repository.clone(),
            _ => short_id(&self.id).to_owned(),
        }
    }
}

/// The first twelve hex digits of an id, as engines show them.
pub(crate) fn short_id(id: &str) -> &str {
    let id = id.strip_prefix("sha256:").unwrap_or(id);
    id.get(..12).unwrap_or(id)
}

/// Containers that belong together: a Compose project, or one container on
/// its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group<'a> {
    /// The project's name; `None` for a container on its own.
    pub project: Option<&'a str>,
    pub containers: Vec<&'a Container>,
}

impl Group<'_> {
    /// How many of its containers run.
    pub fn running(&self) -> usize {
        self.containers
            .iter()
            .filter(|container| container.state == State::Running)
            .count()
    }

    /// The ids of its containers, for an action on all of them.
    pub fn ids(&self) -> Vec<String> {
        self.containers
            .iter()
            .map(|container| container.id.clone())
            .collect()
    }
}

/// Everything a host's engine reported at once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub containers: Vec<Container>,
    pub images: Vec<Image>,
}

impl Snapshot {
    /// How many containers run.
    pub fn running(&self) -> usize {
        self.containers
            .iter()
            .filter(|container| container.state == State::Running)
            .count()
    }

    /// Compose projects first, by name, then the containers on their own;
    /// containers by name within each.
    pub fn groups(&self) -> Vec<Group<'_>> {
        let mut containers: Vec<&Container> = self.containers.iter().collect();
        containers.sort_by(|a, b| {
            (a.project.is_none(), &a.project, &a.name).cmp(&(
                b.project.is_none(),
                &b.project,
                &b.name,
            ))
        });
        let mut groups: Vec<Group<'_>> = Vec::new();
        for container in containers {
            match (container.project.as_deref(), groups.last_mut()) {
                (Some(project), Some(group)) if group.project == Some(project) => {
                    group.containers.push(container);
                }
                (project, _) => groups.push(Group {
                    project,
                    containers: vec![container],
                }),
            }
        }
        groups
    }

    pub fn container(&self, id: &str) -> Option<&Container> {
        self.containers.iter().find(|container| container.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn container(name: &str, project: Option<&str>, state: State) -> Container {
        Container {
            id: format!("{name}-id"),
            name: name.into(),
            image: "alpine".into(),
            state,
            status: String::new(),
            project: project.map(Into::into),
            service: None,
        }
    }

    #[test]
    fn groups_put_projects_first_and_keep_lone_containers_apart() {
        let snapshot = Snapshot {
            containers: vec![
                container("web", None, State::Running),
                container("db", Some("shop"), State::Running),
                container("api", Some("shop"), State::Exited),
                container("cache", None, State::Exited),
                container("app", Some("blog"), State::Paused),
            ],
            images: Vec::new(),
        };
        let groups = snapshot.groups();
        let shape: Vec<_> = groups
            .iter()
            .map(|group| {
                let names: Vec<_> = group.containers.iter().map(|c| c.name.as_str()).collect();
                (group.project, names)
            })
            .collect();
        assert_eq!(
            shape,
            [
                (Some("blog"), vec!["app"]),
                (Some("shop"), vec!["api", "db"]),
                (None, vec!["cache"]),
                (None, vec!["web"]),
            ]
        );
        assert_eq!(groups[1].running(), 1);
        assert_eq!(groups[1].ids(), ["api-id", "db-id"]);
        assert_eq!(snapshot.running(), 2);
    }

    #[test]
    fn states_are_read_from_either_engine() {
        assert_eq!(State::parse("running"), State::Running);
        assert_eq!(State::parse("Exited"), State::Exited);
        assert_eq!(State::parse("stopped"), State::Exited);
        assert_eq!(State::parse("odd"), State::Other("odd".into()));
        assert!(State::Paused.is_live());
        assert!(!State::Exited.is_live());
    }

    #[test]
    fn untagged_images_are_named_by_their_short_id() {
        let image = Image {
            id: "sha256:0123456789abcdef".into(),
            repository: None,
            tag: None,
            size: String::new(),
            created: String::new(),
        };
        assert_eq!(image.name(), "0123456789ab");
    }
}
