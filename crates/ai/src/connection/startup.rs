//! Named startup copies are admitted from original borrowed registry facts.
use crate::{
    AgentResourceSettings,
    history::source::SourceContext,
    model::{
        auth::TerminalEnvironment,
        json::deferred::{DecodeLedger, JsonDecodeError},
        native::{NativeList, NativeOwned},
    },
    registry::AgentLaunch,
    workspace::IsolationPaths,
};
use std::path::{Path, PathBuf};

/// Borrowed immutable launch facts; the original Source lease remains separate.
pub struct ManagedStartupRequest<'a> {
    pub launch: &'a AgentLaunch,
    pub working_directory: &'a Path,
    pub create_private: bool,
    pub isolation: Option<&'a IsolationPaths>,
    pub terminal_auth: bool,
    pub resources: &'a AgentResourceSettings,
}
pub struct ManagedLaunch {
    id: String,
    name: String,
    command: String,
    args: Vec<String>,
    env: TerminalEnvironment,
    inherit_env: Vec<String>,
}
impl ManagedLaunch {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn command(&self) -> &str {
        &self.command
    }
    pub fn args(&self) -> &[String] {
        &self.args
    }
    pub fn env(&self) -> &TerminalEnvironment {
        &self.env
    }
    pub fn inherit_env(&self) -> &[String] {
        &self.inherit_env
    }
    fn prepare(
        ledger: &DecodeLedger,
        launch: &AgentLaunch,
    ) -> Result<NativeOwned<Self>, JsonDecodeError> {
        let strings = text(ledger, &launch.id)?
            .join(text(ledger, &launch.name)?)?
            .join(text(ledger, &launch.command)?)?;
        let args = texts(ledger, &launch.args)?;
        let mut env = NativeList::new(ledger, launch.env.len())?;
        for (key, value) in &launch.env {
            env.push(crate::model::native::auth::environment_pair(
                text(ledger, key)?,
                text(ledger, value)?,
            )?)?;
        }
        let env = crate::model::native::auth::environment(env.finish())?;
        let inherit = texts(ledger, &launch.inherit_env)?;
        let ((((((id, name), command), args), env), inherit_env), allocation) =
            strings.join(args)?.join(env)?.join(inherit)?.into_parts();
        Ok(NativeOwned::from_parts(
            Self {
                id,
                name,
                command,
                args,
                env,
                inherit_env,
            },
            allocation,
        ))
    }
}
pub struct ManagedIsolationPaths {
    private: Vec<PathBuf>,
    shared: Vec<PathBuf>,
}
impl ManagedIsolationPaths {
    pub fn private_dirs(&self) -> &[PathBuf] {
        &self.private
    }
    pub fn shared_dirs(&self) -> &[PathBuf] {
        &self.shared
    }
    fn prepare(
        ledger: &DecodeLedger,
        paths: &IsolationPaths,
    ) -> Result<NativeOwned<Self>, JsonDecodeError> {
        let ((private, shared), allocation) = paths_copy(ledger, &paths.private_dirs)?
            .join(paths_copy(ledger, &paths.shared_dirs)?)?
            .into_parts();
        Ok(NativeOwned::from_parts(
            Self { private, shared },
            allocation,
        ))
    }
}
/// Concrete startup data has no Clone route. Its NativeOwned receipt remains
/// pending until every original copied launch/workspace backing is destroyed.
pub struct ManagedStartupFacts {
    launch: ManagedLaunch,
    working_directory: PathBuf,
    create_private: bool,
    isolation: Option<ManagedIsolationPaths>,
    terminal_auth: bool,
    resources: AgentResourceSettings,
}
impl ManagedStartupFacts {
    pub fn launch(&self) -> &ManagedLaunch {
        &self.launch
    }
    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }
    pub fn create_private(&self) -> bool {
        self.create_private
    }
    pub fn isolation(&self) -> Option<&ManagedIsolationPaths> {
        self.isolation.as_ref()
    }
    pub fn terminal_auth(&self) -> bool {
        self.terminal_auth
    }
    pub fn resources(&self) -> &AgentResourceSettings {
        &self.resources
    }
    pub(crate) fn prepare(
        source: &SourceContext,
        request: ManagedStartupRequest<'_>,
    ) -> Result<NativeOwned<Self>, JsonDecodeError> {
        let ledger = source.ledger();
        let launch = ManagedLaunch::prepare(ledger, request.launch)?;
        let working_directory = path_copy(ledger, request.working_directory)?;
        let isolation = match request.isolation {
            Some(paths) => NativeOwned::supplied(ManagedIsolationPaths::prepare(ledger, paths)?),
            None => NativeOwned::absent(ledger)?,
        };
        let (((launch, working_directory), isolation), allocation) = launch
            .join(working_directory)?
            .join(isolation)?
            .into_parts();
        Ok(NativeOwned::from_parts(
            Self {
                launch,
                working_directory,
                isolation,
                create_private: request.create_private,
                terminal_auth: request.terminal_auth,
                resources: request.resources.clone(),
            },
            allocation,
        ))
    }
}
fn text(ledger: &DecodeLedger, value: &str) -> Result<NativeOwned<String>, JsonDecodeError> {
    let allocation = ledger.reserve(value.len())?;
    let copy = String::from(value);
    if copy.capacity() != value.len() {
        return Err(JsonDecodeError::CapacityProfile);
    }
    Ok(NativeOwned::from_parts(copy, allocation))
}
fn texts(
    ledger: &DecodeLedger,
    values: &[String],
) -> Result<NativeOwned<Vec<String>>, JsonDecodeError> {
    let mut native = NativeList::new(ledger, values.len())?;
    for value in values {
        native.push(text(ledger, value)?)?;
    }
    Ok(native.finish())
}
fn path_copy(ledger: &DecodeLedger, value: &Path) -> Result<NativeOwned<PathBuf>, JsonDecodeError> {
    NativeOwned::copy_path(ledger, value)
}
fn paths_copy(
    ledger: &DecodeLedger,
    values: &[PathBuf],
) -> Result<NativeOwned<Vec<PathBuf>>, JsonDecodeError> {
    let mut native = NativeList::new(ledger, values.len())?;
    for value in values {
        native.push(path_copy(ledger, value)?)?;
    }
    Ok(native.finish())
}

#[cfg(test)]
mod tests;

/// The original startup loan survives asynchronous workspace preparation.
pub struct ManagedStartup {
    startup: super::ManagedStartupHandle,
    cancellation: super::ConnectionCancellation,
    source: crate::history::source::SourceLifetime,
}
/// All native launch prerequisites have been admitted under the same original Source.
pub struct ManagedConnectRequest {
    startup: super::ManagedStartupHandle,
    workspace: super::ManagedWorkspaceHandle,
    cancellation: super::ConnectionCancellation,
    source: crate::history::source::SourceLifetime,
}
struct ConnectPreparation {
    startup: NativeOwned<ManagedStartupFacts>,
    source: crate::history::source::SourceLease,
}
impl ManagedStartup {
    pub fn new(
        startup: NativeOwned<ManagedStartupFacts>,
        source: crate::history::source::SourceLease,
    ) -> Result<Self, crate::history::source::SourceError> {
        let preparation = ConnectPreparation { startup, source };
        let context = preparation.source.context()?;
        let startup = super::ManagedStartupHandle::new(&context, preparation.startup)?;
        let cancellation = super::ConnectionCancellation::managed(&context)?;
        let source = preparation.source.into_lifetime()?;
        Ok(Self {
            startup,
            cancellation,
            source,
        })
    }
    pub fn handle(&self) -> super::ManagedStartupHandle {
        self.startup.clone()
    }
    pub fn lifetime(&self) -> crate::history::source::SourceLifetime {
        self.source.clone()
    }
    pub fn cancellation(&self) -> &super::ConnectionCancellation {
        &self.cancellation
    }
    pub fn prepared(
        self,
        workspace: NativeOwned<crate::workspace::PreparedWorkspace>,
    ) -> Result<ManagedConnectRequest, crate::history::source::SourceError> {
        if !workspace.belongs_to_ledger(self.source.context_ref().ledger()) {
            return Err(JsonDecodeError::ForeignLedger.into());
        }
        let workspace = super::ManagedWorkspaceHandle::new(self.source.context_ref(), workspace)?;
        Ok(ManagedConnectRequest {
            startup: self.startup,
            workspace,
            cancellation: self.cancellation,
            source: self.source,
        })
    }
}
impl ManagedConnectRequest {
    pub fn startup(&self) -> &ManagedStartupFacts {
        self.startup.facts()
    }
    pub fn startup_handle(&self) -> super::ManagedStartupHandle {
        self.startup.clone()
    }
    pub fn workspace(&self) -> &crate::workspace::PreparedWorkspace {
        self.workspace.workspace()
    }
    pub fn workspace_handle(&self) -> super::ManagedWorkspaceHandle {
        self.workspace.clone()
    }
    pub fn cancellation(&self) -> &super::ConnectionCancellation {
        &self.cancellation
    }
    pub fn context(&self) -> &SourceContext {
        self.source.context_ref()
    }
    pub fn lifetime(&self) -> crate::history::source::SourceLifetime {
        self.source.clone()
    }
    pub fn into_native_parts(
        self,
    ) -> (
        super::ManagedStartupHandle,
        super::ManagedWorkspaceHandle,
        super::ConnectionCancellation,
        crate::history::source::SourceLifetime,
    ) {
        (self.startup, self.workspace, self.cancellation, self.source)
    }
}
