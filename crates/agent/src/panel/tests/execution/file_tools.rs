//! File tools: offered while an attached terminal serves files but runs no programs, reading
//! freely and changing files only with approval.
use super::*;
use nocterm_ai::{EditFile, ReadFile, ToolOffer};
use nocterm_session::{HostAddress, Target, testing::InMemoryFs};

fn panel_host() -> HostAddress {
    HostAddress::direct(Target {
        host: "Game panel".into(),
        port: 22,
        user: "lobby".into(),
    })
}

/// A chat whose attached terminal serves `fs` and runs no programs, as a plugin's console tab.
fn console(cx: &mut TestAppContext) -> (Fixture, Entity<AgentThread>, Arc<InMemoryFs>, String) {
    let f = fixture(cx);
    let fs = f.access.serve_remote_files(panel_host());
    fs.seed_file("/server.properties", "motd=Hi\nmax-players=20\n")
        .unwrap();
    new_chat(&f, cx);
    let thread = current(&f, cx);
    let id = terminal_id(&thread, cx);
    (f, thread, fs, id)
}

fn last_offer(f: &Fixture, thread: &Entity<AgentThread>, cx: &mut TestAppContext) -> ToolOffer {
    let id = cx.update(|cx| thread.read(cx).registration().as_ref().unwrap().id);
    f.bridge
        .offers
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|(registration, _)| *registration == id)
        .map(|(_, offer)| *offer)
        .expect("the chat offered its tools")
}

fn read(id: &str) -> TerminalCall {
    TerminalCall::ReadFile(ReadFile {
        terminal_id: id.into(),
        path: "/server.properties".into(),
        start_line: None,
        max_lines: None,
    })
}

fn edit(id: &str, old: &str, new: &str) -> TerminalCall {
    TerminalCall::EditFile(EditFile {
        terminal_id: id.into(),
        path: "/server.properties".into(),
        old_text: old.into(),
        new_text: new.into(),
        replace_all: false,
    })
}

fn text(fs: &InMemoryFs) -> String {
    String::from_utf8(fs.bytes("/server.properties").unwrap()).unwrap()
}

#[gpui_kit::test]
fn file_tools_are_offered_only_while_such_a_terminal_is_attached(cx: &mut TestAppContext) {
    let (f, thread, _, _) = console(cx);
    assert_eq!(last_offer(&f, &thread, cx), ToolOffer::new(true));
    let context = cx.update(|cx| thread.update(cx, |thread, cx| thread.context(cx)));
    assert!(context.contains(r#""file_tools":true"#), "{context}");
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            thread.attach(Attachment::Terminal(f.terminal), cx);
        })
    });
    assert_eq!(
        last_offer(&f, &thread, cx),
        ToolOffer::new(false),
        "detaching the terminal withdraws the file tools"
    );
}

#[gpui_kit::test]
fn a_terminal_that_runs_programs_gets_no_file_tools(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let fs = f.access.serve_remote_files(panel_host());
    fs.seed_file("/server.properties", "motd=Hi\n").unwrap();
    *f.access.executor().borrow_mut() = Some(Arc::new(Programs::new(f._directory.jobs.clone())));
    new_chat(&f, cx);
    let thread = current(&f, cx);
    assert_eq!(last_offer(&f, &thread, cx), ToolOffer::new(false));
    let id = terminal_id(&thread, cx);
    let refused = result(submit(&f, &thread, read(&id), true, cx)).unwrap_err();
    assert!(refused.message().contains("file_tools: true"), "{refused}");
}

#[gpui_kit::test]
fn an_agent_reads_freely_and_changes_a_file_after_approval(cx: &mut TestAppContext) {
    let (f, thread, fs, id) = console(cx);
    let content = result(submit(&f, &thread, read(&id), false, cx)).unwrap();
    assert_eq!(content["content"], "motd=Hi\nmax-players=20\n");

    let pending = submit(
        &f,
        &thread,
        edit(&id, "max-players=20", "max-players=40"),
        false,
        cx,
    );
    assert_eq!(
        text(&fs),
        "motd=Hi\nmax-players=20\n",
        "nothing changes before the card is answered"
    );
    cx.update(|cx| {
        thread.update(cx, |thread, cx| {
            let token = thread.tool_cards()[0].token();
            thread.approve_tool(token, true, true, cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(result(pending).unwrap()["replacements"], 1);
    assert_eq!(text(&fs), "motd=Hi\nmax-players=40\n");

    let granted = submit(&f, &thread, edit(&id, "motd=Hi", "motd=Welcome"), false, cx);
    assert!(
        cx.update(|cx| thread.read(cx).tool_cards().is_empty()),
        "the grant covers later changes on this terminal"
    );
    result(granted).unwrap();
    assert_eq!(text(&fs), "motd=Welcome\nmax-players=40\n");
}
