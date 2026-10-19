//! Connection membership and retained process-cleanup ownership.
use super::{Client, Connection};
use gpui::{EntityId, Task};
use nocterm_ai::{AgentCommands, AgentInfo, AgentLaunch};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Default)]
pub(super) struct ConnectionManager {
    active: HashMap<u64, Connection>,
    closing: HashMap<u64, String>,
    cleanup: HashMap<u64, Arc<super::process_cleanup::ProcessCleanup>>,
    serial: u64,
    startups: HashMap<u64, super::startup::Completion>,
}
impl ConnectionManager {
    pub(super) fn reserve_key(
        &mut self,
        launch: &AgentLaunch,
        workdir: &Path,
        isolated: bool,
    ) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        launch.hash(&mut hasher);
        workdir.hash(&mut hasher);
        isolated.hash(&mut hasher);
        self.serial().hash(&mut hasher);
        hasher.finish()
    }
    pub(super) fn serial(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }
    pub(super) fn iter(&self) -> impl Iterator<Item = (&u64, &Connection)> {
        self.active.iter()
    }
    pub(super) fn get(&self, key: u64) -> Option<&Connection> {
        self.active.get(&key)
    }
    pub(super) fn insert(&mut self, key: u64, connection: Connection) {
        self.active.insert(key, connection);
    }
    pub(super) fn connecting(&mut self, key: u64, task: Task<()>) {
        if let Some(connection) = self.active.get_mut(&key) {
            connection._connecting = Some(task)
        }
    }
    pub(super) fn connected(
        &mut self,
        key: u64,
        commands: Arc<dyn AgentCommands>,
        info: AgentInfo,
        workdir: PathBuf,
        events: Task<()>,
    ) -> Vec<Client> {
        let Some(connection) = self.active.get_mut(&key) else {
            return Vec::new();
        };
        connection.commands = Some(commands.clone());
        connection.info = Some(info);
        connection.workdir = workdir;
        connection._events = Some(events);
        let clients = connection.users.values().cloned().collect();
        self.retain_cleanup(key, commands);
        clients
    }
    pub(super) fn occupied(&self) -> usize {
        self.active.len() + self.closing.len()
    }
    pub(super) fn active(&self) -> bool {
        !self.active.is_empty() || !self.closing.is_empty()
    }
    pub(super) fn same_chat(&self, id: &str) -> bool {
        self.active
            .values()
            .any(|connection| connection.chat_id == id)
            || self.closing.values().any(|chat| chat == id)
    }
    pub(super) fn closing_keys(&self) -> impl Iterator<Item = &u64> {
        self.closing.keys()
    }
    pub(super) fn release(
        &mut self,
        key: u64,
        owner: EntityId,
        commands: Option<Arc<dyn AgentCommands>>,
    ) -> Option<(
        Connection,
        Option<Arc<super::process_cleanup::ProcessCleanup>>,
    )> {
        let mut connection = self.active.remove(&key)?;
        connection.cancellation.cancel();
        connection.users.remove(&owner);
        self.closing.insert(key, connection.chat_id.clone());
        let commands = commands.or_else(|| connection.commands.take());
        if commands.is_none()
            && let Some(completion) = connection.startup_completion.as_ref()
        {
            self.startups.insert(key, completion.clone());
        }
        let cleanup = commands.map(|commands| self.retain_cleanup(key, commands));
        Some((connection, cleanup))
    }
    pub(super) fn closed(&mut self, key: u64) {
        self.closing.remove(&key);
        self.cleanup.remove(&key);
        self.startups.remove(&key);
    }
    fn retain_cleanup(
        &mut self,
        key: u64,
        commands: Arc<dyn AgentCommands>,
    ) -> Arc<super::process_cleanup::ProcessCleanup> {
        let cleanup = self
            .cleanup
            .values()
            .find(|cleanup| cleanup.owns(&commands))
            .cloned()
            .unwrap_or_else(|| Arc::new(super::process_cleanup::ProcessCleanup::new(commands)));
        self.cleanup.insert(key, cleanup.clone());
        cleanup
    }
    pub(super) fn take_cleanup(&mut self) -> CleanupWork {
        let mut startups = std::mem::take(&mut self.startups)
            .into_values()
            .collect::<Vec<_>>();
        for (_, mut connection) in self.active.drain() {
            connection.cancellation.cancel();
            connection._connecting.take();
            connection._events.take();
            if let Some(completion) = connection.startup_completion.take() {
                startups.push(completion);
            }
        }
        let mut owners = Vec::new();
        for (_, cleanup) in self.cleanup.drain() {
            if !owners.iter().any(|other| Arc::ptr_eq(other, &cleanup)) {
                owners.push(cleanup);
            }
        }
        let processes = owners.iter().map(|owner| owner.capture()).collect();
        self.closing.clear();
        CleanupWork {
            processes,
            startups,
        }
    }
}

pub(super) struct CleanupWork {
    processes: Vec<super::startup::Completion>,
    startups: Vec<super::startup::Completion>,
}
impl CleanupWork {
    pub(super) async fn run(self) -> Result<(), nocterm_store::StoreError> {
        let processes = futures::future::join_all(self.processes);
        let startups = futures::future::join_all(self.startups);
        let (processes, startups) = futures::future::join(processes, startups).await;
        for result in processes {
            result.map_err(nocterm_store::StoreError::backend)?;
        }
        for result in startups {
            result.map_err(nocterm_store::StoreError::backend)?;
        }
        Ok(())
    }
}
