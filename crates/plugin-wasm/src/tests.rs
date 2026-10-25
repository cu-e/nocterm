//! Behaviour of the code runtime, through `PluginRuntime` and `PluginInstance`, on guests
//! written as WAT.

mod fixtures;

use fixtures::{
    Spec, effects_reply, growing, local, plugin_id, recording_runtime, reply, snapshot, spinning,
    trapping_after_first, view, view_reply,
};
use nocterm_plugin_abi::{
    Answer, MAX_INPUT_BYTES, MAX_LIVE_INSTANCES, MAX_LOG_LINES, MAX_REPLY_BYTES,
    names::EXPORT_ALLOC,
};
use nocterm_plugin_api::{
    Effect, EffectRequest, Effects, Event, ModuleError, Package, PluginInstance, PluginRuntime,
    ProtocolError, RuntimeError,
    capability::notifications::{NoticeTone, Post},
};

use crate::{WasmRuntime, cache::ModuleCache, engine::engine};

fn instance(spec: &Spec) -> (Box<dyn PluginInstance>, WasmRuntime) {
    let (runtime, _) = recording_runtime();
    let instance = runtime
        .instantiate(&spec.package(1))
        .unwrap_or_else(|error| panic!("the guest starts: {error}"));
    (instance, runtime)
}

fn refusal(spec: &Spec) -> RuntimeError {
    let (runtime, _) = recording_runtime();
    runtime
        .instantiate(&spec.package(1))
        .err()
        .expect("the guest is refused")
}

fn render(instance: &dyn PluginInstance) -> Result<nocterm_plugin_api::View, RuntimeError> {
    instance.render(&local("face"), &snapshot())
}

fn activate(instance: &dyn PluginInstance) -> Result<Effects, RuntimeError> {
    instance.on_event(
        &local("board"),
        Event::Activated { key: "go".into() },
        &snapshot(),
    )
}

fn note() -> Effects {
    Effects(vec![EffectRequest {
        key: Some("note".into()),
        effect: Effect::NotificationsPost(Post {
            tone: NoticeTone::Info,
            text: "Hello".into(),
            notice_key: None,
        }),
    }])
}

#[test]
fn render_returns_the_view_the_guest_replied() {
    let (instance, _runtime) = instance(&Spec::replying(&view_reply("12:30")));
    assert_eq!(render(instance.as_ref()), Ok(view("12:30")));
}

#[test]
fn an_event_returns_the_effects_the_guest_replied() {
    let (instance, _runtime) = instance(&Spec::replying(&effects_reply(note())));
    assert_eq!(activate(instance.as_ref()), Ok(note()));
}

#[test]
fn a_context_the_package_does_not_declare_is_unknown() {
    let (instance, _runtime) = instance(&Spec::replying(&view_reply("x")));
    assert_eq!(
        instance.render(&local("nope"), &snapshot()),
        Err(RuntimeError::UnknownContribution(local("nope")))
    );
}

#[test]
fn a_guest_that_never_returns_misses_its_deadline() {
    let (instance, _runtime) = instance(&spinning());
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Deadline));
}

#[test]
fn a_guest_that_grows_without_end_hits_the_memory_limit() {
    let (instance, _runtime) = instance(&growing());
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Memory));
}

#[test]
fn a_module_that_starts_over_the_memory_limit_is_refused() {
    let mut spec = Spec::replying(&view_reply("x"));
    spec.pages = 300;
    assert_eq!(refusal(&spec), RuntimeError::Memory);
}

#[test]
fn a_trap_is_reported_and_the_next_call_starts_the_guest_fresh() {
    let (instance, _runtime) = instance(&trapping_after_first(&view_reply("ok")));
    assert_eq!(render(instance.as_ref()), Ok(view("ok")));
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Trap));
    // A guest that kept its state would trap again; a fresh one counts from zero.
    assert_eq!(render(instance.as_ref()), Ok(view("ok")));
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Trap));
}

#[test]
fn a_call_the_host_cannot_encode_is_refused_without_blaming_or_restarting_the_guest() {
    let (instance, _runtime) = instance(&trapping_after_first(&view_reply("ok")));
    assert_eq!(render(instance.as_ref()), Ok(view("ok")));

    let mut oversized = snapshot();
    oversized
        .storage
        .insert("k".into(), "v".repeat(MAX_INPUT_BYTES));
    assert_eq!(
        instance.render(&local("face"), &oversized),
        Err(RuntimeError::InputTooLarge)
    );
    // The guest kept its state: a restart would answer `ok` again instead of trapping.
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Trap));
}

#[test]
fn a_guest_that_recurses_without_end_traps_instead_of_overflowing_the_host_stack() {
    let mut spec = Spec::replying(&view_reply("x"));
    spec.globals = "(func $down (param i32) (result i32)
        (call $down (i32.add (local.get 0) (i32.const 1))))"
        .into();
    spec.body = "(drop (call $down (i32.const 0))) (i64.const 0)".into();
    let (instance, _runtime) = instance(&spec);
    assert_eq!(render(instance.as_ref()), Err(RuntimeError::Trap));
}

#[test]
fn a_module_of_another_abi_version_is_refused() {
    let mut spec = Spec::replying(&view_reply("x"));
    spec.version = 7;
    assert_eq!(
        refusal(&spec),
        RuntimeError::Module(ModuleError::AbiVersion { found: 7 })
    );
}

/// wasmi reports a table over the limit at instantiation as
/// `Instantiation(FailedToInstantiateTable)`, so this test pins that path. The `ErrorKind::Table`
/// arm of `runtime_error` is not reached by it: a `table.grow` over the limit traps with
/// `GrowthOperationLimited` instead.
#[test]
fn a_module_with_a_table_over_the_limit_is_refused_as_a_module() {
    let mut spec = Spec::replying(&view_reply("x"));
    spec.globals = "(table 20000 funcref)".into();
    assert!(matches!(
        refusal(&spec),
        RuntimeError::Module(ModuleError::Invalid(_))
    ));
}

#[test]
fn a_module_with_an_import_is_refused() {
    let mut spec = Spec::replying(&view_reply("x"));
    spec.imports = r#"(import "env" "now" (func (result i64)))"#.into();
    assert_eq!(refusal(&spec), RuntimeError::Module(ModuleError::Imports));
}

#[test]
fn a_missing_or_mistyped_export_is_refused() {
    let mut missing = Spec::replying(&view_reply("x"));
    missing.alloc = String::new();
    assert_eq!(
        refusal(&missing),
        RuntimeError::Module(ModuleError::Exports)
    );

    let mut mistyped = Spec::replying(&view_reply("x"));
    mistyped.alloc =
        format!(r#"(func (export "{EXPORT_ALLOC}") (param i64) (result i32) (i32.const 65536))"#);
    assert_eq!(
        refusal(&mistyped),
        RuntimeError::Module(ModuleError::Exports)
    );
}

#[test]
fn bytes_that_are_not_a_module_are_refused() {
    let (runtime, _) = recording_runtime();
    let package = fixtures::package(b"\0asm\x01\0\0\0garbage".to_vec(), 1);
    assert!(matches!(
        runtime.instantiate(&package).err(),
        Some(RuntimeError::Module(ModuleError::Invalid(_)))
    ));
}

#[test]
fn a_reply_outside_the_guest_memory_is_malformed() {
    for spec in [
        Spec::pointing(1_048_000, 1000),
        Spec::pointing(0xFFFF_0000, 16),
    ] {
        let (instance, _runtime) = instance(&spec);
        assert_eq!(
            render(instance.as_ref()),
            Err(RuntimeError::Malformed(ProtocolError::OutsideMemory))
        );
    }
}

#[test]
fn a_reply_over_the_size_bound_is_malformed() {
    for len in [MAX_REPLY_BYTES as u32 + 1, u32::MAX] {
        let (instance, _runtime) = instance(&Spec::pointing(0, len));
        assert_eq!(
            render(instance.as_ref()),
            Err(RuntimeError::Malformed(ProtocolError::ReplyTooLarge {
                max: MAX_REPLY_BYTES
            }))
        );
    }
}

#[test]
fn a_reply_that_is_not_postcard_is_malformed_and_keeps_the_decoder_cause() {
    use std::error::Error as _;
    let (instance, _runtime) = instance(&Spec::bytes(vec![0xff; 40]));
    let error = render(instance.as_ref()).unwrap_err();
    let RuntimeError::Malformed(protocol) = &error else {
        panic!("expected a malformed reply, got {error:?}");
    };
    assert!(matches!(protocol, ProtocolError::Undecodable(_)));
    assert!(protocol.source().is_some(), "the cause is kept");
}

#[test]
fn more_log_lines_than_the_bound_make_the_reply_malformed() {
    let (runtime, log) = recording_runtime();
    let lines = ["line"; MAX_LOG_LINES + 1];
    let reply = reply(Answer::View(view("x")), &lines);
    let instance = runtime
        .instantiate(&Spec::replying(&reply).package(1))
        .unwrap();
    assert_eq!(
        render(instance.as_ref()),
        Err(RuntimeError::Malformed(ProtocolError::TooManyLogLines {
            max: MAX_LOG_LINES
        }))
    );
    assert!(
        log.lines().is_empty(),
        "nothing of a refused reply is logged"
    );
}

#[test]
fn the_wrong_kind_of_answer_is_malformed_and_restarts_the_guest() {
    let (instance, _runtime) = instance(&Spec::replying(&view_reply("x")));
    assert_eq!(
        activate(instance.as_ref()),
        Err(RuntimeError::Malformed(ProtocolError::WrongAnswer))
    );
    assert_eq!(render(instance.as_ref()), Ok(view("x")));

    let (instance, _runtime) = instance_of(&effects_reply(note()));
    assert_eq!(
        render(instance.as_ref()),
        Err(RuntimeError::Malformed(ProtocolError::WrongAnswer))
    );
}

fn instance_of(reply: &nocterm_plugin_abi::Reply) -> (Box<dyn PluginInstance>, WasmRuntime) {
    instance(&Spec::replying(reply))
}

#[test]
fn the_seventeenth_live_instance_is_refused_and_a_dropped_one_frees_its_place() {
    let (runtime, _) = recording_runtime();
    let package = Spec::replying(&view_reply("x")).package(1);
    let mut live: Vec<_> = (0..MAX_LIVE_INSTANCES)
        .map(|_| runtime.instantiate(&package).expect("within the budget"))
        .collect();
    assert_eq!(
        runtime.instantiate(&package).err(),
        Some(RuntimeError::Capacity)
    );
    live.pop();
    assert!(runtime.instantiate(&package).is_ok());
}

#[test]
fn a_refused_module_does_not_keep_its_place() {
    let (runtime, _) = recording_runtime();
    let mut spec = Spec::replying(&view_reply("x"));
    spec.version = 2;
    let package = spec.package(1);
    for _ in 0..=MAX_LIVE_INSTANCES {
        assert!(matches!(
            runtime.instantiate(&package).err(),
            Some(RuntimeError::Module(ModuleError::AbiVersion { .. }))
        ));
    }
}

#[test]
fn a_module_is_compiled_once_per_hash_and_the_cache_is_bounded() {
    let engine = engine();
    let cache = ModuleCache::default();
    let bytes = Spec::replying(&view_reply("x")).wasm();
    let package = |hash: u8| -> Package { fixtures::package(bytes.clone(), hash) };
    cache.get_or_compile(&engine, &package(1)).unwrap();
    cache.get_or_compile(&engine, &package(1)).unwrap();
    assert_eq!(cache.len(), 1);
    for hash in 2..40 {
        cache.get_or_compile(&engine, &package(hash)).unwrap();
    }
    assert_eq!(cache.len(), 16);
    // A refused module is not cached.
    let broken = fixtures::package(b"\0asm\x01\0\0\0x".to_vec(), 200);
    assert!(cache.get_or_compile(&engine, &broken).is_err());
    assert_eq!(cache.len(), 16);
}

#[test]
fn log_lines_are_cleaned_cut_and_attributed() {
    let long = "x".repeat(300);
    let lines: Vec<&str> = std::iter::once("a\u{200b}b\u{7}c")
        .chain(std::iter::once(long.as_str()))
        .chain(std::iter::repeat_n("more", MAX_LOG_LINES - 2))
        .collect();
    let (runtime, log) = recording_runtime();
    let reply = reply(Answer::View(view("x")), &lines);
    let instance = runtime
        .instantiate(&Spec::replying(&reply).package(1))
        .unwrap();
    render(instance.as_ref()).unwrap();
    let seen = log.lines();
    assert_eq!(seen.len(), MAX_LOG_LINES);
    assert_eq!(seen[0], "abc");
    assert_eq!(seen[1].chars().count(), 200);
    assert!(log.plugins().iter().all(|plugin| *plugin == plugin_id()));
}

#[test]
fn an_instance_logs_a_bounded_number_of_lines_in_its_lifetime() {
    let (runtime, log) = recording_runtime();
    let lines = ["line"; MAX_LOG_LINES];
    let reply = reply(Answer::View(view("x")), &lines);
    let instance = runtime
        .instantiate(&Spec::replying(&reply).package(1))
        .unwrap();
    for _ in 0..40 {
        render(instance.as_ref()).unwrap();
    }
    let seen = log.lines();
    assert_eq!(seen.len(), 257);
    assert_eq!(
        seen[..256].iter().filter(|text| *text == "line").count(),
        256
    );
    assert!(seen[256].contains("suppressed"));
}
