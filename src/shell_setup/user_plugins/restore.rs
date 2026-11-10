//! Startup acquires native records/packages/data and actual runtime instances off the GUI.
use super::RecordsFile;
use futures::{FutureExt as _, future::Shared};
use gpui_kit::{App, Entity, Task};
use nocterm_core::SharedCause;
use nocterm_plugin_api::{ContentHash, Grant, PluginPackages, PluginRuntime};
use nocterm_plugin_host::{BrokenPackage, MAX_ENABLED, PluginHost, PreparedPlugin, RestoredPlugin};
use nocterm_plugins_ui::UserPluginError;
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

#[derive(Clone)]
pub(super) struct Ready {
    task: Shared<Task<Result<(), UserPluginError>>>,
    completion: Rc<RefCell<Option<Result<(), UserPluginError>>>>,
}
impl Ready {
    pub(super) fn session() -> Self {
        Self {
            task: Task::ready(Ok(())).shared(),
            completion: Rc::new(RefCell::new(Some(Ok(())))),
        }
    }
    pub(super) async fn wait(&self) -> Result<(), UserPluginError> {
        self.task.clone().await
    }
    pub(super) fn is_ready(&self) -> bool {
        self.completion.borrow().as_ref().is_some_and(Result::is_ok)
    }
    pub(super) fn error(&self) -> Option<UserPluginError> {
        self.completion
            .borrow()
            .as_ref()
            .and_then(|result| result.as_ref().err().cloned())
    }
}

pub(super) fn start(
    root: PathBuf,
    host: Entity<PluginHost>,
    records: Rc<RecordsFile>,
    packages: Arc<dyn PluginPackages>,
    runtime: Arc<dyn PluginRuntime>,
    cx: &mut App,
) -> Ready {
    let loaded = cx.background_executor().spawn(async move {
        let document = super::records::read(&root.join("records.json"))?;
        let mut running = 0;
        let plugins = document
            .plugins
            .iter()
            .map(|record| {
                let active = record.enabled && running < MAX_ENABLED;
                running += usize::from(record.enabled);
                let package = (|| {
                    let hash = ContentHash::parse(&record.hash)?;
                    let package = packages.load(&record.id, &hash)?;
                    if package.manifest().plugin().id() != &record.id {
                        return Err(BrokenPackage::OtherPlugin {
                            found: package.manifest().plugin().id().clone(),
                        });
                    }
                    let accepted: Vec<_> =
                        record.granted_optional.iter().map(String::as_str).collect();
                    Grant::consent(&package, &accepted)?;
                    let instance = active.then(|| runtime.instantiate(&package)).transpose()?;
                    Ok(PreparedPlugin::new(package, instance))
                })();
                let values = super::data::read(&root.join("data"), &record.id);
                (record.clone(), package, values)
            })
            .collect::<Vec<_>>();
        Ok::<_, nocterm_plugin_host::RecordsError>((document, plugins))
    });
    let completion: Rc<RefCell<Option<Result<(), UserPluginError>>>> = Rc::default();
    let status = completion.clone();
    let task = cx
        .spawn(async move |cx| {
            let outcome = async {
                let (document, acquired) = loaded
                    .await
                    .map_err(|error| UserPluginError::Manage(SharedCause::new(error)))?;
                cx.update(|cx| {
                    records
                        .initialize(document.clone(), cx)
                        .map_err(|error| UserPluginError::Manage(SharedCause::new(error)))?;
                    let plugins = acquired
                        .into_iter()
                        .map(|(record, package, values)| {
                            RestoredPlugin::new(record, package, values)
                        })
                        .collect();
                    host.update(cx, |host, cx| {
                        host.restore_prepared(plugins, document.audit, cx)
                    });
                    Ok(())
                })
            }
            .await;
            *status.borrow_mut() = Some(outcome.clone());
            cx.update(|cx| host.update(cx, |_, cx| cx.notify()));
            outcome
        })
        .shared();
    Ready { task, completion }
}
