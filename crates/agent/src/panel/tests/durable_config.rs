//! A chat's configuration is its own: it outlives the agent session, is
//! saved with the chat and is given to every session the chat opens.
use super::*;
use crate::thread::AgentThread;
use nocterm_ai::session_config::ConfigValue;
use std::time::Duration;

fn models() -> Vec<acp::SessionConfigOption> {
    vec![
        acp::SessionConfigOption::select(
            "model",
            "Model",
            "sonnet",
            vec![
                acp::SessionConfigSelectOption::new("sonnet", "Sonnet"),
                acp::SessionConfigSelectOption::new("opus", "Opus"),
            ],
        )
        .category(acp::SessionConfigOptionCategory::Model),
    ]
}
fn chat(f: &Fixture, cx: &mut TestAppContext) -> Entity<AgentThread> {
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.new_thread("codex".into(), window, cx));
        f.panel.read(cx).current().unwrap()
    })
    .unwrap()
}
fn choose(thread: &Entity<AgentThread>, model: &str, cx: &mut TestAppContext) {
    thread.update(cx, |thread, cx| {
        thread.set_config(
            acp::SessionConfigId::new("model"),
            acp::SessionConfigOptionValue::value_id(model.to_owned()),
            cx,
        )
    });
    cx.run_until_parked();
}
fn current_model(thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| crate::thread::config_label(&thread.read(cx).state.config_options[0]))
}
fn tick(cx: &mut TestAppContext, seconds: u64) {
    cx.background_executor
        .advance_clock(Duration::from_secs(seconds));
    cx.run_until_parked();
}

#[gpui_kit::test]
async fn a_chat_keeps_its_model_after_its_session_sleeps(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| {
            settings.sessions.idle_timeout_secs = 2
        })
    })
    .await
    .unwrap();
    *f.commands.config.lock().unwrap() = models();
    let first = chat(&f, cx);
    cx.run_until_parked();
    choose(&first, "opus", cx);
    exchange(&f, "hello", "Hi.", cx);
    let id = cx.update(|cx| first.read(cx).chat_id.clone());

    // Another chat of the agent picks another model, which becomes the
    // agent's default; the first chat keeps its own.
    let second = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(current_model(&second, cx), "Opus");
    choose(&second, "sonnet", cx);
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| assert!(first.read(cx).lease.is_none(), "the hidden chat slept"));
    assert_eq!(current_model(&first, cx), "Opus");
    let saved = cx.update(|cx| {
        let dir = Runtime::global(cx).read(cx).services.chats_dir.clone();
        nocterm_ai::history::load(&dir, &id).unwrap()
    });
    assert_eq!(
        saved.config.get("model"),
        Some(&ConfigValue::Value("opus".into()))
    );

    // A new process starts with the agent's defaults again.
    *f.commands.config.lock().unwrap() = models();
    let requests = f.commands.config_requests.lock().unwrap().len();
    f.panel
        .update(cx, |panel, cx| panel.open_thread(first.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(first.read(cx).session().is_some()));
    assert_eq!(current_model(&first, cx), "Opus");
    let asked = f.commands.config_requests.lock().unwrap()[requests..].to_vec();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        ConfigValue::from_acp(&asked[0].value),
        Some(ConfigValue::Value("opus".into()))
    );
    exchange(&f, "and now?", "Still Opus.", cx);
    cx.update(|cx| assert_eq!(first.read(cx).model().as_deref(), Some("Opus")));
}

#[gpui_kit::test]
fn a_saved_chat_shows_its_own_model_before_it_connects(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(cx);
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.remember_options("codex", &models(), cx)
        })
    });
    let mut saved = nocterm_ai::history::SavedChat::new("codex".into());
    saved
        .entries
        .push(nocterm_ai::thread::Entry::Agent("earlier answer".into()));
    saved
        .config
        .insert("model".into(), ConfigValue::Value("opus".into()));
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, _| runtime.saved_chats = Some(vec![saved]))
    });
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).threads[0].clone());
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    assert_eq!(current_model(&thread, cx), "Opus");
}
