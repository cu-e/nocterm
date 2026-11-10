//! Terminal tabs a user plugin drives (`terminals.open`, `terminals.write`) in one window.
//!
//! A plugin terminal is an ordinary terminal tab on an attached session that runs nothing: what
//! is typed into it, by the user or by an AI chat it is attached to, is line-edited here and
//! handed to the plugin as `terminal-line` events; what the plugin writes is shown after the
//! host kept only text, line breaks and colours. The host itself marks a clean prompt for the
//! terminal ([`line::Prompt`]), so an AI chat's input is judged as at a shell's prompt; a
//! plugin's output cannot carry such a marker. Closing the tab tells the plugin
//! `terminal-closed`. A tab opened with `files` also has a file system the plugin serves
//! ([`fs::PluginFs`]): the file browser and AI chats ask it, each request becomes a
//! `file-request` event, and the plugin's `terminals.files` effect answers it. Every tab holds a
//! bar: the plugin's toolbar for its own tabs, if it declares one, drawn along the edge the
//! window's layout gives that toolbar (the plugin's suggestion until the user hides or moves it
//! in the status bar menu) and following it when it changes; the plugin's `terminals.bar` effect
//! only takes it off one tab or puts it back. The tab owns the bar, so it goes when the tab
//! closes. Nothing here reaches a shell, a file of this computer or the network.
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures::{
    FutureExt as _, SinkExt as _, StreamExt as _,
    channel::{mpsc, oneshot},
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, BackgroundExecutor, Context, Subscription, WeakEntity,
    Window,
};
use nocterm_plugin_api::{
    ContributionContext, Event, HostError, Kind, LocalId, Zone,
    capability::{
        files::{
            FILE_TIMEOUT_MS, FileAnswer, FileFailure, FileReply, MAX_FILE_REQUESTS, ServeFiles,
        },
        terminals::{
            Bar, MAX_TERMINALS, Open, OpenTerminal, ShowTerminalBar, Write, WriteTerminal,
            terminal_output,
        },
    },
};
use nocterm_plugin_host::{Call, PluginHost};
use nocterm_plugin_model::{ContributionId, ContributionKind, PluginId, ToolbarTabs};
use nocterm_plugin_ports::{Apply, PluginProviders};
use nocterm_session::{
    CloseReason, Command, ConnectRequest, Event as SessionEvent, Session, SessionDriver, Transport,
};
use nocterm_workspace::{Workspace, WorkspaceEvent};

mod fs;
mod line;

use line::{Fed, LineEditor, Prompt};

/// A file request of a plugin's terminal: the plugin, the terminal and the request's number.
type FileKey = (PluginId, String, u32);
/// The file requests waiting for the plugin's answer, by key.
type Waiting = Rc<RefCell<BTreeMap<FileKey, oneshot::Sender<FileAnswer>>>>;

/// Most writes waiting for a terminal to show them.
const OUTPUT_BACKLOG: usize = 64;

/// Opens a tab on an attached session in the window's workspace: the terminal feature's
/// `open_attached`, composed by the application.
pub(crate) type OpenAttached = Rc<
    dyn Fn(
        &mut Workspace,
        nocterm_terminal::AttachedSession,
        &mut Window,
        &mut Context<Workspace>,
    ) -> Result<(), nocterm_workspace_ports::ContextSelectionError>,
>;

struct OpenTab {
    output: mpsc::Sender<Vec<u8>>,
    alive: Arc<AtomicBool>,
    /// The tab's bar, which the tab owns.
    bar: WeakEntity<nocterm_terminal::TerminalBar>,
    /// Whether the plugin took its toolbar off this tab.
    bar_off: bool,
}

/// The plugin terminals of one window.
pub(crate) struct PluginTerminals {
    window: AnyWindowHandle,
    workspace: WeakEntity<Workspace>,
    open_attached: OpenAttached,
    host: WeakEntity<PluginHost>,
    bars: crate::shell_setup::user_plugin_panel::TerminalBars,
    /// The window's providers, for the events lines raise; released with the window.
    providers: RefCell<Option<PluginProviders>>,
    open: Rc<RefCell<BTreeMap<(PluginId, String), OpenTab>>>,
    /// File requests of the tabs that serve files, waiting for the plugin's answer.
    waiting: Waiting,
    /// Moves the tabs' bars when the window's layout moves or hides their toolbars.
    following: RefCell<Option<Subscription>>,
}

impl PluginTerminals {
    pub(crate) fn new(
        window: AnyWindowHandle,
        workspace: WeakEntity<Workspace>,
        open_attached: OpenAttached,
        host: WeakEntity<PluginHost>,
        bars: crate::shell_setup::user_plugin_panel::TerminalBars,
    ) -> Rc<Self> {
        Rc::new(Self {
            window,
            workspace,
            open_attached,
            host,
            bars,
            providers: RefCell::new(None),
            open: Rc::default(),
            waiting: Rc::default(),
            following: RefCell::new(None),
        })
    }

    /// Keeps the tabs' bars where the window's layout puts the plugins' toolbars, until
    /// [`PluginTerminals::release`].
    pub(crate) fn follow_toolbars(self: &Rc<Self>, cx: &mut App) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let terminals = Rc::downgrade(self);
        let following = cx.subscribe(&workspace, move |_, event, cx| {
            if *event == WorkspaceEvent::ToolbarsChanged
                && let Some(terminals) = terminals.upgrade()
            {
                terminals.place_bars(cx);
            }
        });
        *self.following.borrow_mut() = Some(following);
    }

    /// The providers lines are reported with, which hold these terminals: set once they exist,
    /// and released with [`PluginTerminals::release`] when the window goes.
    pub(crate) fn bind(&self, providers: PluginProviders) {
        *self.providers.borrow_mut() = Some(providers);
    }

    pub(crate) fn release(&self) {
        self.following.borrow_mut().take();
        self.providers.borrow_mut().take();
        self.open.borrow_mut().clear();
        self.waiting.borrow_mut().clear();
    }

    fn live(&self, plugin: &PluginId) -> usize {
        self.open
            .borrow()
            .iter()
            .filter(|((owner, _), tab)| owner == plugin && tab.alive.load(Ordering::Acquire))
            .count()
    }

    /// The plugin's name, which the tab shows as its host.
    fn name_of(&self, plugin: &PluginId, cx: &App) -> String {
        self.host
            .upgrade()
            .and_then(|host| {
                let host = host.read(cx);
                host.manifest(plugin)
                    .map(|manifest| manifest.plugin().name().to_owned())
            })
            .unwrap_or_else(|| plugin.to_string())
    }

    /// The plugin's toolbar for its own tabs, if it declares one: its id and local name.
    fn own_toolbar(&self, plugin: &PluginId, cx: &App) -> Option<(ContributionId, LocalId)> {
        let host = self.host.upgrade()?;
        let manifest = host.read(cx).manifest(plugin)?;
        let toolbar = manifest
            .plugin()
            .contributions()
            .iter()
            .find(|contribution| {
                contribution.kind() == ContributionKind::Toolbar
                    && contribution.tabs() == Some(ToolbarTabs::Own)
            })?;
        let (_, local) = toolbar.id().as_str().split_once('.')?;
        Some((toolbar.id().clone(), LocalId::parse(local).ok()?))
    }

    /// Places the bars of every open tab again.
    fn place_bars(&self, cx: &mut App) {
        let tabs: Vec<_> = self.open.borrow().keys().cloned().collect();
        for (plugin, terminal) in tabs {
            // A tab that closed in between has nothing to place.
            let _ = self.place_bar(&plugin, &terminal, cx);
        }
    }

    /// Draws the plugin's toolbar along the edge of one of its tabs the window's layout gives
    /// it, or takes the bar away while the layout hides it, the plugin took it off this tab or
    /// the plugin declares none. A bar that stays across or along a side keeps its view; one that
    /// turns is drawn anew in the other frame.
    fn place_bar(&self, plugin: &PluginId, terminal: &str, cx: &mut App) -> Result<(), HostError> {
        let (bar, off) = {
            let open = self.open.borrow();
            let tab = open
                .get(&(plugin.clone(), terminal.to_owned()))
                .filter(|tab| tab.alive.load(Ordering::Acquire))
                .ok_or(HostError::Unavailable)?;
            (
                tab.bar.upgrade().ok_or(HostError::Unavailable)?,
                tab.bar_off,
            )
        };
        let placed = (!off)
            .then(|| self.own_toolbar(plugin, cx))
            .flatten()
            .and_then(|(id, local)| {
                let edge = self.workspace.upgrade()?.read(cx).toolbar_edge(&id)?;
                Some((bar_edge(edge), local))
            });
        let Some((edge, local)) = placed else {
            bar.update(cx, |bar, cx| bar.hide(cx));
            return Ok(());
        };
        let side = is_side(edge);
        let kept = {
            let shown = bar.read(cx);
            shown
                .view()
                .filter(|_| shown.edge().map(is_side) == Some(side))
                .cloned()
        };
        let view = match kept {
            Some(view) => view,
            None => {
                let call = self
                    .call(plugin, &local, cx)
                    .ok_or(HostError::Unavailable)?;
                let providers = self
                    .providers
                    .borrow()
                    .clone()
                    .ok_or(HostError::Unavailable)?;
                let host = self.host.upgrade().ok_or(HostError::Unavailable)?;
                let name = self.name_of(plugin, cx);
                self.bars
                    .view(host, &call, terminal, &name, along(edge), providers, cx)
                    .ok_or(HostError::Unavailable)?
            }
        };
        bar.update(cx, |bar, cx| bar.show(edge, view, cx));
        Ok(())
    }

    /// The call that reports the terminal's lines to `local`.
    fn call(&self, plugin: &PluginId, local: &LocalId, cx: &App) -> Option<Call> {
        let host = self.host.upgrade()?;
        let manifest = host.read(cx).manifest(plugin)?;
        let id = nocterm_plugin_model::ContributionId::in_plugin(plugin, local.as_str()).ok()?;
        let contribution = manifest
            .plugin()
            .contributions()
            .iter()
            .find(|contribution| *contribution.id() == id)?;
        Some(Call {
            window: self.window.window_id(),
            plugin: plugin.clone(),
            local: local.clone(),
            context: ContributionContext {
                kind: Kind::of_model(contribution.kind()),
                zone: match contribution.placement().zone {
                    nocterm_plugin_model::Zone::Left => Zone::Left,
                    nocterm_plugin_model::Zone::Center => Zone::Center,
                    nocterm_plugin_model::Zone::Right => Zone::Right,
                },
            },
            in_focus: false,
            terminal: None,
        })
    }
}

impl Apply<OpenTerminal> for PluginTerminals {
    fn apply(&self, plugin: &PluginId, request: Open, cx: &mut App) -> Result<(), HostError> {
        let slot = (plugin.clone(), request.terminal.clone());
        if self
            .open
            .borrow()
            .get(&slot)
            .is_some_and(|tab| tab.alive.load(Ordering::Acquire))
        {
            return Ok(());
        }
        if self.live(plugin) >= MAX_TERMINALS {
            return Err(HostError::Denied);
        }
        let call = self
            .call(plugin, &request.local, cx)
            .ok_or(HostError::Unavailable)?;
        let providers = self
            .providers
            .borrow()
            .clone()
            .ok_or(HostError::Unavailable)?;
        let (files, asks) = if request.files {
            let (asks, asked) = mpsc::channel(MAX_FILE_REQUESTS);
            let files: Arc<dyn nocterm_session::RemoteFs> = Arc::new(fs::PluginFs::new(asks));
            (Some(files), Some(asked))
        } else {
            (None, None)
        };
        let (session, driver) = nocterm_session::channel(files);
        let (output, shown) = mpsc::channel(OUTPUT_BACKLOG);
        let (typed, lines) = mpsc::channel(OUTPUT_BACKLOG);
        let alive = Arc::new(AtomicBool::new(true));
        let name = self.name_of(plugin, cx);
        let target = nocterm_session::Target {
            host: name,
            port: nocterm_session::DEFAULT_PORT,
            user: request.title.clone(),
        };
        let transport = Arc::new(AttachedPlugin(Mutex::new(Some(session))));
        let bar = cx.new(|_| nocterm_terminal::TerminalBar::default());
        let attached = nocterm_terminal::AttachedSession::new(request.title, target, transport)
            .with_bar(bar.clone());
        let (open_attached, workspace) = (self.open_attached.clone(), self.workspace.clone());
        let opened = self.window.update(cx, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                open_attached(workspace, attached, window, cx)
            })
        });
        if !matches!(opened, Ok(Ok(Ok(())))) {
            return Err(HostError::Unavailable);
        }
        let executor = cx.background_executor().clone();
        cx.background_executor()
            .spawn(drive(driver, shown, typed, alive.clone(), executor))
            .detach();
        let tab = Tab {
            host: self.host.clone(),
            call: call.clone(),
            providers,
            terminal: request.terminal,
        };
        if let Some(asked) = asks {
            serve_files(tab.clone(), asked, self.waiting.clone(), cx);
        }
        forward_lines(tab, lines, self.waiting.clone(), cx);
        let bar = bar.downgrade();
        let terminal = slot.1.clone();
        self.open.borrow_mut().insert(
            slot,
            OpenTab {
                output,
                alive,
                bar,
                bar_off: false,
            },
        );
        // A plugin without a toolbar for its tabs, or one the layout hides, shows no bar.
        let _ = self.place_bar(plugin, &terminal, cx);
        Ok(())
    }
}

/// Takes the plugin's toolbar off one of its open tabs (no edge) or puts it back where the
/// window's layout places the toolbar. The edge the plugin names is not read: the layout's is
/// the user's.
impl Apply<ShowTerminalBar> for PluginTerminals {
    fn apply(&self, plugin: &PluginId, request: Bar, cx: &mut App) -> Result<(), HostError> {
        {
            let mut open = self.open.borrow_mut();
            let tab = open
                .get_mut(&(plugin.clone(), request.terminal.clone()))
                .filter(|tab| tab.alive.load(Ordering::Acquire))
                .ok_or(HostError::Unavailable)?;
            tab.bar_off = request.edge.is_none();
        }
        self.place_bar(plugin, &request.terminal, cx)
    }
}

fn bar_edge(edge: nocterm_plugin_model::Edge) -> nocterm_terminal::BarEdge {
    use nocterm_plugin_model::Edge;
    match edge {
        Edge::Top => nocterm_terminal::BarEdge::Top,
        Edge::Bottom => nocterm_terminal::BarEdge::Bottom,
        Edge::Left => nocterm_terminal::BarEdge::Left,
        Edge::Right => nocterm_terminal::BarEdge::Right,
    }
}

/// The zone a toolbar's calls name for its axis: `Center` across the tab, the side otherwise.
fn along(edge: nocterm_terminal::BarEdge) -> Zone {
    match edge {
        nocterm_terminal::BarEdge::Top | nocterm_terminal::BarEdge::Bottom => Zone::Center,
        nocterm_terminal::BarEdge::Left => Zone::Left,
        nocterm_terminal::BarEdge::Right => Zone::Right,
    }
}

fn is_side(edge: nocterm_terminal::BarEdge) -> bool {
    matches!(
        edge,
        nocterm_terminal::BarEdge::Left | nocterm_terminal::BarEdge::Right
    )
}

/// What a window's providers hold of its plugin terminals. The terminals keep those providers
/// for the events their lines raise, so a strong handle here would be a cycle that keeps the
/// terminals, and the terminal source they open tabs with, alive after the window.
pub(crate) struct TerminalsHandle(Weak<PluginTerminals>);

impl TerminalsHandle {
    pub(crate) fn new(terminals: &Rc<PluginTerminals>) -> Self {
        Self(Rc::downgrade(terminals))
    }

    fn terminals(&self) -> Result<Rc<PluginTerminals>, HostError> {
        self.0.upgrade().ok_or(HostError::Unavailable)
    }
}

impl Apply<OpenTerminal> for TerminalsHandle {
    fn apply(&self, plugin: &PluginId, request: Open, cx: &mut App) -> Result<(), HostError> {
        Apply::<OpenTerminal>::apply(&*self.terminals()?, plugin, request, cx)
    }
}

impl Apply<ServeFiles> for TerminalsHandle {
    fn apply(&self, plugin: &PluginId, request: FileReply, cx: &mut App) -> Result<(), HostError> {
        Apply::<ServeFiles>::apply(&*self.terminals()?, plugin, request, cx)
    }
}

impl Apply<ShowTerminalBar> for TerminalsHandle {
    fn apply(&self, plugin: &PluginId, request: Bar, cx: &mut App) -> Result<(), HostError> {
        Apply::<ShowTerminalBar>::apply(&*self.terminals()?, plugin, request, cx)
    }
}

impl Apply<WriteTerminal> for TerminalsHandle {
    fn apply(&self, plugin: &PluginId, request: Write, cx: &mut App) -> Result<(), HostError> {
        Apply::<WriteTerminal>::apply(&*self.terminals()?, plugin, request, cx)
    }
}

/// A tab of a plugin, and how to reach the plugin with its events.
#[derive(Clone)]
struct Tab {
    host: WeakEntity<PluginHost>,
    call: Call,
    providers: PluginProviders,
    terminal: String,
}

/// Hands the lines typed into a tab to its plugin, then that it closed; the file requests of a
/// closed tab are not answered any more.
fn forward_lines(tab: Tab, mut lines: mpsc::Receiver<String>, waiting: Waiting, cx: &mut App) {
    cx.spawn(async move |cx| {
        while let Some(line) = lines.next().await {
            let event = Event::TerminalLine {
                terminal: tab.terminal.clone(),
                line,
            };
            report(&tab.host, &tab.call, event, &tab.providers, cx);
        }
        waiting
            .borrow_mut()
            .retain(|(plugin, open, _), _| *plugin != tab.call.plugin || *open != tab.terminal);
        let closed = Event::TerminalClosed {
            terminal: tab.terminal.clone(),
        };
        report(&tab.host, &tab.call, closed, &tab.providers, cx);
    })
    .detach();
}

/// Hands each file request of a tab to its plugin as a `file-request` event and waits for the
/// answer, at most [`FILE_TIMEOUT_MS`]. A request the plugin is not told of fails at once.
fn serve_files(tab: Tab, mut asked: mpsc::Receiver<fs::Asked>, waiting: Waiting, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut next: u32 = 0;
        while let Some(asked) = asked.next().await {
            let (op, reply) = asked.into_parts();
            let request = next;
            next = next.wrapping_add(1);
            let key: FileKey = (tab.call.plugin.clone(), tab.terminal.clone(), request);
            waiting.borrow_mut().insert(key.clone(), reply);
            let event = Event::FileRequest {
                terminal: tab.terminal.clone(),
                request,
                op,
            };
            if !report(&tab.host, &tab.call, event, &tab.providers, cx) {
                fail(&waiting, &key, "the plugin is not running");
                continue;
            }
            let timeout = cx
                .background_executor()
                .timer(Duration::from_millis(u64::from(FILE_TIMEOUT_MS)));
            let waiting = waiting.clone();
            cx.spawn(async move |_| {
                timeout.await;
                fail(&waiting, &key, "the plugin did not answer in time");
            })
            .detach();
        }
    })
    .detach();
}

/// Fails the request `key` if it still waits.
fn fail(waiting: &Waiting, key: &FileKey, reason: &str) {
    if let Some(reply) = waiting.borrow_mut().remove(key) {
        let _ = reply.send(FileAnswer::Failed(FileFailure::Other(reason.to_owned())));
    }
}

/// The plugin's answer to a file request of one of its tabs.
impl Apply<ServeFiles> for PluginTerminals {
    fn apply(&self, plugin: &PluginId, request: FileReply, _: &mut App) -> Result<(), HostError> {
        let key: FileKey = (plugin.clone(), request.terminal, request.request);
        let reply = self
            .waiting
            .borrow_mut()
            .remove(&key)
            .ok_or(HostError::Unavailable)?;
        reply
            .send(request.answer)
            .map_err(|_| HostError::Unavailable)
    }
}

impl Apply<WriteTerminal> for PluginTerminals {
    fn apply(&self, plugin: &PluginId, request: Write, _: &mut App) -> Result<(), HostError> {
        let mut open = self.open.borrow_mut();
        let tab = open
            .get_mut(&(plugin.clone(), request.terminal))
            .filter(|tab| tab.alive.load(Ordering::Acquire))
            .ok_or(HostError::Unavailable)?;
        tab.output
            .try_send(terminal_output(&request.text).into_bytes())
            .map_err(|_| HostError::RateLimited {
                retry_after_ms: 250,
            })
    }
}

/// Tells the plugin `event`; whether it was told.
fn report(
    host: &WeakEntity<PluginHost>,
    call: &Call,
    event: Event,
    providers: &PluginProviders,
    cx: &mut gpui_kit::AsyncApp,
) -> bool {
    let delivered = host.update(cx, |host, cx| {
        host.deliver(call, None, event, providers, cx)
    });
    match delivered {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            tracing::debug!(plugin = %call.plugin, %error, "a plugin terminal event was dropped");
            false
        }
        Err(_) => false,
    }
}

/// The transport of one plugin terminal: it hands over the session made for it once. The tab
/// reattaches to nothing; a second open is a closed session.
struct AttachedPlugin(Mutex<Option<Session>>);

impl Transport for AttachedPlugin {
    fn open(&self, _: ConnectRequest) -> Session {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .unwrap_or_else(|| {
                let (session, driver) = nocterm_session::channel(None);
                drop(driver);
                session
            })
    }
}

/// Runs the session: what is typed is line-edited and its lines sent on, what the plugin
/// writes is shown, and the terminal is told when the console shows a clean prompt, until the
/// tab closes.
async fn drive(
    driver: SessionDriver,
    mut shown: mpsc::Receiver<Vec<u8>>,
    mut typed: mpsc::Sender<String>,
    alive: Arc<AtomicBool>,
    executor: BackgroundExecutor,
) {
    let (mut editor, mut prompt) = (LineEditor::default(), Prompt::default());
    if driver.emit(SessionEvent::Connected).await
        && driver
            .emit(SessionEvent::Output(Prompt::opened().to_vec()))
            .await
    {
        loop {
            let settled = match prompt.deadline() {
                Some(deadline) => executor
                    .timer(deadline.saturating_duration_since(executor.now()))
                    .boxed(),
                None => futures::future::pending().boxed(),
            };
            let shown_now = futures::select_biased! {
                command = driver.next_command().fuse() => match command {
                    Some(Command::Input(bytes)) => {
                        let fed = editor.feed(&bytes);
                        let marker = prompt.typed(&fed, editor.is_empty(), executor.now());
                        let Fed { mut echo, lines } = fed;
                        echo.extend_from_slice(marker);
                        if !echo.is_empty() && !driver.emit(SessionEvent::Output(echo)).await {
                            break;
                        }
                        for line in lines {
                            if typed.send(line).await.is_err() {
                                break;
                            }
                        }
                        None
                    }
                    Some(Command::Resize(_)) => None,
                    Some(Command::Close) | None => break,
                },
                output = shown.next() => match output {
                    Some(bytes) => {
                        prompt.shown(executor.now());
                        Some(editor.around_output(&bytes))
                    }
                    None => break,
                },
                () = settled.fuse() => {
                    Some(prompt.settle(editor.is_empty(), executor.now()).to_vec())
                }
            };
            if let Some(bytes) = shown_now.filter(|bytes| !bytes.is_empty())
                && !driver.emit(SessionEvent::Output(bytes)).await
            {
                break;
            }
        }
    }
    alive.store(false, Ordering::Release);
    let _ = driver
        .emit(SessionEvent::Closed(CloseReason::ClosedByUser))
        .await;
}

#[cfg(test)]
mod tests;
