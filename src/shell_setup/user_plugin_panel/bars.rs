//! The bars of a window's plugin terminal tabs (`terminals.bar`): the view the contribution a
//! tab reports to draws for that terminal, in the host's frame, refreshed while the tab is open.
use gpui_kit::{
    AnyView, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, WeakEntity, Window, div,
};
use nocterm_plugin_api::{ContributionContext, Kind, Zone};
use nocterm_plugin_host::{Call, PluginHost};
use nocterm_plugin_ports::PluginProviders;
use nocterm_plugin_view::FrameKind;

use super::{
    content::{Content, FrameSpec},
    driver::RefreshDriver,
};

/// What a window draws the bars of its plugin terminals with: the window's refresh driver,
/// which the window keeps, and whether the frames are a developer session's.
#[derive(Clone)]
pub(in crate::shell_setup) struct TerminalBars {
    driver: WeakEntity<RefreshDriver>,
    dev: bool,
}

impl TerminalBars {
    pub(in crate::shell_setup) fn new(driver: WeakEntity<RefreshDriver>, dev: bool) -> Self {
        Self { driver, dev }
    }

    /// The bar of `terminal`, which the contribution of `call` draws: a strip across the tab
    /// (`along` is `Center`), or a rail along its left or right side. Its calls name the terminal
    /// and are drawn as a toolbar's, with `along` as the zone, so the plugin knows its axis.
    /// `None` once the window is gone.
    #[expect(
        clippy::too_many_arguments,
        reason = "one bar's parts, each from its own owner"
    )]
    pub(in crate::shell_setup) fn view(
        &self,
        host: Entity<PluginHost>,
        call: &Call,
        terminal: &str,
        name: &str,
        along: Zone,
        providers: PluginProviders,
        cx: &mut App,
    ) -> Option<AnyView> {
        let driver = self.driver.upgrade()?;
        let call = Call {
            context: ContributionContext {
                kind: Kind::Toolbar,
                zone: along,
            },
            in_focus: false,
            terminal: Some(terminal.to_owned()),
            ..call.clone()
        };
        let frame = FrameSpec {
            name,
            kind: if along == Zone::Center {
                FrameKind::Strip
            } else {
                FrameKind::Rail
            },
            dev: self.dev,
        };
        let content = Content::new(host, driver, call, frame, providers, cx);
        content.update(cx, |content, cx| content.visible(true, false, cx));
        Some(cx.new(|_| UserPluginBar::new(content)).into())
    }
}

/// A plugin's toolbar: in one of its terminal tabs, or along the window's tab area.
pub(super) struct UserPluginBar {
    content: Entity<Content>,
}

impl UserPluginBar {
    pub(super) fn new(content: Entity<Content>) -> Self {
        Self { content }
    }
}

impl Render for UserPluginBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.content.update(cx, |content, cx| content.restyle(cx));
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.content.read(cx).view().clone())
    }
}
