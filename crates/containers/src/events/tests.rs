use super::*;

/// A Docker event line, as `docker events --format '{{json .}}'` prints it.
fn docker(kind: &str, action: &str, id: &str) -> String {
    format!(
        r#"{{"status":"{action}","id":"{id}","Type":"{kind}","Action":"{action}","Actor":{{"ID":"{id}","Attributes":{{"name":"pg"}}}},"scope":"local","time":1}}"#
    ) + "\n"
}

/// A Podman event line, as `podman events --format json` prints it.
fn podman(action: &str, id: &str, health: Option<&str>) -> String {
    let health = health.map_or(String::new(), |health| {
        format!(r#","HealthStatus":"{health}""#)
    });
    format!(r#"{{"ID":"{id}","Name":"pg","Status":"{action}","Type":"container"{health}}}"#) + "\n"
}

fn feed(reader: &mut EventReader, line: &str) -> Changes {
    reader.feed(line.as_bytes())
}

#[test]
fn health_check_commands_change_no_listing() {
    let mut reader = EventReader::default();
    for action in [
        "exec_create: pg_isready -U nocterm",
        "exec_start: pg_isready -U nocterm",
        "exec_die",
    ] {
        assert_eq!(
            feed(&mut reader, &docker("container", action, "c1")),
            Changes::NONE
        );
    }
    for action in ["exec", "exec_died"] {
        assert_eq!(
            feed(&mut reader, &podman(action, "c1", None)),
            Changes::NONE
        );
    }
}

#[test]
fn container_life_changes_only_the_container_listing() {
    let mut reader = EventReader::default();
    for action in [
        "create", "start", "die", "stop", "pause", "destroy", "rename",
    ] {
        assert_eq!(
            feed(&mut reader, &docker("container", action, "c1")),
            Changes::CONTAINERS,
            "{action}"
        );
    }
    assert_eq!(
        feed(&mut reader, &podman("died", "c1", None)),
        Changes::CONTAINERS
    );
    assert_eq!(
        feed(&mut reader, &docker("container", "commit", "c1")),
        Changes::ALL,
        "a commit makes an image"
    );
}

#[test]
fn image_events_change_the_image_listing_and_renames_both() {
    let mut reader = EventReader::default();
    assert_eq!(
        feed(&mut reader, &docker("image", "pull", "alpine:3")),
        Changes::IMAGES
    );
    assert_eq!(
        feed(&mut reader, &docker("image", "push", "alpine:3")),
        Changes::NONE
    );
    for action in ["tag", "untag", "delete"] {
        assert_eq!(
            feed(&mut reader, &docker("image", action, "sha256:1")),
            Changes::ALL,
            "containers may be shown by the image's name: {action}"
        );
    }
    assert_eq!(
        feed(&mut reader, &docker("network", "connect", "n1")),
        Changes::NONE
    );
}

#[test]
fn only_a_new_health_status_changes_the_listing() {
    let mut reader = EventReader::default();
    let healthy = docker("container", "health_status: healthy", "c1");
    assert_eq!(feed(&mut reader, &healthy), Changes::CONTAINERS);
    assert_eq!(feed(&mut reader, &healthy), Changes::NONE);
    assert_eq!(
        feed(
            &mut reader,
            &docker("container", "health_status: healthy", "c2")
        ),
        Changes::CONTAINERS,
        "another container's status is its own"
    );
    assert_eq!(
        feed(
            &mut reader,
            &docker("container", "health_status: unhealthy", "c1")
        ),
        Changes::CONTAINERS
    );

    // Podman repeats the status after every check.
    let repeated = podman("health_status", "p1", Some("healthy"));
    assert_eq!(feed(&mut reader, &repeated), Changes::CONTAINERS);
    assert_eq!(feed(&mut reader, &repeated), Changes::NONE);
    // A restart resets the status the listing shows, so the same one is news again.
    assert_eq!(
        feed(&mut reader, &podman("restart", "p1", None)),
        Changes::CONTAINERS
    );
    assert_eq!(feed(&mut reader, &repeated), Changes::CONTAINERS);
    // Without a status to compare, the listing is the only way to know.
    let unknown = podman("health_status", "p1", None);
    assert_eq!(feed(&mut reader, &unknown), Changes::CONTAINERS);
}

#[test]
fn lines_split_across_chunks_are_read_whole() {
    let mut reader = EventReader::default();
    let line = docker("container", "start", "c1");
    let (head, tail) = line.split_at(line.len() / 2);
    assert_eq!(feed(&mut reader, head), Changes::NONE);
    assert_eq!(feed(&mut reader, tail), Changes::CONTAINERS);

    let quiet = docker("container", "exec_die", "c1");
    let both = format!("{quiet}{}", docker("image", "pull", "alpine"));
    assert_eq!(feed(&mut reader, &both), Changes::IMAGES);
}

#[test]
fn unreadable_lines_relist_everything() {
    let mut reader = EventReader::default();
    assert_eq!(feed(&mut reader, "\n  \n"), Changes::NONE);
    assert_eq!(feed(&mut reader, "container die\n"), Changes::ALL);
    assert_eq!(
        feed(&mut reader, "{\"Type\":\"container\"}\n"),
        Changes::ALL
    );

    let long = "x".repeat(MAX_LINE + 1);
    assert_eq!(feed(&mut reader, &long), Changes::ALL);
    assert_eq!(
        feed(&mut reader, "rest of it\n"),
        Changes::NONE,
        "the rest of a dropped line is skipped"
    );
    assert_eq!(
        feed(&mut reader, &docker("container", "start", "c1")),
        Changes::CONTAINERS
    );
}

#[test]
fn changes_combine() {
    assert_eq!(Changes::CONTAINERS | Changes::IMAGES, Changes::ALL);
    let mut changes = Changes::NONE;
    assert!(changes.is_empty());
    changes |= Changes::IMAGES;
    assert!(changes.images() && !changes.containers());
}
