use super::*;
use futures::executor::block_on;
use nocterm_session::testing::{ExecResponse, FakeExec, Gate};

fn run(exec: FakeExec, engine: Engine) -> Result<Vec<RegistryImage>, ImageSearchError> {
    block_on(NativeImageSearch.search(
        Arc::new(exec),
        engine,
        ImageSearchQuery::parse("debian").unwrap(),
        CancelFlag::default(),
    ))
}
const DOCKER: &str = r#"{"Name":"debian","Description":"Debian Linux","IsOfficial":"true","StarCount":"5832"}
{"Name":"someone/debian","Description":"another build","IsOfficial":"false","StarCount":"1"}"#;
const PODMAN: &str = r#"[{"Index":"docker.io","Name":"docker.io/library/debian","Description":"Debian Linux","Official":"[OK]","Stars":5832,"Automated":"","Tag":""}]"#;

#[test]
fn both_engines_decode_their_real_wire_formats_and_keep_original_executor() {
    for (engine, fixture) in [(Engine::Docker, DOCKER), (Engine::Podman, PODMAN)] {
        let exec = FakeExec::default();
        exec.respond(
            engine.name().to_lowercase(),
            Ok(ExecResponse::text(fixture, 0, "")),
        );
        let images = run(exec.clone(), engine).unwrap();
        assert_eq!(images[0].reference, "docker.io/library/debian");
        assert!(images[0].official);
        assert_eq!(images[0].stars, Some(5832));
        let request = exec.requests().pop().unwrap();
        assert_eq!(request.program, engine.name().to_lowercase());
        assert_eq!(request.args.last().unwrap(), "debian");
        assert!(
            request
                .args
                .windows(2)
                .any(|args| args == ["--limit", "25"])
        );
        assert!(!request.args.iter().any(|arg| arg == "pull"));
    }
}
#[test]
fn literal_query_is_an_argument_and_never_shell_source() {
    let exec = FakeExec::default();
    exec.respond("docker", Ok(ExecResponse::text("", 0, "")));
    let query = "debian;$(touch nope)";
    block_on(NativeImageSearch.search(
        Arc::new(exec.clone()),
        Engine::Docker,
        ImageSearchQuery::parse(query).unwrap(),
        CancelFlag::default(),
    ))
    .unwrap();
    assert_eq!(exec.requests()[0].args.last().unwrap(), query);
    assert_eq!(exec.requests()[0].program, "docker");
}
#[test]
fn both_engine_empty_results_are_successful() {
    for engine in Engine::ALL {
        let exec = FakeExec::default();
        exec.respond(
            engine.name().to_lowercase(),
            Ok(ExecResponse::text("", 0, "")),
        );
        assert!(run(exec, engine).unwrap().is_empty());
    }
}
#[test]
fn typed_failures_distinguish_quota_engine_and_malformed_responses() {
    for (status, stderr) in [
        (429, "too many requests"),
        (1, "TOOMANYREQUESTS"),
        (1, "HTTP 429"),
    ] {
        let exec = FakeExec::default();
        exec.respond("docker", Ok(ExecResponse::text("", status, stderr)));
        assert!(
            matches!(run(exec, Engine::Docker), Err(ImageSearchError::RateLimited { cooldown }) if cooldown == Duration::from_secs(60))
        );
    }
    let exec = FakeExec::default();
    exec.respond("docker", Err(ExecError::NotFound("docker".into())));
    assert!(matches!(
        run(exec, Engine::Docker),
        Err(ImageSearchError::Unavailable)
    ));
    let exec = FakeExec::default();
    exec.respond("docker", Ok(ExecResponse::text("not-json", 0, "")));
    let error = run(exec, Engine::Docker).unwrap_err();
    assert!(std::error::Error::source(&error).is_some());
    assert!(matches!(error, ImageSearchError::Codec(_)));
    let exec = FakeExec::default();
    exec.respond(
        "docker",
        Ok(ExecResponse::text("", 5, "unavailable registry")),
    );
    assert!(matches!(
        run(exec, Engine::Docker),
        Err(ImageSearchError::CommandFailed { status: Some(5) })
    ));
}
#[test]
fn large_and_invalid_results_cannot_escape_the_output_bound() {
    let exec = FakeExec::default();
    exec.respond(
        "docker",
        Ok(ExecResponse::text("x".repeat(BYTE_LIMIT + 1), 0, "")),
    );
    assert!(matches!(
        run(exec, Engine::Docker),
        Err(ImageSearchError::OutputLimit)
    ));
    for name in [
        "invalid reference",
        "",
        "docker.io/library/",
        "docker.io//debian",
        "docker.io/x;echo",
    ] {
        let exec = FakeExec::default();
        exec.respond(
            "podman",
            Ok(ExecResponse::text(
                PODMAN.replace("docker.io/library/debian", name),
                0,
                "",
            )),
        );
        assert!(matches!(
            run(exec, Engine::Podman),
            Err(ImageSearchError::InvalidResponse)
        ));
    }
    let rows = vec![
        serde_json::json!({"Name":"docker.io/library/debian","Description":"", "Stars":0});
        30
    ];
    let exec = FakeExec::default();
    exec.respond(
        "podman",
        Ok(ExecResponse::text(
            serde_json::to_string(&rows).unwrap(),
            0,
            "",
        )),
    );
    assert_eq!(run(exec, Engine::Podman).unwrap().len(), 25);
}
#[test]
fn canceled_and_timed_out_requests_drop_the_running_program() {
    let exec = FakeExec::default();
    let sink = exec.queue_stream("docker");
    let cancel = CancelFlag::default();
    let mut pending = NativeImageSearch.search(
        Arc::new(exec),
        Engine::Docker,
        ImageSearchQuery::parse("debian").unwrap(),
        cancel.clone(),
    );
    assert!(pending.as_mut().now_or_never().is_none());
    cancel.cancel();
    assert!(matches!(block_on(pending), Err(ImageSearchError::Canceled)));
    assert!(sink.is_closed());
    let exec = FakeExec::default();
    let sink = exec.queue_stream("podman");
    assert!(matches!(
        block_on(search(
            Arc::new(exec),
            Engine::Podman,
            ImageSearchQuery::parse("debian").unwrap(),
            CancelFlag::default(),
            Duration::from_millis(1)
        )),
        Err(ImageSearchError::TimedOut)
    ));
    assert!(sink.is_closed());
}
#[test]
fn canceled_search_never_starts_or_publishes_a_result() {
    let exec = FakeExec::default();
    let cancel = CancelFlag::default();
    cancel.cancel();
    assert!(matches!(
        block_on(NativeImageSearch.search(
            Arc::new(exec.clone()),
            Engine::Docker,
            ImageSearchQuery::parse("debian").unwrap(),
            cancel
        )),
        Err(ImageSearchError::Canceled)
    ));
    assert!(exec.requests().is_empty());
    let exec = FakeExec::default();
    exec.suspend(Gate::pending());
    let cancel = CancelFlag::default();
    let mut pending = NativeImageSearch.search(
        Arc::new(exec),
        Engine::Docker,
        ImageSearchQuery::parse("debian").unwrap(),
        cancel.clone(),
    );
    assert!(pending.as_mut().now_or_never().is_none());
    cancel.cancel();
    assert!(matches!(block_on(pending), Err(ImageSearchError::Canceled)));
}
