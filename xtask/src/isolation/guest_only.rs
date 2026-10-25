//! Guest-only crates: they build the plugin side of the code ABI and never ship in the
//! application, with or without the user-plugins feature.
use std::collections::BTreeSet;

use super::{graph::Graph, policy::Policy};

/// Names of the listed crates the application reaches through normal and build dependencies in
/// `graph`. A dev-dependency of the application does not count: it is not shipped.
///
/// A policy the workspace no longer matches is a problem as well: an application or a listed
/// crate that is not a workspace member (renamed or removed) would make the rule pass without
/// checking anything.
pub(super) fn problems(policy: &Policy, graph: &Graph, build: &str) -> BTreeSet<String> {
    let mut problems = BTreeSet::new();
    if !graph.is_member(&policy.app) {
        problems.insert(format!(
            "guest-only: the application `{}` is not a workspace package ({build}); \
             the rule would check nothing",
            policy.app
        ));
    }
    for listed in &policy.guest_only {
        if !graph.is_member(listed) {
            problems.insert(format!(
                "guest-only: `{listed}` is not a workspace package ({build}); \
                 rename it in `guest_only` of the policy or remove it"
            ));
        }
    }
    let reached = graph.member_closure(&policy.app).unwrap_or_default();
    problems.extend(
        reached
            .into_iter()
            .filter(|reached| policy.guest_only.iter().any(|listed| listed == reached))
            .map(|reached| {
                format!(
                    "guest-only: {} reaches {reached} ({build}), which is for plugin guests",
                    policy.app
                )
            }),
    );
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolation::tests::{graph, package};

    const POLICY: &str = r#"
[isolation]
gpui = "gpui-pre"
app = "nocterm"
guest_only = ["nocterm-plugin-sdk"]

[isolation.providers]
allowed = []

[isolation.compile_out]
crates = []
"#;

    fn policy() -> Policy {
        Policy::parse(POLICY).unwrap()
    }

    #[test]
    fn an_app_that_does_not_reach_a_guest_crate_passes() {
        let packages = vec![
            package("nocterm", &[("nocterm-ui", "")]),
            package("nocterm-ui", &[]),
            package("nocterm-plugin-sdk", &[]),
        ];
        assert!(problems(&policy(), &graph(packages), "default").is_empty());
    }

    #[test]
    fn a_guest_crate_in_the_app_closure_is_refused_even_through_another_crate() {
        let packages = vec![
            package("nocterm", &[("nocterm-ui", "")]),
            package("nocterm-ui", &[("nocterm-plugin-sdk", "")]),
            package("nocterm-plugin-sdk", &[]),
        ];
        let found = problems(&policy(), &graph(packages), "with user-plugins");
        assert_eq!(found.len(), 1);
        let text = found.iter().next().unwrap();
        assert!(text.contains("nocterm-plugin-sdk") && text.contains("with user-plugins"));
    }

    #[test]
    fn a_dev_dependency_of_the_app_is_not_shipped_and_passes() {
        let packages = vec![
            package("nocterm", &[("nocterm-plugin-sdk", "dev")]),
            package("nocterm-plugin-sdk", &[]),
        ];
        assert!(problems(&policy(), &graph(packages), "default").is_empty());
    }

    #[test]
    fn an_application_that_is_not_a_workspace_package_fails_instead_of_passing_vacuously() {
        let packages = vec![
            package("nocterm-renamed", &[("nocterm-plugin-sdk", "")]),
            package("nocterm-plugin-sdk", &[]),
        ];
        let found = problems(&policy(), &graph(packages), "default");
        assert!(
            found
                .iter()
                .any(|text| text.contains("application `nocterm` is not a workspace package")),
            "{found:?}"
        );
    }

    #[test]
    fn a_listed_crate_that_is_not_a_workspace_package_fails() {
        let packages = vec![package("nocterm", &[]), package("nocterm-guest-sdk", &[])];
        let found = problems(&policy(), &graph(packages), "default");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found
                .iter()
                .next()
                .unwrap()
                .contains("`nocterm-plugin-sdk` is not a workspace package")
        );
    }
}
