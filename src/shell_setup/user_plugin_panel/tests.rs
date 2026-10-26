//! A status bar widget asks its plugin for a view only while the bar draws it.
use std::{cell::Cell, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, TestAppContext,
    Window, WindowHandle, div, prelude::*,
};
use nocterm_core::{Paths, clock::WallClock, clock::WallTime};
use nocterm_plugin_api::{ContributionContext, Kind, LocalId, Zone};
use nocterm_plugin_host::{Call, PluginHost};
use nocterm_plugin_model::PluginId;
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::FrameKind;
use nocterm_store::{SettingsConfig, ShutdownCoordinator};

use super::{content::Content, views::UserPluginWidget};
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
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().when(self.drawn, |bar| bar.child(self.widget.clone()))
    }
}

struct Rig {
    window: WindowHandle<Bar>,
    host: Entity<PluginHost>,
    plugin: PluginId,
    /// How many views the widget's content showed.
    shown: Rc<Cell<usize>>,
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
    let plugin = cx.update(|cx| {
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
    });
    let shown = Rc::new(Cell::new(0));
    let window = cx.add_window({
        let (host, plugin, shown) = (host.clone(), plugin.clone(), shown.clone());
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
            };
            let widget = cx.new(|cx| {
                let content = Content::new(
                    host,
                    call,
                    "Example Clock",
                    FrameKind::Bar,
                    PluginProviders::new(),
                    false,
                    cx,
                );
                cx.observe(content.view(), move |_, _, _| shown.set(shown.get() + 1))
                    .detach();
                UserPluginWidget::new(content, "Clock".into(), cx)
            });
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
fn a_drawn_widget_shows_a_new_view_after_its_plugin_changed(cx: &mut TestAppContext) {
    let rig = rig(cx);
    let before = rig.shown.get();
    assert!(before > 0, "the first view is asked for at once");

    rig.change_records(false, cx);
    rig.settle(cx);

    assert!(rig.shown.get() > before);
}

#[gpui_kit::test]
fn a_widget_the_bar_does_not_draw_asks_its_plugin_for_nothing(cx: &mut TestAppContext) {
    let rig = rig(cx);
    rig.draw(false, cx);
    // The step that follows the last draw may still ask once; then the widget is quiet.
    rig.change_records(true, cx);
    rig.settle(cx);
    let hidden = rig.shown.get();

    for round in 0..20 {
        rig.change_records(round % 2 == 0, cx);
        rig.settle(cx);
    }

    assert_eq!(
        rig.shown.get(),
        hidden,
        "20 steps over 5 seconds asked for nothing"
    );
}

#[gpui_kit::test]
fn a_widget_asks_again_once_the_bar_draws_it_again(cx: &mut TestAppContext) {
    let rig = rig(cx);
    rig.draw(false, cx);
    rig.change_records(true, cx);
    rig.settle(cx);
    let hidden = rig.shown.get();

    rig.draw(true, cx);
    rig.change_records(false, cx);
    rig.settle(cx);

    assert!(rig.shown.get() > hidden);
}
