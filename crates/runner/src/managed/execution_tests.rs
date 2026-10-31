//! A managed agent runs a tool: the model call goes to a local stub broker with
//! the chat's token, the tool runs in real `ContainedLocal` containment under
//! the instance workspace, and its output reaches the second model request. No
//! provider key is involved.
use super::*;
use crate::managed::testing::{exec_reply, text_reply};
use nocterm_cloud_model::{
    engine::{EngineRequest, EngineToolResult, EngineToolResults},
    payload::BoundedJson,
};
use nocterm_local::{ContainedLocal, ContainmentLimits};
use nocterm_session::{ExecRequest, HostExec as _};
use std::os::unix::fs::PermissionsExt as _;

fn private(path: &std::path::Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
#[ignore = "requires bubblewrap, prlimit and enabled Linux user namespaces"]
async fn a_managed_tool_runs_contained_and_its_output_reaches_the_next_model_request() {
    let root = tempfile::tempdir().unwrap();
    let (workspace, state) = (root.path().join("workspace"), root.path().join("state"));
    private(&workspace);
    private(&state);
    let limits = ContainmentLimits::new(256 << 20, 64, 64, 10, 8 << 20).unwrap();
    let contained = ContainedLocal::prepare(workspace.clone(), state, limits)
        .await
        .expect("nested containment is required for this test");

    let arguments = serde_json::json!({
        "terminal_id": "t", "program": "/bin/sh",
        "args": ["-c", "printf from-the-sandbox > /workspace/marker && cat /workspace/marker"],
    })
    .to_string();
    let broker = Broker::start(vec![exec_reply(&arguments), text_reply("all done")]).await;
    let providers = providers(broker.root());
    providers.tokens().issue(chat(1), &token("one")).unwrap();
    let mut engine = create(&providers, 1).unwrap();

    let EngineAction::Tools(batch) = advance(engine.as_mut(), 1).await.unwrap() else {
        panic!("the first broker answer is a tool call");
    };
    let call = &batch.calls()[0];
    let requested = call.call().arguments().unwrap();
    let args: Vec<String> = serde_json::from_value(requested["args"].clone()).unwrap();
    let request = ExecRequest::new(requested["program"].as_str().unwrap()).args(args);
    let output = contained.exec(request).await.unwrap().collect(4096).await;
    assert!(output.exit.success(), "{:?}", output.exit);
    let result = EngineToolResult::new(
        call.slot(),
        BoundedJson::new(serde_json::json!({ "output": output.text() })).unwrap(),
        false,
    );
    let results = EngineToolResults::new(batch.step(), vec![result]).unwrap();
    let next = advance_with(
        engine.as_mut(),
        1,
        StepId::from_bytes([2; 16]),
        EngineRequest::ToolResults(results),
    )
    .await
    .unwrap();

    assert!(matches!(next, EngineAction::Complete(_)));
    assert_eq!(
        std::fs::read(workspace.join("marker")).unwrap(),
        b"from-the-sandbox"
    );
    let bodies = broker.bodies();
    assert_eq!(bodies.len(), 2);
    assert!(bodies[1].contains("from-the-sandbox"), "{}", bodies[1]);
    assert!(broker.heads().iter().all(|h| h.contains("bearer nvt1_one")));
}
