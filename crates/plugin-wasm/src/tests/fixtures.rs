//! Guests written as WAT text and the packages that carry them.

use std::sync::{Arc, Mutex};

use nocterm_plugin_abi::{Answer, Reply, encode_reply, pack};
use nocterm_plugin_api::{
    ContentHash, ContributionContext, Effects, HostContext, Kind, LocalId, Manifest, ManifestDto,
    Node, NodeKind, Package, Snapshot, Tone, View, Zone,
};
use nocterm_plugin_model::PluginId;

use crate::GuestLog;

/// Where the scripted reply sits in the guest's memory.
const REPLY_AT: u32 = 1024;
/// The buffer `nocterm_alloc` hands out; 16 pages leave room for the largest call.
const INPUT_AT: u32 = 65_536;

/// The parts of a test guest. Anything not set is a well-behaved guest that replies `reply`.
pub(super) struct Spec {
    pub pages: u32,
    pub version: i32,
    pub reply: Vec<u8>,
    pub imports: String,
    pub globals: String,
    pub alloc: String,
    pub body: String,
}

impl Spec {
    /// A guest that answers every call with the encoded `reply`.
    pub(super) fn replying(reply: &Reply) -> Self {
        Self::bytes(encode_reply(reply).expect("a test reply fits the ABI"))
    }

    /// A guest that answers every call with `reply`, whatever the bytes are.
    pub(super) fn bytes(reply: Vec<u8>) -> Self {
        let packed = pack(REPLY_AT, reply.len() as u32) as i64;
        Self {
            pages: 16,
            version: 1,
            reply,
            imports: String::new(),
            globals: String::new(),
            alloc: format!(
                r#"(func (export "nocterm_alloc") (param i32) (result i32) (i32.const {INPUT_AT}))"#
            ),
            body: format!("(i64.const {packed})"),
        }
    }

    /// A guest that returns the pair `(ptr, len)` without writing anything there.
    pub(super) fn pointing(ptr: u32, len: u32) -> Self {
        let mut spec = Self::bytes(Vec::new());
        spec.body = format!("(i64.const {})", pack(ptr, len) as i64);
        spec
    }

    pub(super) fn wasm(&self) -> Vec<u8> {
        let data: String = self
            .reply
            .iter()
            .map(|byte| format!("\\{byte:02x}"))
            .collect();
        let text = format!(
            r#"(module
                {imports}
                {globals}
                (memory (export "memory") {pages})
                (data (i32.const {REPLY_AT}) "{data}")
                (func (export "nocterm_abi_version") (result i32) (i32.const {version}))
                {alloc}
                (func (export "nocterm_call") (param i32 i32) (result i64) {body}))"#,
            imports = self.imports,
            globals = self.globals,
            pages = self.pages,
            version = self.version,
            alloc = self.alloc,
            body = self.body,
        );
        wat::parse_str(text).expect("a test guest is valid WAT")
    }

    pub(super) fn package(&self, hash: u8) -> Package {
        package(self.wasm(), hash)
    }
}

/// A guest that never returns from `nocterm_call`.
pub(super) fn spinning() -> Spec {
    let mut spec = Spec::bytes(Vec::new());
    spec.body = "(loop $spin (br $spin)) (i64.const 0)".into();
    spec
}

/// A guest that grows its memory page by page until the host refuses.
pub(super) fn growing() -> Spec {
    let mut spec = Spec::bytes(Vec::new());
    spec.body = "(loop $grow (drop (memory.grow (i32.const 1))) (br $grow)) (i64.const 0)".into();
    spec
}

/// A guest that answers `reply` on its first call and traps on every later call of the same
/// instance, so a restart is visible as a successful call after a trap.
pub(super) fn trapping_after_first(reply: &Reply) -> Spec {
    let mut spec = Spec::replying(reply);
    spec.globals = "(global $calls (mut i32) (i32.const 0))".into();
    let packed = pack(REPLY_AT, spec.reply.len() as u32) as i64;
    spec.body = format!(
        "(global.set $calls (i32.add (global.get $calls) (i32.const 1)))
         (if (i32.ge_u (global.get $calls) (i32.const 2)) (then unreachable))
         (i64.const {packed})"
    );
    spec
}

/// A package with the module bytes, the locals `face` (widget) and `board` (panel), and a hash
/// made of `hash` repeated.
pub(super) fn package(bytes: Vec<u8>, hash: u8) -> Package {
    let dto: ManifestDto = serde_json::from_value(serde_json::json!({
        "api": "0.1",
        "runtime": "wasm",
        "publisher": "acme",
        "id": "clock",
        "name": "Acme clock",
        "description": "A test plugin",
        "capabilities": ["clock.now"],
        "contribution": [
            { "local": "face", "kind": "widget", "title": "Face" },
            { "local": "board", "kind": "panel", "title": "Board" }
        ]
    }))
    .expect("the fixture is a manifest");
    let manifest =
        Manifest::validate(&dto, &HostContext::default()).expect("the fixture is a valid manifest");
    Package::new(ContentHash::new([hash; 32]), manifest, bytes)
}

pub(super) fn plugin_id() -> PluginId {
    PluginId::owned("user-acme-clock")
}

pub(super) fn local(name: &str) -> LocalId {
    LocalId::parse(name).expect("a test local name is valid")
}

pub(super) fn snapshot() -> Snapshot {
    Snapshot::new(ContributionContext {
        kind: Kind::Widget,
        zone: Zone::Right,
    })
}

/// A view of one text node.
pub(super) fn view(text: &str) -> View {
    View {
        nodes: vec![Node {
            parent: None,
            key: None,
            kind: NodeKind::Text(text.into()),
            tone: Tone::Default,
        }],
        refresh: None,
    }
}

pub(super) fn reply(answer: Answer, log: &[&str]) -> Reply {
    Reply {
        answer,
        log: log.iter().map(|line| (*line).to_owned()).collect(),
    }
}

pub(super) fn view_reply(text: &str) -> Reply {
    reply(Answer::View(view(text)), &[])
}

pub(super) fn effects_reply(effects: Effects) -> Reply {
    reply(Answer::Effects(effects), &[])
}

/// A log sink that keeps what it is given.
#[derive(Default)]
pub(super) struct RecordingLog(Mutex<Vec<(PluginId, String)>>);

impl RecordingLog {
    pub(super) fn lines(&self) -> Vec<String> {
        let lines = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lines.iter().map(|(_, text)| text.clone()).collect()
    }

    pub(super) fn plugins(&self) -> Vec<PluginId> {
        let lines = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lines.iter().map(|(plugin, _)| plugin.clone()).collect()
    }
}

impl GuestLog for RecordingLog {
    fn line(&self, plugin: &PluginId, text: &str) {
        let mut lines = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        lines.push((plugin.clone(), text.to_owned()));
    }
}

pub(super) fn recording_runtime() -> (crate::WasmRuntime, Arc<RecordingLog>) {
    let log = Arc::new(RecordingLog::default());
    (crate::WasmRuntime::new(log.clone()), log)
}
