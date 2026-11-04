//! What an engine's event stream means for its listings.
//!
//! An engine reports far more than what changes a listing: a health check
//! inside a container is an `exec_create`, `exec_start` and `exec_die` every
//! few seconds, and Podman repeats an unchanged health status after every
//! check. Relisting for each of them makes the daemon walk every container
//! and image again and again, so [`EventReader`] keeps only the events that
//! change what a listing shows, and says which of the two listings they
//! change. A line it cannot read changes both: being unsure costs a listing,
//! never a stale one.

use std::collections::HashMap;

use serde::Deserialize;

/// The longest event line kept; a longer one is dropped, as unreadable.
const MAX_LINE: usize = 64 * 1024;
/// The most containers whose health status is remembered at once.
const MAX_HEALTH: usize = 4096;

/// Container events that never change a listing: commands run inside a
/// container (health checks are these), terminals attached to it and files
/// copied from it.
const QUIET_CONTAINER: [&str; 16] = [
    "exec_create",
    "exec_start",
    "exec_die",
    "exec_detach",
    "exec",
    "exec_died",
    "attach",
    "detach",
    "resize",
    "top",
    "copy",
    "export",
    "archive-path",
    "extract-to-dir",
    "mount",
    "unmount",
];
/// Image events that never change a listing.
const QUIET_IMAGE: [&str; 4] = ["push", "save", "mount", "unmount"];
/// Image events that rename or remove an image a container may be shown by.
const RENAMING_IMAGE: [&str; 5] = ["tag", "untag", "delete", "remove", "prune"];
/// The container event that makes a new image of a container.
const COMMIT: &str = "commit";
const HEALTH: &str = "health_status";

/// Which listings an engine's events made stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    containers: bool,
    images: bool,
}

impl Changes {
    /// Nothing to list again.
    pub const NONE: Self = Self {
        containers: false,
        images: false,
    };
    /// Both listings.
    pub const ALL: Self = Self {
        containers: true,
        images: true,
    };
    pub const CONTAINERS: Self = Self {
        containers: true,
        images: false,
    };
    pub const IMAGES: Self = Self {
        containers: false,
        images: true,
    };

    pub fn containers(self) -> bool {
        self.containers
    }

    pub fn images(self) -> bool {
        self.images
    }

    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }
}

impl std::ops::BitOr for Changes {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self {
            containers: self.containers || other.containers,
            images: self.images || other.images,
        }
    }
}

impl std::ops::BitOrAssign for Changes {
    fn bitor_assign(&mut self, other: Self) {
        *self = *self | other;
    }
}

/// Reads the output of [`Engine::events`](crate::Engine::events), Docker's
/// or Podman's, in chunks as they arrive.
#[derive(Debug, Default)]
pub struct EventReader {
    /// The end of the last chunk, up to its first newline.
    partial: Vec<u8>,
    /// The rest of a line too long to keep is skipped.
    skipping: bool,
    /// The last health status heard of each container.
    health: HashMap<String, String>,
}

impl EventReader {
    /// The listings the complete lines of `chunk`, with what came before it,
    /// make stale.
    pub fn feed(&mut self, chunk: &[u8]) -> Changes {
        let mut changes = Changes::NONE;
        let mut rest = chunk;
        while let Some(end) = rest.iter().position(|&byte| byte == b'\n') {
            let (line, after) = rest.split_at(end);
            rest = &after[1..];
            if std::mem::take(&mut self.skipping) {
                self.partial.clear();
                continue;
            }
            if self.partial.is_empty() {
                changes |= self.line(line);
            } else {
                let mut whole = std::mem::take(&mut self.partial);
                whole.extend_from_slice(line);
                changes |= self.line(&whole);
            }
        }
        if !self.skipping {
            self.partial.extend_from_slice(rest);
            if self.partial.len() > MAX_LINE {
                self.partial = Vec::new();
                self.skipping = true;
                changes = Changes::ALL;
            }
        }
        changes
    }

    fn line(&mut self, line: &[u8]) -> Changes {
        if line.iter().all(u8::is_ascii_whitespace) {
            return Changes::NONE;
        }
        let Ok(event) = serde_json::from_slice::<Event>(line) else {
            return Changes::ALL;
        };
        let Some(action) = event.action() else {
            return Changes::ALL;
        };
        let (name, detail) = match action.split_once(':') {
            Some((name, detail)) => (name.trim(), Some(detail.trim())),
            None => (action.trim(), None),
        };
        match event.kind.as_str() {
            "container" => self.container(&event, name, detail),
            "image" => image(name),
            // Only containers and images are asked for.
            _ => Changes::NONE,
        }
    }

    fn container(&mut self, event: &Event, name: &str, detail: Option<&str>) -> Changes {
        let id = event.id();
        if name == HEALTH {
            let status = event.health.as_deref().or(detail);
            return match (id, status) {
                (Some(id), Some(status)) => self.health_changed(id, status),
                _ => Changes::CONTAINERS,
            };
        }
        if QUIET_CONTAINER.contains(&name) {
            return Changes::NONE;
        }
        // Whatever else happened to it, its next health status is news.
        if let Some(id) = id {
            self.health.remove(id);
        }
        if name == COMMIT {
            Changes::ALL
        } else {
            Changes::CONTAINERS
        }
    }

    fn health_changed(&mut self, id: &str, status: &str) -> Changes {
        if self.health.get(id).is_some_and(|known| known == status) {
            return Changes::NONE;
        }
        if self.health.len() >= MAX_HEALTH && !self.health.contains_key(id) {
            self.health.clear();
        }
        self.health.insert(id.to_owned(), status.to_owned());
        Changes::CONTAINERS
    }
}

fn image(name: &str) -> Changes {
    if QUIET_IMAGE.contains(&name) {
        Changes::NONE
    } else if RENAMING_IMAGE.contains(&name) {
        Changes::ALL
    } else {
        Changes::IMAGES
    }
}

/// One event, as either engine prints it: Docker names what happened
/// `Action` (with a deprecated `status`) and the object in `Actor`, Podman
/// names it `Status` with the object's `ID` and a separate `HealthStatus`.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "Type", default)]
    kind: String,
    #[serde(rename = "Action")]
    action: Option<String>,
    #[serde(rename = "Status")]
    podman_status: Option<String>,
    #[serde(rename = "status")]
    docker_status: Option<String>,
    #[serde(rename = "Actor")]
    actor: Option<Actor>,
    #[serde(rename = "ID")]
    podman_id: Option<String>,
    #[serde(rename = "id")]
    docker_id: Option<String>,
    #[serde(rename = "HealthStatus")]
    health: Option<String>,
}

#[derive(Deserialize)]
struct Actor {
    #[serde(rename = "ID")]
    id: Option<String>,
}

impl Event {
    fn action(&self) -> Option<&str> {
        [&self.action, &self.podman_status, &self.docker_status]
            .into_iter()
            .find_map(|value| value.as_deref().filter(|value| !value.trim().is_empty()))
    }

    fn id(&self) -> Option<&str> {
        self.actor
            .as_ref()
            .and_then(|actor| actor.id.as_deref())
            .or(self.podman_id.as_deref())
            .or(self.docker_id.as_deref())
            .filter(|id| !id.is_empty())
    }
}

#[cfg(test)]
mod tests;
