//! Checked mapping of own cgroup membership into one visible controller mount.
/// Ambiguous/malformed/escaped mounts never select a parent-root fallback.
pub(super) fn resolve(members: &[&str], mounts: &[&str], controller: &str) -> Option<String> {
    if members.len() > 256 || mounts.len() > 4096 {
        return None;
    }
    let memberships: Vec<_> = members
        .iter()
        .filter_map(|line| {
            let mut fields = line.splitn(3, ':');
            let hierarchy = fields.next()?;
            let controllers = fields.next()?;
            let path = fields.next()?;
            ((controller == "v2" && hierarchy == "0" && controllers.is_empty())
                || controllers.split(',').any(|item| item == controller))
            .then_some(path)
        })
        .collect();
    if memberships.len() != 1 {
        return None;
    }
    let mut found = None;
    for line in mounts {
        if line.len() > 16384 {
            return None;
        }
        let (before, after) = line.split_once(" - ")?;
        let fields: Vec<_> = before.split_whitespace().collect();
        let filesystem: Vec<_> = after.split_whitespace().collect();
        if fields.len() < 6 || filesystem.len() < 3 {
            return None;
        }
        let matches = if controller == "v2" {
            filesystem[0] == "cgroup2"
        } else {
            filesystem[0] == "cgroup" && filesystem[2].split(',').any(|item| item == controller)
        };
        if !matches {
            continue;
        }
        if found.is_some() {
            return None;
        }
        let root = decode(fields[3])?;
        let mount = decode(fields[4])?;
        let member = memberships[0];
        if !valid(member) || !valid(&root) || !valid(&mount) {
            return None;
        }
        let relative = if root == "/" {
            member
        } else if member == root {
            "/"
        } else {
            member
                .strip_prefix(&format!("{root}/"))
                .map_or(member, |rest| rest)
        };
        let joined = if relative == "/" {
            mount
        } else {
            format!(
                "{}/{}",
                mount.trim_end_matches('/'),
                relative.trim_start_matches('/')
            )
        };
        found = Some(joined);
    }
    found
}
fn valid(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4096
        && !path.contains(['\\', '\0', '\n', '\r'])
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}
fn decode(value: &str) -> Option<String> {
    let value = value.replace("\\040", " ");
    valid(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn mount(root: &str, mount: &str, fs: &str, options: &str) -> String {
        format!("1 2 0:1 {root} {mount} ro - {fs} cgroup {options}")
    }
    #[test]
    fn namespace_root_and_subtree_mounts_resolve_the_actual_membership() {
        for (member, root, expected) in [
            ("/", "/docker/id", "/cg"),
            ("/child", "/docker/id", "/cg/child"),
            ("/docker/id/child", "/docker/id", "/cg/child"),
            ("/docker/id", "/", "/cg/docker/id"),
        ] {
            assert_eq!(
                resolve(
                    &[&format!("0::{member}")],
                    &[&mount(root, "/cg", "cgroup2", "rw")],
                    "v2"
                )
                .as_deref(),
                Some(expected)
            );
        }
    }
    #[test]
    fn separate_v1_controllers_and_escaped_spaces_are_exact() {
        let mounts = [
            mount("/", "/cpu\\040group", "cgroup", "rw,cpu,cpuacct"),
            mount("/", "/memory", "cgroup", "rw,memory"),
        ];
        let mounts: Vec<_> = mounts.iter().map(String::as_str).collect();
        let members = ["2:cpu,cpuacct:/a", "3:memory:/b"];
        assert_eq!(
            resolve(&members, &mounts, "cpuacct").as_deref(),
            Some("/cpu group/a")
        );
        assert_eq!(
            resolve(&members, &mounts, "memory").as_deref(),
            Some("/memory/b")
        );
    }
    #[test]
    fn ambiguous_missing_and_outside_namespace_memberships_are_refused() {
        let one = mount("/", "/cg", "cgroup2", "rw");
        assert!(resolve(&["0::/a"], &[&one, &one], "v2").is_none());
        assert!(resolve(&["0::/a", "0::/b"], &[&one], "v2").is_none());
        assert!(resolve(&["0::/../outside"], &[&one], "v2").is_none());
        assert!(resolve(&["0::/a"], &[], "v2").is_none());
    }
}
