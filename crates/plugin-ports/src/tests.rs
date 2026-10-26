use gpui::TestAppContext;
use nocterm_plugin_api::{
    Effect, HostError, LocalId, Observed,
    capability::{
        clock::{ClockNow, Now},
        monitor::UsageNow,
        notifications::{NoticeTone, OwnNotices, Post, PostNotice},
        panels::{SetBadge, SetOpen, SetPanelOpen},
        storage::Set,
        transfers::{Counts, CountsNow},
        workspace::{Kind, WorkspaceKind, WorkspaceState},
    },
};
use nocterm_plugin_model::PluginId;

use crate::{
    PluginProviders,
    test_support::{RecordingApply, ScriptedObserve, ScriptedSurface},
};

fn plugin() -> PluginId {
    PluginId::owned("user-acme-clock")
}

fn local() -> LocalId {
    LocalId::parse("face").unwrap()
}

#[gpui::test]
fn a_window_without_providers_answers_unavailable(cx: &mut TestAppContext) {
    let providers = PluginProviders::new();
    cx.update(|cx| {
        for id in [
            "clock.now",
            "workspace.kind",
            "monitor.usage",
            "transfers.counts",
            "notifications.own",
        ] {
            assert_eq!(
                providers.read(id, &plugin(), cx),
                Err(HostError::Unavailable),
                "{id}"
            );
        }
        assert_eq!(
            providers.permit(&plugin(), &local(), cx),
            Err(HostError::Unavailable)
        );
    });
}

#[gpui::test]
fn a_capability_that_is_not_an_observation_is_unavailable(cx: &mut TestAppContext) {
    let providers = PluginProviders::new();
    cx.update(|cx| {
        assert_eq!(
            providers.read("storage.set", &plugin(), cx),
            Err(HostError::Unavailable)
        );
        assert_eq!(
            providers.read("nothing.like-it", &plugin(), cx),
            Err(HostError::Unavailable)
        );
    });
}

#[gpui::test]
fn each_provider_answers_its_own_capability_and_no_other(cx: &mut TestAppContext) {
    let clock = ScriptedObserve::<Now>::new(Ok(ClockNow {
        unix_minutes: 5,
        utc_offset_seconds: 60,
    }));
    let state = WorkspaceState {
        kind: WorkspaceKind::Remote,
        connected: true,
    };
    let workspace = ScriptedObserve::<Kind>::new(Ok(state));
    let usage = ScriptedObserve::<UsageNow>::new(Err(HostError::Unavailable));
    let counts = ScriptedObserve::<CountsNow>::new(Ok(Counts {
        active: 2,
        failed: 1,
    }));
    let providers = PluginProviders::new()
        .with_clock(clock.clone())
        .with_workspace(workspace.clone())
        .with_usage(usage.clone())
        .with_counts(counts.clone());
    cx.update(|cx| {
        assert!(matches!(
            providers.read("clock.now", &plugin(), cx),
            Ok(Observed::Clock(ClockNow {
                unix_minutes: 5,
                ..
            }))
        ));
        assert_eq!(
            providers.read("workspace.kind", &plugin(), cx),
            Ok(Observed::Workspace(state))
        );
        assert_eq!(
            providers.read("monitor.usage", &plugin(), cx),
            Err(HostError::Unavailable)
        );
        assert_eq!(
            providers.read("transfers.counts", &plugin(), cx),
            Ok(Observed::Counts(Counts {
                active: 2,
                failed: 1
            }))
        );
        assert_eq!(
            providers.read("notifications.own", &plugin(), cx),
            Err(HostError::Unavailable)
        );
    });
    assert_eq!(
        [
            clock.reads(),
            workspace.reads(),
            usage.reads(),
            counts.reads()
        ],
        [1, 1, 1, 1]
    );
}

#[gpui::test]
fn an_effect_goes_to_its_own_provider_with_the_plugin_that_asked(cx: &mut TestAppContext) {
    let post = RecordingApply::<PostNotice>::new();
    let open = RecordingApply::<SetPanelOpen>::new();
    let providers = PluginProviders::new()
        .with_post(post.clone())
        .with_open(open.clone());
    let notice = Post {
        tone: NoticeTone::Info,
        text: "hello".into(),
        notice_key: None,
    };
    cx.update(|cx| {
        assert_eq!(
            providers.perform(&plugin(), Effect::NotificationsPost(notice.clone()), cx),
            Ok(())
        );
        let request = SetOpen {
            local: local(),
            open: true,
        };
        assert_eq!(
            providers.perform(&plugin(), Effect::PanelsSetOpen(request), cx),
            Ok(())
        );
    });
    assert_eq!(post.requests(), vec![(plugin(), notice)]);
    assert_eq!(open.requests().len(), 1);
}

#[gpui::test]
fn an_effect_without_a_provider_is_unavailable_and_storage_is_never_a_provider_effect(
    cx: &mut TestAppContext,
) {
    let providers = PluginProviders::new();
    cx.update(|cx| {
        let badge = SetBadge {
            local: local(),
            badge: Some(3),
        };
        assert_eq!(
            providers.perform(&plugin(), Effect::PanelsSetBadge(badge), cx),
            Err(HostError::Unavailable)
        );
        let set = Set {
            key: "k".into(),
            value: "v".into(),
        };
        assert_eq!(
            providers.perform(&plugin(), Effect::StorageSet(set), cx),
            Err(HostError::Denied)
        );
    });
}

#[gpui::test]
fn the_surface_decides_whether_a_contribution_may_be_acted_on(cx: &mut TestAppContext) {
    let surface = ScriptedSurface::new(Ok(()));
    let providers = PluginProviders::new().with_surface(surface.clone());
    cx.update(|cx| {
        assert_eq!(providers.permit(&plugin(), &local(), cx), Ok(()));
        surface.answer_with(Err(HostError::Denied));
        assert_eq!(
            providers.permit(&plugin(), &local(), cx),
            Err(HostError::Denied)
        );
    });
    assert_eq!(surface.asked().len(), 2);
}

#[gpui::test]
fn own_notices_are_read_for_the_plugin_that_asks(cx: &mut TestAppContext) {
    use nocterm_plugin_api::capability::notifications::Own;
    let own = ScriptedObserve::<Own>::new(Ok(OwnNotices::default()));
    let providers = PluginProviders::new().with_own_notices(own);
    cx.update(|cx| {
        assert_eq!(
            providers.read("notifications.own", &plugin(), cx),
            Ok(Observed::Notices(OwnNotices::default()))
        );
    });
}

#[test]
fn a_record_says_which_capabilities_it_provides() {
    let none = PluginProviders::new();
    for id in ["clock.now", "notifications.post", "panels.set-open"] {
        assert!(!none.provides(id), "{id}");
    }

    let some = PluginProviders::new()
        .with_clock(ScriptedObserve::<Now>::new(Err(HostError::Unavailable)))
        .with_open(RecordingApply::<SetPanelOpen>::new());

    assert!(some.provides("clock.now"));
    assert!(some.provides("panels.set-open"));
    assert!(!some.provides("panels.set-badge"));
    assert!(!some.provides("workspace.kind"));
    assert!(
        !some.provides("storage.set"),
        "the host provides storage itself"
    );
    assert!(!some.provides("nothing.like-it"));
}
