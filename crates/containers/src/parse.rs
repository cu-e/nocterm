//! The engines' JSON listings.
//!
//! Docker prints one object per line; Podman prints one array. Their field
//! names differ too (`ID` and a comma-separated `Labels` string against
//! `Id` and a `Labels` object, `Names` as a string against an array), so
//! fields are read loosely rather than into fixed structs.

use serde_json::{Map, Value};

use crate::model::{Container, Image, State};

/// Compose project labels: Docker Compose's, then podman-compose's.
const PROJECT_LABELS: [&str; 2] = ["com.docker.compose.project", "io.podman.compose.project"];
const SERVICE_LABELS: [&str; 2] = ["com.docker.compose.service", "io.podman.compose.service"];

pub(crate) fn containers(output: &str) -> Result<Vec<Container>, String> {
    objects(output)?
        .iter()
        .map(container)
        .collect::<Option<_>>()
        .ok_or_else(|| "the engine listed a container without an id".into())
}

pub(crate) fn images(output: &str) -> Result<Vec<Image>, String> {
    objects(output)?
        .iter()
        .map(image)
        .collect::<Option<_>>()
        .ok_or_else(|| "the engine listed an image without an id".into())
}

/// The objects of a listing in either shape.
fn objects(output: &str) -> Result<Vec<Map<String, Value>>, String> {
    let output = output.trim();
    let values = if output.starts_with('[') {
        match serde_json::from_str(output) {
            Ok(Value::Array(values)) => values,
            _ => return Err(unreadable(output)),
        }
    } else {
        output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).map_err(|_| unreadable(line)))
            .collect::<Result<_, _>>()?
    };
    values
        .into_iter()
        .map(|value| match value {
            Value::Object(object) => Ok(object),
            other => Err(unreadable(&other.to_string())),
        })
        .collect()
}

fn unreadable(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    format!("the engine's listing could not be read: {line}")
}

fn container(object: &Map<String, Value>) -> Option<Container> {
    let labels = labels(object.get("Labels"));
    let label = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| labels.iter().find(|(name, _)| name == key))
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    };
    let state = State::parse(&text(object, "State").unwrap_or_default());
    let status = text(object, "Status")
        .filter(|status| !status.is_empty())
        .unwrap_or_else(|| podman_status(object, &state));
    Some(Container {
        id: text(object, "ID").or_else(|| text(object, "Id"))?,
        name: first(object.get("Names"))
            .map(|name| name.trim_start_matches('/').to_owned())
            .unwrap_or_default(),
        image: text(object, "Image").unwrap_or_default(),
        state,
        status,
        project: label(&PROJECT_LABELS),
        service: label(&SERVICE_LABELS),
    })
}

/// Podman's listing leaves the summary to its templates; this is the part of
/// it the listing carries.
fn podman_status(object: &Map<String, Value>, state: &State) -> String {
    match state {
        State::Running => "Up".into(),
        State::Exited => match object.get("ExitCode").and_then(Value::as_i64) {
            Some(code) => format!("Exited ({code})"),
            None => "Exited".into(),
        },
        State::Other(other) => other.clone(),
        other => format!("{other:?}"),
    }
}

fn image(object: &Map<String, Value>) -> Option<Image> {
    let id = text(object, "ID").or_else(|| text(object, "Id"))?;
    let (repository, tag) = match text(object, "Repository") {
        Some(repository) => (Some(repository), text(object, "Tag")),
        None => match first(object.get("RepoTags")).or_else(|| first(object.get("Names"))) {
            Some(name) => match name.rsplit_once(':') {
                Some((repository, tag)) if !tag.contains('/') => {
                    (Some(repository.to_owned()), Some(tag.to_owned()))
                }
                _ => (Some(name), None),
            },
            None => (None, None),
        },
    };
    let untagged = |part: &Option<String>| part.as_deref().is_none_or(|part| part == "<none>");
    let (repository, tag) = if untagged(&repository) {
        (None, None)
    } else if untagged(&tag) {
        (repository, None)
    } else {
        (repository, tag)
    };
    let size = match object.get("Size") {
        Some(Value::Number(bytes)) => bytes.as_u64().map(human_size).unwrap_or_default(),
        Some(Value::String(size)) => size.clone(),
        _ => String::new(),
    };
    Some(Image {
        id,
        repository,
        tag,
        size,
        created: text(object, "CreatedSince").unwrap_or_default(),
    })
}

fn text(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key)?.as_str().map(str::to_owned)
}

/// A string, or the first string of an array.
fn first(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.split(',').next().unwrap_or(text).to_owned()),
        Value::Array(values) => values.first()?.as_str().map(str::to_owned),
        _ => None,
    }
}

/// Labels as an object, or as Docker's `key=value,key=value` string.
fn labels(value: Option<&Value>) -> Vec<(String, String)> {
    match value {
        Some(Value::Object(labels)) => labels
            .iter()
            .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
            .collect(),
        Some(Value::String(labels)) => labels
            .split(',')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Bytes the way the engines print them: decimal units, three digits.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit + 1 < UNITS.len() {
        size /= 1000.0;
        unit += 1;
    }
    let precision = match size {
        _ if unit == 0 => 0,
        size if size >= 100.0 => 0,
        size if size >= 10.0 => 1,
        _ => 2,
    };
    let digits = format!("{size:.precision$}");
    let digits = match digits.contains('.') {
        true => digits.trim_end_matches('0').trim_end_matches('.'),
        false => &digits,
    };
    format!("{digits}{}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_listings_are_one_object_a_line() {
        let output = concat!(
            r#"{"ID":"a1","Image":"postgres:16","Labels":"com.docker.compose.project=shop,com.docker.compose.service=db","Names":"shop-db-1","State":"running","Status":"Up 2 hours"}"#,
            "\n",
            r#"{"ID":"b2","Image":"alpine","Labels":"","Names":"scratch","State":"exited","Status":"Exited (0) 3 days ago"}"#,
            "\n",
        );
        let listed = containers(output).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "a1");
        assert_eq!(listed[0].name, "shop-db-1");
        assert_eq!(listed[0].state, State::Running);
        assert_eq!(listed[0].project.as_deref(), Some("shop"));
        assert_eq!(listed[0].service.as_deref(), Some("db"));
        assert_eq!(listed[1].project, None);
        assert_eq!(listed[1].status, "Exited (0) 3 days ago");
    }

    #[test]
    fn podman_listings_are_one_array() {
        let output = r#"[
            {"Id":"c3","Image":"docker.io/library/nginx:latest","Names":["web"],
             "Labels":{"io.podman.compose.project":"site"},"State":"exited","ExitCode":137},
            {"Id":"d4","Image":"redis","Names":["cache"],"Labels":null,"State":"running"}
        ]"#;
        let listed = containers(output).unwrap();
        assert_eq!(listed[0].name, "web");
        assert_eq!(listed[0].project.as_deref(), Some("site"));
        assert_eq!(listed[0].status, "Exited (137)");
        assert_eq!(listed[1].status, "Up");
        assert_eq!(containers("").unwrap(), []);
        assert_eq!(containers("[]").unwrap(), []);
    }

    #[test]
    fn listings_that_are_not_json_are_an_error() {
        let error = containers("json\njson\n").unwrap_err();
        assert!(error.contains("json"), "{error}");
        assert!(containers(r#"{"Names":"no-id"}"#).is_err());
    }

    #[test]
    fn images_are_read_from_either_engine() {
        let docker = concat!(
            r#"{"ID":"sha256:1","Repository":"alpine","Tag":"3.20","Size":"7.8MB","CreatedSince":"2 weeks ago"}"#,
            "\n",
            r#"{"ID":"sha256:2","Repository":"<none>","Tag":"<none>","Size":"1GB"}"#,
        );
        let listed = images(docker).unwrap();
        assert_eq!(listed[0].name(), "alpine:3.20");
        assert_eq!(listed[0].created, "2 weeks ago");
        assert_eq!(listed[1].repository, None);
        let podman = r#"[{"Id":"e5","RepoTags":["localhost:5000/app:dev"],"Size":187654321},
                         {"Id":"f6","RepoTags":null,"Size":999}]"#;
        let listed = images(podman).unwrap();
        assert_eq!(listed[0].repository.as_deref(), Some("localhost:5000/app"));
        assert_eq!(listed[0].tag.as_deref(), Some("dev"));
        assert_eq!(listed[0].size, "188MB");
        assert_eq!(listed[1].repository, None);
        assert_eq!(listed[1].size, "999B");
    }

    #[test]
    fn sizes_keep_three_digits() {
        assert_eq!(human_size(7_812_345), "7.81MB");
        assert_eq!(human_size(12_345_678), "12.3MB");
        assert_eq!(human_size(1_000_000_000), "1GB");
    }
}
