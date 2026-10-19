//! A shown chat connects before its first message, so its model can be
//! chosen first; choices made earlier still reach the session first.
use super::*;
use crate::thread::AgentThread;
use nocterm_ai::session_config::ConfigValue;

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
fn choose_opus(thread: &Entity<AgentThread>, cx: &mut TestAppContext) {
    thread.update(cx, |thread, cx| {
        thread.set_config(
            nocterm_ai::model::ids::ConfigId::new("model"),
            ConfigValue::Value("opus".into()),
            cx,
        )
    });
    cx.run_until_parked();
}
fn current_model(thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| crate::thread::config_label(&thread.read(cx).config_options()[0]))
}

#[gpui_kit::test]
fn a_new_chat_connects_at_once_so_its_model_is_chosen_before_the_first_message(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    f.commands.config.set_options(models());
    let thread = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| assert!(thread.read(cx).session().is_some()));
    assert_eq!(current_model(&thread, cx), "Sonnet");

    choose_opus(&thread, cx);
    assert_eq!(f.commands.config.requests().len(), 1);
    assert_eq!(current_model(&thread, cx), "Opus");
    exchange(&f, "hello", "Hi.", cx);
    cx.update(|cx| {
        assert_eq!(thread.read(cx).model().as_deref(), Some("Opus"));
        let state = &f.installation.installed.runtime().favorites(cx);
        assert_eq!(
            state.choices("codex").get("model"),
            Some(&ConfigValue::Value("opus".into()))
        );
        assert_eq!(state.options["codex"].len(), 1);
    });
}

#[gpui_kit::test]
fn a_model_chosen_before_the_session_opens_is_set_before_the_first_prompt(cx: &mut TestAppContext) {
    let f = fixture(cx);
    lazy_start(&f, cx);
    f.commands.config.set_options(models());
    cx.update(|cx| {
        let runtime = f.installation.installed.runtime();
        runtime.remember_options("codex", &models(), cx)
    });
    let thread = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 0);
    assert_eq!(
        current_model(&thread, cx),
        "Sonnet",
        "the agent's last options show at once"
    );

    choose_opus(&thread, cx);
    assert_eq!(current_model(&thread, cx), "Opus");
    assert!(f.commands.config.requests().is_empty());
    thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    complete_active(&f, cx);
    let requests = f.commands.config.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].option.as_str(), "model");
    cx.update(|cx| {
        assert_eq!(
            thread.read(cx).model().as_deref(),
            Some("Opus"),
            "the prompt went out with the chosen model"
        )
    });

    // The agent's next chat starts with the same model.
    let next = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(current_model(&next, cx), "Opus");
    cx.update(|cx| {
        assert_eq!(
            nocterm_ai::session_config::current(&next.read(cx).config_options()).get("model"),
            Some(&ConfigValue::Value("opus".into()))
        )
    });
}

#[gpui_kit::test]
async fn a_chat_hidden_before_it_was_admitted_gives_up_its_place(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        f.installation
            .ready
            .settings()
            .update::<nocterm_ai::AiSettings>(cx, |settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let busy = chat(&f, cx);
    cx.run_until_parked();
    busy.update(cx, |thread, cx| thread.send("long task".into(), cx));
    cx.run_until_parked();

    let skipped = chat(&f, cx);
    skipped.update(cx, |thread, cx| thread.rename(Some("kept".into()), cx));
    cx.run_until_parked();
    cx.update(|cx| assert!(skipped.read(cx).session_queued()));
    let wanted = chat(&f, cx);
    cx.run_until_parked();
    cx.update(|cx| assert!(!skipped.read(cx).session_queued()));

    complete_active(&f, cx);
    cx.background_executor
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!busy.read(cx).is_leased(), "the hidden idle chat yields");
        assert!(!skipped.read(cx).is_leased());
        assert!(wanted.read(cx).session().is_some());
    });
    assert_eq!(f.connector.connect_count(), 2);
}

#[gpui_kit::test]
fn a_saved_chat_connects_when_opened_and_its_queue_still_waits(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.commands.config.set_options(models());
    let saved = nocterm_ai::history::SavedChat::for_identity(
        &f.installation.retention,
        "codex".into(),
        "00000000-0000-4000-8000-000000000071".into(),
        1,
    );
    let (id, metadata, resume, mut transcript, mut composer, _) = saved.into_parts();
    transcript
        .entries
        .push(nocterm_ai::thread::Entry::Agent("earlier answer".into()));
    composer.queue.push(nocterm_ai::history::SavedPrompt {
        id: 1,
        text: "left over".into(),
        images: Vec::new(),
        attachments: Vec::new(),
    });
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
    assert_eq!(
        f.connector.connect_count(),
        0,
        "listed history stays asleep"
    );
    let thread = cx.update(|cx| f.panel.read(cx).chat_documents().next().unwrap().clone());
    f.panel
        .update(cx, |panel, cx| panel.open_thread(thread.entity_id(), cx));
    cx.run_until_parked();
    assert_eq!(f.connector.connect_count(), 1);
    cx.update(|cx| {
        let thread = thread.read(cx);
        assert!(thread.session().is_some());
        assert!(thread.composer().paused());
        assert_eq!(thread.composer().queue().len(), 1);
    });
    assert_eq!(current_model(&thread, cx), "Sonnet", "its pickers work");
    assert!(f.commands.prompts.lock().unwrap().is_empty());
}
