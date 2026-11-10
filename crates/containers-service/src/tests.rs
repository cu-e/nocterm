use futures::FutureExt as _;
use std::sync::Arc;

use gpui::{Entity, TestAppContext};
use nocterm_containers::{Changes, ContainersError, Engine, State};
use nocterm_session::HostExec;

use crate::{
    ContainerHosts, HostContainers, ListingStatus, RETRY, SETTLE,
    test_support::{EXITED, FakeDockerHost, IMAGE, docker_event},
};

fn watch(
    cx: &mut TestAppContext,
    hosts: &ContainerHosts,
    host: &Arc<FakeDockerHost>,
) -> Entity<HostContainers> {
    let exec: Arc<dyn HostExec> = host.clone();
    let watched = cx.update(|cx| hosts.watch(&exec, cx));
    cx.run_until_parked();
    watched
}

fn settle(cx: &mut TestAppContext) {
    cx.executor().advance_clock(SETTLE * 2);
    cx.run_until_parked();
}

/// Lets GPUI drop entities nobody holds any more.
fn release(cx: &mut TestAppContext) {
    cx.update(|_| {});
    cx.run_until_parked();
}

fn listings(host: &FakeDockerHost) -> (usize, usize) {
    (host.count("docker ps"), host.count("docker images"))
}

#[gpui::test]
fn every_window_on_a_host_shares_one_watch(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let first = watch(cx, &hosts, &local);
    let second = watch(cx, &hosts, &local);
    assert_eq!(first.entity_id(), second.entity_id());
    assert_eq!(local.count("docker events"), 1);
    assert_eq!(listings(&local), (1, 1));
    first.read_with(cx, |host, _| {
        assert_eq!(host.engine(), Some(Engine::Docker));
        assert!(matches!(host.status(), ListingStatus::Ready));
        assert_eq!(host.snapshot().running(), 1);
    });

    let other = FakeDockerHost::new(true);
    let elsewhere = watch(cx, &hosts, &other);
    assert_ne!(elsewhere.entity_id(), first.entity_id());
    assert_eq!(other.count("docker events"), 1);
}

#[gpui::test]
fn health_checks_list_nothing_again(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let _watched = watch(cx, &hosts, &local);
    for _ in 0..20 {
        for action in [
            "exec_create: pg_isready",
            "exec_start: pg_isready",
            "exec_die",
        ] {
            local.announce(&docker_event("container", action, "pg"));
        }
        settle(cx);
    }
    local.announce(&docker_event("container", "health_status: healthy", "pg"));
    settle(cx);
    assert_eq!(listings(&local), (2, 1), "a new health status, once");
    local.announce(&docker_event("container", "health_status: healthy", "pg"));
    settle(cx);
    assert_eq!(listings(&local), (2, 1), "the same status again is no news");
}

#[gpui::test]
fn a_change_lists_only_what_it_changed_once_per_burst(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let watched = watch(cx, &hosts, &local);

    local.set_containers(EXITED);
    for action in ["kill", "die", "stop"] {
        local.announce(&docker_event("container", action, "a1"));
    }
    settle(cx);
    assert_eq!(
        listings(&local),
        (2, 1),
        "containers once, images not at all"
    );
    watched.read_with(cx, |host, _| {
        assert_eq!(host.snapshot().containers[0].state, State::Exited);
    });

    local.set_images(IMAGE);
    local.announce(&docker_event("image", "pull", "nginx:1"));
    settle(cx);
    assert_eq!(listings(&local), (2, 2), "images alone");
    watched.read_with(cx, |host, _| {
        assert_eq!(host.snapshot().images[0].name(), "nginx:1");
    });
    assert!(local.following(0), "the same stream is still followed");
}

#[gpui::test]
fn asking_to_relist_reuses_the_followed_stream(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let watched = watch(cx, &hosts, &local);
    watched.update(cx, |host, cx| host.relist(Changes::CONTAINERS, cx));
    settle(cx);
    assert_eq!(listings(&local), (2, 1));
    watched.update(cx, |host, cx| host.relist(Changes::ALL, cx));
    settle(cx);
    assert_eq!(listings(&local), (3, 2));
    assert_eq!(local.count("docker events"), 1);
    assert!(local.following(0));
}

#[gpui::test]
fn the_watch_stops_when_nobody_holds_it(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let first = watch(cx, &hosts, &local);
    let second = watch(cx, &hosts, &local);
    drop(first);
    release(cx);
    assert!(local.following(0), "one window still shows it");
    drop(second);
    release(cx);
    assert_eq!(local.followers(), 0, "the last one let go");

    let _again = watch(cx, &hosts, &local);
    assert_eq!(
        local.count("docker events"),
        2,
        "a new watch, from the start"
    );
    assert_eq!(local.followers(), 1);
}

#[gpui::test]
fn without_events_the_host_is_listed_every_retry(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let _watched = watch(cx, &hosts, &local);
    local.end_events();
    cx.run_until_parked();
    assert_eq!(listings(&local), (1, 1));
    cx.executor().advance_clock(RETRY);
    cx.run_until_parked();
    assert_eq!(listings(&local), (2, 2));
    assert_eq!(local.count("docker events"), 2);
}

#[gpui::test]
fn a_host_without_an_engine_is_asked_again_only_on_request(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let bare = FakeDockerHost::new(false);
    let watched = watch(cx, &hosts, &bare);
    watched.read_with(cx, |host, _| {
        assert!(matches!(
            host.status(),
            ListingStatus::Failed(ContainersError::NotInstalled)
        ));
    });
    let asked = bare.ran().len();
    cx.executor().advance_clock(RETRY * 6);
    cx.run_until_parked();
    assert_eq!(bare.ran().len(), asked, "not asked again by itself");

    let same = watch(cx, &hosts, &bare);
    assert_eq!(same.entity_id(), watched.entity_id());
    assert!(
        bare.ran().len() > asked,
        "showing it again looks for an engine anew"
    );
}

#[gpui::test]
fn a_refresh_during_initial_listing_is_not_lost(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    let gate = nocterm_session::testing::Gate::pending();
    let waiting = gate.clone();
    local.suspend_listing(async move { waiting.wait().await }.boxed());
    let watched = watch(cx, &hosts, &local);
    assert_eq!(
        local.count("docker events"),
        1,
        "subscribed before snapshot"
    );
    local.set_containers(EXITED);
    watched.update(cx, |host, cx| host.relist(Changes::CONTAINERS, cx));
    gate.release();
    cx.run_until_parked();
    settle(cx);
    watched.read_with(cx, |host, _| {
        assert_eq!(host.snapshot().containers[0].state, State::Exited)
    });
    assert_eq!(listings(&local), (2, 1));
}

#[gpui::test]
fn a_stalled_listing_times_out_and_can_be_retried(cx: &mut TestAppContext) {
    let hosts = ContainerHosts::new();
    let local = FakeDockerHost::new(true);
    local.suspend_listing(futures::future::pending().boxed());
    let watched = watch(cx, &hosts, &local);
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(31));
    cx.run_until_parked();
    watched.read_with(cx, |host, _| {
        assert!(matches!(
            host.status(),
            ListingStatus::Failed(ContainersError::TimedOut)
        ))
    });
    watched.update(cx, |host, cx| host.relist(Changes::ALL, cx));
    cx.run_until_parked();
    watched.read_with(cx, |host, _| {
        assert!(matches!(host.status(), ListingStatus::Ready))
    });
}
