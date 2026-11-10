//! A status bar widget asks its plugin for a view only while the bar draws it.
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, TestAppContext,
    Window, WindowHandle, div, prelude::*,
};
use nocterm_core::{Paths, clock::WallClock, clock::WallTime};
use nocterm_plugin_api::{
    ContributionContext, Kind, LocalId, Zone,
    capability::clock::{ClockNow, Now},
};
use nocterm_plugin_host::{Call, PluginHost};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::{PluginProviders, test_support::ScriptedObserve};
use nocterm_plugin_view::FrameKind;
use nocterm_store::{SettingsConfig, ShutdownCoordinator};

use super::{
    content::{Content, FrameSpec},
    driver::RefreshDriver,
    views::UserPluginWidget,
};
use crate::shell_setup::{UserPluginMode, UserShell, plugin_catalog};

struct Noon;
impl WallClock for Noon {
    fn now(&self) -> WallTime {
        WallTime::default()
    }
}

/// A bar that draws its widget only when told to, the way the status bar leaves out what does
/// not fit.
struct Bar {
    widget: Entity<UserPluginWidget>,
    drawn: bool,
}

impl Render for Bar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.widget
            .update(cx, |widget, cx| widget.set_visible(self.drawn, cx));
        div().when(self.drawn, |bar| bar.child(self.widget.clone()))
    }
}

struct Rig {
    window: WindowHandle<Bar>,
    host: Entity<PluginHost>,
    plugin: PluginId,
    /// How many views the widget's content showed.
    shown: Rc<Cell<usize>>,
    clock: Rc<ScriptedObserve<Now>>,
}

fn install_clock(host: &Entity<PluginHost>, cx: &mut TestAppContext) -> PluginId {
    cx.update(|cx| {
        host.update(cx, |host, cx| {
            let package = host
                .read_package(&format!(
                    "{}/examples/plugins/clock.toml",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .expect("the example reads");
            let id = host.install(package, &[], cx).expect("it installs");
            host.set_placeable(&id, true, cx).expect("it is installed");
            id
        })
    })
}

fn rig(cx: &mut TestAppContext) -> Rig {
    let catalog = Arc::new(plugin_catalog().expect("the application catalog"));
    let state = tempfile::tempdir().unwrap();
    let paths = Paths::rooted_at(state.path());
    let shell = cx.update(|cx| {
        gpui_kit::init(cx);
        let settings = SettingsConfig::in_memory(Default::default()).install(cx);
        nocterm_ui::init(nocterm_ui::DesignTokens::builtin(), settings.clone(), cx);
        UserShell::install(
            &catalog,
            &paths,
            &ShutdownCoordinator::default(),
            Arc::new(Noon),
            UserPluginMode::Dev("clock.toml".into()),
            &settings,
            cx,
        )
    });
    let host = shell.host().expect("a developer session loads").clone();
    let plugin = install_clock(&host, cx);
    let shown = Rc::new(Cell::new(0));
    let clock = ScriptedObserve::<Now>::new(Ok(ClockNow {
        unix_minutes: 0,
        utc_offset_seconds: 0,
    }));
    let window = cx.add_window({
        let (host, plugin, shown) = (host.clone(), plugin.clone(), shown.clone());
        let clock = clock.clone();
        move |window, cx| {
            let call = Call {
                window: window.window_handle().window_id(),
                plugin,
                local: LocalId::parse("face").unwrap(),
                context: ContributionContext {
                    kind: Kind::Widget,
                    zone: Zone::Right,
                },
                in_focus: false,
                terminal: None,
            };
            let driver = RefreshDriver::new(host.clone(), window.window_handle().window_id(), cx);
            let widget = cx.new(|cx| {
                let content = Content::new(
                    host,
                    driver.clone(),
                    call,
                    FrameSpec {
                        name: "Example Clock",
                        kind: FrameKind::Bar,
                        dev: false,
                    },
                    PluginProviders::new().with_clock(clock.clone()),
                    cx,
                );
                let view = content.read(cx).view().clone();
                cx.observe(&view, move |_, _, _| shown.set(shown.get() + 1))
                    .detach();
                UserPluginWidget::new(content, "Clock".into())
            });
            cx.on_release(move |_, cx| driver.update(cx, |driver, cx| driver.close(cx)))
                .detach();
            Bar {
                widget,
                drawn: true,
            }
        }
    });
    drop(state);
    let rig = Rig {
        window,
        host,
        plugin,
        shown,
        clock,
    };
    rig.settle(cx);
    rig
}

impl Rig {
    /// One refresh step of the widget, and the frame it leads to.
    fn settle(&self, cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(250));
        cx.run_until_parked();
        cx.update_window(*self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn change_label(&self, label: &str, cx: &mut TestAppContext) {
        let mut values = nocterm_plugin_model::SettingValues::default();
        values.insert(
            "label",
            nocterm_plugin_model::SettingValue::Text(label.into()),
        );
        self.host
            .update(cx, |host, cx| {
                host.replace_values(&self.plugin, &values, cx)
            })
            .unwrap();
    }

    fn draw(&self, drawn: bool, cx: &mut TestAppContext) {
        self.window
            .update(cx, |bar, _, cx| {
                bar.drawn = drawn;
                cx.notify();
            })
            .unwrap();
        self.settle(cx);
    }

    /// Makes the widget's view stale, as a change of the plugin's records does.
    fn change_records(&self, placeable: bool, cx: &mut TestAppContext) {
        cx.update(|cx| {
            self.host.update(cx, |host, cx| {
                host.set_placeable(&self.plugin, placeable, cx)
                    .expect("it is installed");
            });
        });
    }
}

#[gpui_kit::test]
fn an_unchanged_view_after_a_record_change_keeps_the_drawn_tree(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let before = rig.shown.get();
    assert!(before > 0, "the first view is asked for at once");

    rig.change_records(false, cx);
    rig.settle(cx);

    assert_eq!(
        rig.shown.get(),
        before,
        "equal nodes and style do not rebuild or notify"
    );
}

#[gpui_kit::test]
fn a_widget_the_bar_does_not_draw_asks_its_plugin_for_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx);
    rig.draw(false, cx);
    // The step that follows the last draw may still ask once; then the widget is quiet.
    rig.change_records(true, cx);
    rig.settle(cx);
    let hidden = rig.shown.get();
    let reads = rig.clock.reads();
    assert!(
        reads > 0,
        "the visible clock actually sampled its granted capability"
    );

    for round in 0..20 {
        rig.change_records(round % 2 == 0, cx);
        rig.settle(cx);
    }

    assert_eq!(
        rig.shown.get(),
        hidden,
        "20 steps over 5 seconds asked for nothing"
    );
    assert_eq!(
        rig.clock.reads(),
        reads,
        "20 hidden steps do not sample the clock, even after cached observations expire"
    );
}

#[gpui_kit::test]
fn a_widget_asks_again_once_the_bar_draws_it_again(cx: &mut TestAppContext) {
    let rig = rig(cx);
    rig.draw(false, cx);
    rig.change_records(true, cx);
    rig.settle(cx);
    let hidden = rig.shown.get();
    let reads = rig.clock.reads();
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    assert_eq!(
        rig.clock.reads(),
        reads,
        "hidden time alone samples nothing"
    );

    rig.change_label("Changed", cx);
    rig.draw(true, cx);
    rig.change_records(false, cx);
    rig.settle(cx);

    assert!(rig.shown.get() > hidden);
    assert!(
        rig.clock.reads() > reads,
        "visible interest samples again after the observation cache expires"
    );
}

#[gpui_kit::test]
fn a_visible_widget_publishes_changed_content_once_and_keeps_equal_refreshes_quiet(
    cx: &mut TestAppContext,
) {
    let rig = rig(cx);
    let before = rig.shown.get();
    rig.change_label("Changed", cx);
    rig.settle(cx);
    assert!(rig.shown.get() > before);
    let changed = rig.shown.get();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    rig.settle(cx);
    assert_eq!(
        rig.shown.get(),
        changed,
        "the minute deadline may sample without rebuilding equal nodes"
    );
}
