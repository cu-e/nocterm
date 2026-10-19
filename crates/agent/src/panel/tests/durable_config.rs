//! A chat's configuration is its own: it outlives the agent session, is
//! saved with the chat and is given to every session the chat opens.
use super::*;
use crate::thread::AgentThread;
use nocterm_ai::session_config::ConfigValue;
use std::time::Duration;

fn models() -> Vec<ConfigOption> {
    use nocterm_ai::model::{
        controls::{ConfigCategory, ConfigChoice, ConfigKind, SelectChoices},
        ids::{ConfigChoiceId, ConfigId},
    };
    vec![ConfigOption {
        id: ConfigId::new("model"),
        name: "Model".into(),
        description: None,
        category: Some(ConfigCategory::Model),
        extensions: None,
        kind: ConfigKind::Select {
            current: ConfigChoiceId::new("sonnet"),
            choices: SelectChoices::Flat(vec![
                ConfigChoice {
                    id: ConfigChoiceId::new("sonnet"),
                    name: "Sonnet".into(),
                    description: None,
                    extensions: None,
                },
                ConfigChoice {
                    id: ConfigChoiceId::new("opus"),
                    name: "Opus".into(),
                    description: None,
                    extensions: None,
                },
            ]),
        },
    }]
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
            nocterm_ai::model::ids::ConfigId::new("model"),
            ConfigValue::Value(model.to_owned()),
            cx,
        )
    });
    cx.run_until_parked();
}
fn current_model(thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| crate::thread::config_label(&thread.read(cx).config_options()[0]))
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
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| {
                settings.sessions.idle_timeout_secs = 2
            })
    })
    .await
    .unwrap();
    f.commands.config.set_options(models());
    let first = chat(&f, cx);
    cx.run_until_parked();
    choose(&first, "opus", cx);
    exchange(&f, "hello", "Hi.", cx);
    let id = cx.update(|cx| first.read(cx).chat_id_string());

    // Another chat of the agent picks another model, which becomes the
    // agent's default; the first chat keeps its own.
    let second = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(current_model(&second, cx), "Opus");
    choose(&second, "sonnet", cx);
    tick(cx, 1);
    tick(cx, 3);
    cx.update(|cx| assert!(!first.read(cx).is_leased(), "the hidden chat slept"));
    assert_eq!(current_model(&first, cx), "Opus");
    let saved = cx.update(|_| f.repository.transcript(&id).unwrap());
    assert_eq!(
        saved.resume().config.get("model"),
        Some(&ConfigValue::Value("opus".into()))
    );

    // A new process starts with the agent's defaults again.
    f.commands.config.set_options(models());
    let requests = f.commands.config.requests().len();
    f.panel
        .update(cx, |panel, cx| panel.open_thread(first.entity_id(), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(first.read(cx).session().is_some()));
    assert_eq!(current_model(&first, cx), "Opus");
    let asked = f.commands.config.requests()[requests..].to_vec();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        Some(asked[0].value.clone()),
        Some(ConfigValue::Value("opus".into()))
    );
    exchange(&f, "and now?", "Still Opus.", cx);
    cx.update(|cx| assert_eq!(first.read(cx).model().as_deref(), Some("Opus")));
}

#[gpui_kit::test]
fn a_saved_chat_shows_its_own_model_before_it_connects(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    lazy_start(&f, cx);
    cx.update(|cx| {
        let runtime = f.installation.installed.runtime();
        runtime.remember_options("codex", &models(), cx)
    });
    let saved = nocterm_ai::history::SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-00000000006d".into(),
        1,
    );
    let (id, metadata, mut resume, mut transcript, composer, _) = saved.into_parts();
    transcript
        .entries
        .push(nocterm_ai::thread::Entry::Agent("earlier answer".into()));
    resume
        .config
        .insert("model".into(), ConfigValue::Value("opus".into()));
    let saved = nocterm_ai::history::SavedChat::from_parts(
        &f.installation.retention,
        id,
        metadata,
        resume,
        transcript,
        composer,
    )
    .unwrap();
    reinstall_history(&mut f, vec![saved], cx);
    cx.update_window(f.handle, |_, window, cx| {
        f.panel
            .update(cx, |panel, cx| panel.adopt_saved_chats(window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let thread = cx.update(|cx| f.panel.read(cx).chat_documents().next().unwrap().clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 0);
    assert_eq!(current_model(&thread, cx), "Opus");
}
