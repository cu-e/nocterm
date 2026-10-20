//! Conformance: every built-in profile satisfies these without being named.
use super::*;
use nocterm_ai::{AiSettings, AvailableAgents, agents::AgentSource};

#[test]
fn every_builtin_is_available_by_default_with_its_own_profile() {
    let catalog = catalog();
    let agents = AvailableAgents::resolve(&catalog, &AiSettings::default());
    assert!(agents.warnings().is_empty());
    for profile in PROFILES {
        let agent = agents.get(profile.id()).expect("built-in agent available");
        assert_eq!(agent.source(), AgentSource::Builtin);
        assert_eq!(agent.profile(), Some(profile));
        assert_eq!(agent.name(), profile.name());
    }
}

#[test]
fn package_runner_launches_pin_a_version() {
    for profile in PROFILES {
        let launch = profile.launch_defaults();
        if launch.command() == "npx" {
            let package = launch.args().last().expect("package argument");
            let (_, version) = package.rsplit_once('@').expect("pinned package");
            assert!(
                version.split('.').count() == 3
                    && version.split('.').all(|part| part.parse::<u32>().is_ok()),
                "{} pins {package}",
                profile.id()
            );
        }
    }
}

#[test]
fn every_builtin_has_a_one_colour_mark_under_a_unique_asset_path() {
    let mut assets = Vec::new();
    for profile in PROFILES {
        let mark = profile.mark().expect("brand mark");
        let svg = std::str::from_utf8(mark.svg()).expect("UTF-8 SVG");
        assert!(
            svg.contains("<svg") && svg.contains("currentColor"),
            "{}",
            profile.id()
        );
        assert!(mark.asset().starts_with("icons/agents/"));
        assert!(!assets.contains(&mark.asset()), "{}", mark.asset());
        assets.push(mark.asset());
    }
}

#[test]
fn markers_recognize_the_profile_and_no_other_builtin() {
    let catalog = catalog();
    for profile in PROFILES {
        for marker in profile.markers() {
            let mut launch = profile.default_launch();
            launch.id = "custom".into();
            launch.command = format!("/opt/{marker}");
            launch.args.clear();
            assert_eq!(catalog.for_launch(&launch), Some(profile), "{marker}");
        }
    }
}
