//! A chat whose agent runs elsewhere attaches the servers its agent may sign in to, from the
//! attach menu, instead of this device's terminals; the attached ones show as chips.
use super::*;
use nocterm_ai::model::controls::{ConfigCategory, ConfigChoice, ConfigOption, ConfigSetting};

fn server(id: &str, name: &str, attached: bool) -> ConfigOption {
    ConfigOption::boolean(format!("server:{id}"), name, attached)
        .description(format!("root@{id}.example"))
        .category(ConfigCategory::Server)
}

fn hub_chat(f: &Fixture, cx: &mut TestAppContext) {
    f.connector.run_elsewhere();
    f.commands.config.set_options(vec![
        ConfigOption::select("model", "Model", "m1", vec![ConfigChoice::new("m1", "m1")])
            .category(ConfigCategory::Model),
        server("web", "Web", false),
        server("build", "Build", true),
    ]);
    new_chat(f, cx);
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_attach_menu_of_an_agent_that_runs_elsewhere_lists_the_chats_servers(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    hub_chat(&f, cx);
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        // Servers are not configured beside the model; one is attached already.
        assert!(window.try_find("config-picker-model").is_some());
        assert!(window.try_find("config-picker-server:web").is_none());
        assert!(window.try_find("server-chip-0").is_some());
        assert!(window.try_find("server-chip-1").is_none());
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        assert!(window.try_find(("attach-server", 0usize)).is_some());
        assert!(window.try_find(("attach-server", 1usize)).is_some());
        // None of this device's terminals or saved servers are offered.
        assert!(window.try_find("attach-group-homelab").is_none());
        window.click(("attach-server", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let request = f.commands.config.requests().pop().expect("a choice");
    assert_eq!(request.config.as_str(), "server:web");
    assert_eq!(request.setting, ConfigSetting::Boolean(true));
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("server-chip-1").is_some());
        // The menu stays open for more choices; once closed, a chip's cross detaches its server.
        assert!(window.try_find(("attach-server", 0usize)).is_some());
        window.click("agent-attach-context", cx);
        window.render_frame(cx);
        window.click("server-chip-0", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let request = f.commands.config.requests().pop().expect("a choice");
    assert_eq!(request.setting, ConfigSetting::Boolean(false));
}
