//! In-memory ports and a scripted runtime for tests of the host and of the application's
//! adapters.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex, MutexGuard},
};

use gpui::{App, Task};
use nocterm_plugin_api::{
    Cause, ContentHash, Effects, Event, HostContext, LocalId, Manifest, ManifestDto, Node,
    NodeKind, Package, PackageError, PluginInstance, PluginPackages, PluginRuntime, RuntimeError,
    Snapshot, View,
};
use nocterm_plugin_model::PluginId;

use crate::{
    PluginData, PluginDataChange, PluginDataDoc, PluginDataError, PluginRecords, RecordsDoc,
    RecordsError,
};

/// A records port that keeps the last document in memory.
#[derive(Default)]
pub struct MemoryRecords {
    doc: RefCell<RecordsDoc>,
    saves: RefCell<usize>,
}

impl MemoryRecords {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Starts from a document, as if it had been saved by an earlier run.
    pub fn seeded(doc: RecordsDoc) -> Rc<Self> {
        Rc::new(Self {
            doc: RefCell::new(doc),
            saves: RefCell::new(0),
        })
    }

    /// The document last saved.
    pub fn doc(&self) -> RecordsDoc {
        self.doc.borrow().clone()
    }

    /// How many times the host saved.
    pub fn saves(&self) -> usize {
        *self.saves.borrow()
    }
}

impl PluginRecords for MemoryRecords {
    fn load(&self, _: &App) -> RecordsDoc {
        self.doc.borrow().clone()
    }

    fn replace(&self, doc: RecordsDoc, _: &mut App) -> Task<Result<(), RecordsError>> {
        *self.doc.borrow_mut() = doc;
        *self.saves.borrow_mut() += 1;
        Task::ready(Ok(()))
    }
}

/// Why [`MemoryData::refuse_changes`] refuses.
#[derive(Debug, thiserror::Error)]
#[error("the stand-in refuses every change")]
struct Refused;

/// A data port that keeps documents in memory and can hold a broken file.
#[derive(Default)]
pub struct MemoryData {
    docs: RefCell<BTreeMap<PluginId, PluginDataDoc>>,
    broken: RefCell<BTreeSet<PluginId>>,
    changes: RefCell<Vec<(PluginId, PluginDataChange)>>,
    refusing: Cell<bool>,
}

impl MemoryData {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Makes the port refuse every change, as a writer that is closing does.
    pub fn refuse_changes(&self) {
        self.refusing.set(true);
    }

    /// Puts a document on "disk".
    pub fn put(&self, plugin: &PluginId, doc: PluginDataDoc) {
        self.docs.borrow_mut().insert(plugin.clone(), doc);
    }

    /// Makes the plugin's file unreadable until a change replaces it.
    pub fn break_file(&self, plugin: &PluginId) {
        self.broken.borrow_mut().insert(plugin.clone());
    }

    /// The document on "disk", if any.
    pub fn stored(&self, plugin: &PluginId) -> Option<PluginDataDoc> {
        self.docs.borrow().get(plugin).cloned()
    }

    /// Whether the plugin's file is still the unreadable one.
    pub fn is_broken(&self, plugin: &PluginId) -> bool {
        self.broken.borrow().contains(plugin)
    }

    /// Every change the host asked for, oldest first.
    pub fn changes(&self) -> Vec<(PluginId, PluginDataChange)> {
        self.changes.borrow().clone()
    }
}

impl PluginData for MemoryData {
    fn load(&self, plugin: &PluginId, _: &App) -> Result<PluginDataDoc, PluginDataError> {
        if self.is_broken(plugin) {
            let syntax = PluginDataDoc::decode(b"not a data document")
                .expect_err("the text is not a document");
            return Err(PluginDataError::Broken(Cause::new(syntax)));
        }
        Ok(self.stored(plugin).unwrap_or_default())
    }

    fn change(
        &self,
        plugin: PluginId,
        change: PluginDataChange,
        _: &mut App,
    ) -> Result<(), PluginDataError> {
        if self.refusing.get() {
            return Err(PluginDataError::NotSaved(Cause::new(Refused)));
        }
        self.broken.borrow_mut().remove(&plugin);
        match &change {
            PluginDataChange::Replace(doc) => {
                self.docs.borrow_mut().insert(plugin.clone(), doc.clone());
            }
            PluginDataChange::Remove => {
                self.docs.borrow_mut().remove(&plugin);
            }
        }
        self.changes.borrow_mut().push((plugin, change));
        Ok(())
    }
}

/// A package store in memory. A package loads only under the hash it was published with.
#[derive(Default)]
pub struct MemoryPackages {
    stored: Mutex<BTreeMap<PluginId, Package>>,
}

impl MemoryPackages {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn stored(&self) -> MutexGuard<'_, BTreeMap<PluginId, Package>> {
        self.stored
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Puts a package in the store, as if an earlier run had published it.
    pub fn put(&self, package: &Package) {
        let id = package.manifest().plugin().id().clone();
        self.stored().insert(id, package.clone());
    }

    /// Whether a package of the plugin is stored.
    pub fn holds(&self, plugin: &PluginId) -> bool {
        self.stored().contains_key(plugin)
    }
}

impl PluginPackages for MemoryPackages {
    fn read(&self, _: &str) -> Result<Package, PackageError> {
        Err(PackageError::NotFound)
    }

    fn publish(&self, package: &Package, replace: bool) -> Result<(), PackageError> {
        let id = package.manifest().plugin().id().clone();
        let mut stored = self.stored();
        if stored.contains_key(&id) && !replace {
            return Err(PackageError::Duplicate(id));
        }
        stored.insert(id, package.clone());
        Ok(())
    }

    fn load(&self, plugin: &PluginId, expected: &ContentHash) -> Result<Package, PackageError> {
        match self.stored().get(plugin) {
            None => Err(PackageError::NotFound),
            Some(package) if package.hash() == expected => Ok(package.clone()),
            Some(_) => Err(PackageError::HashChanged),
        }
    }

    fn remove(&self, plugin: &PluginId) -> Result<(), PackageError> {
        self.stored().remove(plugin);
        Ok(())
    }
}

/// What a scripted plugin does: shared between the test and the instances the runtime makes.
#[derive(Clone)]
pub struct Script {
    state: Arc<Mutex<ScriptState>>,
}

struct ScriptState {
    view: Result<View, RuntimeError>,
    events: VecDeque<Result<Effects, RuntimeError>>,
    snapshots: Vec<Snapshot>,
    instances: usize,
}

impl Default for Script {
    fn default() -> Self {
        Self::new()
    }
}

impl Script {
    pub fn new() -> Self {
        let node = Node::new(NodeKind::Text("hello".into()));
        Self {
            state: Arc::new(Mutex::new(ScriptState {
                view: Ok(View {
                    nodes: vec![node],
                    ..View::default()
                }),
                events: VecDeque::new(),
                snapshots: Vec::new(),
                instances: 0,
            })),
        }
    }

    fn state(&self) -> MutexGuard<'_, ScriptState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What `render` answers from now on.
    pub fn render_with(&self, view: Result<View, RuntimeError>) {
        self.state().view = view;
    }

    /// What the next `on_event` answers; later calls answer with no effects.
    pub fn answer_event(&self, answer: Result<Effects, RuntimeError>) {
        self.state().events.push_back(answer);
    }

    /// Every snapshot the guest was given, oldest first.
    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.state().snapshots.clone()
    }

    /// How many instances the runtime made.
    pub fn instances(&self) -> usize {
        self.state().instances
    }
}

/// A runtime whose instances do what the [`Script`] says.
pub struct ScriptedRuntime(Script);

impl ScriptedRuntime {
    pub fn new(script: &Script) -> Arc<Self> {
        Arc::new(Self(script.clone()))
    }
}

impl PluginRuntime for ScriptedRuntime {
    fn instantiate(&self, _: &Package) -> Result<Box<dyn PluginInstance>, RuntimeError> {
        self.0.state().instances += 1;
        Ok(Box::new(ScriptedInstance(self.0.clone())))
    }
}

struct ScriptedInstance(Script);

impl PluginInstance for ScriptedInstance {
    fn render(&self, _: &LocalId, snapshot: &Snapshot) -> Result<View, RuntimeError> {
        let mut state = self.0.state();
        state.snapshots.push(snapshot.clone());
        state.view.clone()
    }

    fn on_event(
        &self,
        _: &LocalId,
        _: Event,
        snapshot: &Snapshot,
    ) -> Result<Effects, RuntimeError> {
        let mut state = self.0.state();
        state.snapshots.push(snapshot.clone());
        state.events.pop_front().unwrap_or(Ok(Effects::default()))
    }
}

/// A package of the plugin `user-acme-clock` with two contributions, `face` (a widget) and
/// `board` (a panel), every capability of the table declared and `monitor.usage` optional.
pub fn package(hash_byte: u8) -> Package {
    package_named("clock", hash_byte)
}

/// The same package under another plugin name.
pub fn package_named(name: &str, hash_byte: u8) -> Package {
    package_with(name, hash_byte, ALL)
}

/// Every capability of the table that is not optional in [`package`].
pub const ALL: &[&str] = &[
    "clock.now",
    "workspace.kind",
    "transfers.counts",
    "notifications.own",
    "notifications.post",
    "panels.set-open",
    "panels.set-badge",
    "storage.set",
    "storage.remove",
];

/// A package that declares exactly these required capabilities (and `monitor.usage` as
/// optional).
pub fn package_with(name: &str, hash_byte: u8, capabilities: &[&str]) -> Package {
    let text = serde_json::json!({
        "api": "0.1",
        "publisher": "acme",
        "id": name,
        "name": format!("Acme {name}"),
        "description": "A test plugin",
        "capabilities": capabilities,
        "optional_capabilities": ["monitor.usage"],
        "contribution": [
            {
                "local": "face", "kind": "widget", "title": "Face",
                "view": { "nodes": [{ "kind": "text", "text": "{clock.now|hh-mm}" }] }
            },
            {
                "local": "board", "kind": "panel", "title": "Board",
                "view": { "nodes": [{ "kind": "text", "text": "board" }] }
            }
        ],
        "settings": {
            "version": 1,
            "group": [{
                "title": "General",
                "field": [{ "key": "city", "label": "City", "kind": "text", "max_len": 40 }]
            }]
        }
    });
    let dto: ManifestDto = serde_json::from_value(text).expect("the fixture is a manifest");
    let manifest = Manifest::validate(&dto, &HostContext::default().with_stand_in_runtime())
        .expect("the fixture is a valid manifest");
    Package::new(ContentHash::new([hash_byte; 32]), manifest, Vec::new())
}

/// The id of [`package`].
pub fn plugin_id() -> PluginId {
    PluginId::owned("user-acme-clock")
}

/// An effect outcome as a test compares it: the key and what happened.
pub type Outcome = (Option<String>, Result<(), nocterm_plugin_api::HostError>);
