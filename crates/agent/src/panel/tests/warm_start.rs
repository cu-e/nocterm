//! A shown chat connects before its first message, so its model can be
//! chosen first; choices made earlier still reach the session first.
use super::*;
use crate::thread::AgentThread;
use nocterm_ai::session_config::ConfigValue;

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
fn choose_opus(thread: &Entity<AgentThread>, cx: &mut TestAppContext) {
    thread.update(cx, |thread, cx| {
        thread.set_config(
            acp::SessionConfigId::new("model"),
            acp::SessionConfigOptionValue::value_id("opus"),
            cx,
        )
    });
    cx.run_until_parked();
}
fn current_model(thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> String {
    cx.update(|cx| crate::thread::config_label(&thread.read(cx).state.config_options[0]))
}

#[gpui_kit::test]
fn a_new_chat_connects_at_once_so_its_model_is_chosen_before_the_first_message(
    cx: &mut TestAppContext,
) {
    let f = fixture(cx);
    *f.commands.config.lock().unwrap() = models();
    let thread = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 1);
    cx.update(|cx| assert!(thread.read(cx).session().is_some()));
    assert_eq!(current_model(&thread, cx), "Sonnet");

    choose_opus(&thread, cx);
    assert_eq!(f.commands.config_requests.lock().unwrap().len(), 1);
    assert_eq!(current_model(&thread, cx), "Opus");
    exchange(&f, "hello", "Hi.", cx);
    cx.update(|cx| {
        assert_eq!(thread.read(cx).model().as_deref(), Some("Opus"));
        let state = &Runtime::global(cx).read(cx).favorites;
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
    lazy_start(cx);
    *f.commands.config.lock().unwrap() = models();
    cx.update(|cx| {
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.remember_options("codex", &models(), cx)
        })
    });
    let thread = chat(&f, cx);
    cx.run_until_parked();
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 0);
    assert_eq!(
        current_model(&thread, cx),
        "Sonnet",
        "the agent's last options show at once"
    );

    choose_opus(&thread, cx);
    assert_eq!(current_model(&thread, cx), "Opus");
    assert!(f.commands.config_requests.lock().unwrap().is_empty());
    thread.update(cx, |thread, cx| thread.send("hello".into(), cx));
    cx.run_until_parked();
    assert_eq!(f.commands.prompts.lock().unwrap().len(), 1);
    complete_active(&f, cx);
    let requests = f.commands.config_requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    assert_eq!(&*requests[0].config_id.0, "model");
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
            next.read(cx).config_choices.get("model"),
            Some(&ConfigValue::Value("opus".into()))
        )
    });
}

#[gpui_kit::test]
async fn a_chat_hidden_before_it_was_admitted_gives_up_its_place(cx: &mut TestAppContext) {
    let f = fixture(cx);
    cx.update(|cx| {
        cx.update_setting::<nocterm_ai::AiSettings>(|settings| settings.sessions.max_live = 1)
    })
    .await
    .unwrap();
    let busy = chat(&f, cx);
    cx.run_until_parked();
    busy.update(cx, |thread, cx| thread.send("long task".into(), cx));
    cx.run_until_parked();

    let skipped = chat(&f, cx);
    skipped.update(cx, |thread, _| thread.name = Some("kept".into()));
    cx.run_until_parked();
    cx.update(|cx| assert!(skipped.read(cx).lifecycle.queued()));
    let wanted = chat(&f, cx);
    cx.run_until_parked();
    cx.update(|cx| assert!(!skipped.read(cx).lifecycle.queued()));

    complete_active(&f, cx);
    cx.background_executor
        .advance_clock(std::time::Duration::from_secs(1));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(busy.read(cx).lease.is_none(), "the hidden idle chat yields");
        assert!(skipped.read(cx).lease.is_none());
        assert!(wanted.read(cx).session().is_some());
    });
    assert_eq!(f.connector.connects.load(Ordering::SeqCst), 2);
}
