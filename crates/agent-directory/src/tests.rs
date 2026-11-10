use super::*;
use gpui::TestAppContext;
use nocterm_ai::{
    AgentServerSettings,
    agents::{AgentSource, LaunchDefaults},
};

const ALPHA: AgentProfile = AgentProfile::new(
    "alpha",
    "Alpha",
    LaunchDefaults::new("alpha", &["acp"], &[]),
)
.recognized_by(&["alpha"]);

fn handle(cx: &mut TestAppContext) -> AgentsHandle {
    cx.update(|cx| {
        let settings = nocterm_store::SettingsConfig::in_memory(Default::default()).install(cx);
        AgentsHandle::new(AgentCatalog::new([ALPHA]).unwrap(), settings, cx)
    })
}

#[gpui::test]
async fn available_agents_follow_the_current_settings(cx: &mut TestAppContext) {
    let agents = handle(cx);
    cx.update(|cx| {
        assert_eq!(
            agents.get("alpha", cx).unwrap().source(),
            AgentSource::Builtin
        );
        assert!(agents.get("custom", cx).is_none());
    });
    cx.update(|cx| {
        agents.settings.update::<AiSettings>(cx, |ai| {
            ai.agents.insert(
                "alpha".into(),
                AgentServerSettings {
                    enabled: false,
                    ..Default::default()
                },
            );
            ai.agents.insert(
                "custom".into(),
                AgentServerSettings {
                    command: Some("/opt/alpha/run".into()),
                    ..Default::default()
                },
            );
        })
    })
    .await
    .unwrap();
    cx.update(|cx| {
        assert!(agents.get("alpha", cx).is_none());
        assert_eq!(agents.profile("alpha", cx), Some(ALPHA));
        let custom = agents.get("custom", cx).unwrap();
        assert_eq!(custom.source(), AgentSource::Custom);
        assert_eq!(agents.profile("custom", cx), Some(ALPHA));
        assert_eq!(agents.catalog().iter().count(), 1);
    });
}

#[gpui::test]
async fn the_agents_of_signed_in_hubs_are_available_beside_the_local_ones(cx: &mut TestAppContext) {
    use nocterm_hubs_model::{HubAddress, HubId};
    let hub = HubId::from(HubAddress::parse("hub.example:7443").unwrap());
    let launch = nocterm_ai::agents::AgentLaunch::hub_agent(hub, "a1".into(), "Ops".into());
    let id = launch.id.clone();
    let agents = handle(cx).with_hub_agents(std::rc::Rc::new(move |_| vec![launch.clone()]));
    cx.update(|cx| {
        assert_eq!(agents.get(&id, cx).unwrap().source(), AgentSource::Hub);
        assert!(agents.get("alpha", cx).is_some());
    });
}
