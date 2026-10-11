//! Dev-only in-memory library and registry; no native I/O or scheduling.
use crate::{
    CatalogBuilder, ExtensionInfo, ThemeCatalog, ThemeChange, ThemeLibrary, ThemeMutation,
    ThemeOperationError, ThemeRegistry, ThemeSource,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
#[derive(Default)]
pub struct InMemoryThemes {
    state: Mutex<Memory>,
    searches: AtomicUsize,
    installs: AtomicUsize,
    before_install: Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
}
#[derive(Default)]
struct Memory {
    available: BTreeMap<String, (ExtensionInfo, Vec<u8>)>,
    installed: BTreeMap<String, (ExtensionInfo, Vec<u8>)>,
    users: BTreeMap<PathBuf, Vec<u8>>,
    failures: BTreeMap<String, bool>,
}
impl InMemoryThemes {
    pub fn before_install(&self, hook: Option<std::sync::Arc<dyn Fn() + Send + Sync>>) {
        *self.before_install.lock().unwrap() = hook;
    }
    pub fn offer(&self, info: ExtensionInfo, bytes: Vec<u8>) {
        self.state
            .lock()
            .unwrap()
            .available
            .insert(info.id.clone(), (info, bytes));
    }
    pub fn user_document(&self, path: PathBuf, bytes: Vec<u8>) {
        self.state.lock().unwrap().users.insert(path, bytes);
    }
    pub fn fail_install(&self, id: &str, fail: bool) {
        self.state.lock().unwrap().failures.insert(id.into(), fail);
    }
    pub fn searches(&self) -> usize {
        self.searches.load(Ordering::SeqCst)
    }
    pub fn installs(&self) -> usize {
        self.installs.load(Ordering::SeqCst)
    }
}
fn catalog(state: &Memory) -> ThemeCatalog {
    let mut builder = CatalogBuilder::new();
    for (path, bytes) in &state.users {
        builder.document(path, ThemeSource::User(path.clone()), bytes);
    }
    for (id, (info, bytes)) in &state.installed {
        let path = PathBuf::from(id).join("extension.toml");
        let manifest = toml::to_string(info).unwrap();
        if let Some(source) = builder.manifest(&path, id, manifest.as_bytes()) {
            builder.document(&PathBuf::from(id).join("themes/theme.json"), source, bytes);
        }
    }
    builder.finish()
}
impl ThemeRegistry for InMemoryThemes {
    fn search(&self, _: &str) -> Result<Vec<ExtensionInfo>, ThemeOperationError> {
        self.searches.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .state
            .lock()
            .unwrap()
            .available
            .values()
            .map(|(info, _)| info.clone())
            .collect())
    }
}
impl ThemeLibrary for InMemoryThemes {
    fn scan(&self) -> ThemeCatalog {
        catalog(&self.state.lock().unwrap())
    }
    fn install(&self, info: &ExtensionInfo) -> Result<ThemeChange, ThemeOperationError> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        let hook = self.before_install.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
        let mut state = self.state.lock().unwrap();
        if state.failures.get(&info.id) == Some(&true) {
            return Err(ThemeOperationError::Unsupported);
        }
        let (_, bytes) = state
            .available
            .get(&info.id)
            .cloned()
            .ok_or(ThemeOperationError::Identity)?;
        crate::parse_family(&bytes)?;
        state
            .installed
            .insert(info.id.clone(), (info.clone(), bytes));
        Ok(ThemeChange {
            operation: ThemeMutation::Install(info.id.clone()),
            catalog: catalog(&state),
            removed: Vec::new(),
        })
    }
    fn uninstall(&self, id: &str) -> Result<ThemeChange, ThemeOperationError> {
        let mut state = self.state.lock().unwrap();
        let before = catalog(&state);
        state.installed.remove(id);
        let removed = before
            .entries()
            .iter()
            .filter(|e| matches!(&e.source,ThemeSource::Installed{id:pack,..} if pack==id))
            .map(|e| (e.name.clone(), e.appearance))
            .collect();
        Ok(ThemeChange {
            operation: ThemeMutation::Uninstall(id.into()),
            catalog: catalog(&state),
            removed,
        })
    }
    fn prepare_user_directory(&self) -> Result<PathBuf, ThemeOperationError> {
        Ok("themes".into())
    }
}
