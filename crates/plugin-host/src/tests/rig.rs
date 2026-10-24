//! One host over in-memory ports and a scripted plugin.

use std::{rc::Rc, sync::Arc};

use gpui::{AppContext as _, Entity, TestAppContext, WindowId};
use nocterm_plugin_api::{
    ContributionContext, Effect, EffectRequest, Effects, Event, Kind, LocalId, Package, View, Zone,
    capability::{
        clock::{ClockNow, Now},
        notifications::{NoticeTone, Post, PostNotice},
        panels::{SetOpen, SetPanelBadge, SetPanelOpen},
    },
};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{
    PluginProviders,
    test_support::{RecordingApply, ScriptedObserve, ScriptedSurface},
};

use crate::{
    Call, CallError, HostPorts, Input, ManageError, PluginHost,
    test_support::{
        MemoryData, MemoryPackages, MemoryRecords, Script, ScriptedRuntime, package, plugin_id,
    },
};

pub(super) struct Rig {
    pub(super) host: Entity<PluginHost>,
    pub(super) records: Rc<MemoryRecords>,
    pub(super) data: Rc<MemoryData>,
    pub(super) packages: Arc<MemoryPackages>,
    pub(super) script: Script,
    pub(super) post: Rc<RecordingApply<PostNotice>>,
    pub(super) open: Rc<RecordingApply<SetPanelOpen>>,
    pub(super) badge: Rc<RecordingApply<SetPanelBadge>>,
    pub(super) surface: Rc<ScriptedSurface>,
    pub(super) clock: Rc<ScriptedObserve<Now>>,
    pub(super) providers: PluginProviders,
    pub(super) window: WindowId,
}

pub(super) fn windows(cx: &mut TestAppContext) -> Vec<WindowId> {
    cx.add_empty_window();
    cx.add_empty_window();
    cx.windows()
        .iter()
        .map(|window| window.window_id())
        .collect()
}

impl Rig {
    pub(super) fn new(cx: &mut TestAppContext) -> Self {
        Self::over(
            MemoryRecords::new(),
            MemoryData::new(),
            MemoryPackages::new(),
            cx,
        )
    }

    /// A host over ports that may already hold records, data and packages.
    pub(super) fn over(
        records: Rc<MemoryRecords>,
        data: Rc<MemoryData>,
        packages: Arc<MemoryPackages>,
        cx: &mut TestAppContext,
    ) -> Self {
        let script = Script::new();
        let post = RecordingApply::<PostNotice>::new();
        let open = RecordingApply::<SetPanelOpen>::new();
        let badge = RecordingApply::<SetPanelBadge>::new();
        let surface = ScriptedSurface::new(Ok(()));
        let clock = ScriptedObserve::<Now>::new(Ok(ClockNow {
            unix_minutes: 600,
            utc_offset_seconds: 0,
        }));
        let providers = PluginProviders::new()
            .with_clock(clock.clone())
            .with_post(post.clone())
            .with_open(open.clone())
            .with_badge(badge.clone())
            .with_surface(surface.clone());
        let ports = HostPorts {
            records: records.clone(),
            data: data.clone(),
            packages: packages.clone(),
            runtime: ScriptedRuntime::new(&script),
        };
        let host = cx.new(|_| PluginHost::new(ports));
        let window = windows(cx)[0];
        Self {
            host,
            records,
            data,
            packages,
            script,
            post,
            open,
            badge,
            surface,
            clock,
            providers,
            window,
        }
    }

    pub(super) fn restore(&self, cx: &mut TestAppContext) {
        self.host.update(cx, |host, cx| host.restore(cx));
    }

    pub(super) fn install(
        &self,
        package: Package,
        accepted: &[&str],
        cx: &mut TestAppContext,
    ) -> Result<PluginId, ManageError> {
        self.host
            .update(cx, |host, cx| host.install(package, accepted, cx))
    }

    /// Installs the standard package and returns its id.
    pub(super) fn installed(&self, cx: &mut TestAppContext) -> PluginId {
        self.install(package(1), &[], cx)
            .expect("the package installs")
    }

    pub(super) fn call(&self, local: &str) -> Call {
        Call {
            window: self.window,
            plugin: plugin_id(),
            local: LocalId::parse(local).expect("a local id"),
            context: ContributionContext {
                kind: Kind::Widget,
                zone: Zone::Right,
            },
            in_focus: false,
        }
    }

    pub(super) fn render(&self, local: &str, cx: &mut TestAppContext) -> Result<View, CallError> {
        let call = self.call(local);
        self.host
            .update(cx, |host, cx| host.render(&call, &self.providers, cx))
    }

    pub(super) fn event(
        &self,
        local: &str,
        input: Input,
        event: Event,
        cx: &mut TestAppContext,
    ) -> Result<(), CallError> {
        let call = self.call(local);
        self.host.update(cx, |host, cx| {
            host.handle(&call, input, event, &self.providers, cx)
        })
    }

    /// A pointer press on a button of the plugin.
    pub(super) fn press(&self, local: &str, cx: &mut TestAppContext) -> Result<(), CallError> {
        self.event(local, Input::Pointer, activated(), cx)
    }

    /// The outcomes the plugin sees on its next event, which the call consumes.
    pub(super) fn outcomes(&self, cx: &mut TestAppContext) -> Vec<crate::test_support::Outcome> {
        let before = self.script.snapshots().len();
        self.event("face", Input::Host, Event::SettingsChanged, cx)
            .expect("the event runs");
        let snapshots = self.script.snapshots();
        snapshots[before]
            .outcomes
            .iter()
            .map(|outcome| (outcome.key.clone(), outcome.result))
            .collect()
    }
}

pub(super) fn activated() -> Event {
    Event::Activated { key: "go".into() }
}

pub(super) fn effects(requests: Vec<(Option<&str>, Effect)>) -> Effects {
    Effects(
        requests
            .into_iter()
            .map(|(key, effect)| EffectRequest {
                key: key.map(str::to_owned),
                effect,
            })
            .collect(),
    )
}

pub(super) fn post_effect(text: &str) -> Effect {
    Effect::NotificationsPost(Post {
        tone: NoticeTone::Info,
        text: text.into(),
        notice_key: None,
    })
}

pub(super) fn open_effect(local: &str, open: bool) -> Effect {
    Effect::PanelsSetOpen(SetOpen {
        local: LocalId::parse(local).expect("a local id"),
        open,
    })
}
