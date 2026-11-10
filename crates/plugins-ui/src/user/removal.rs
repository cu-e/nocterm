//! What a removal takes away, and the dialog that asks which.

use gpui_kit::{
    App, AppContext as _, Context, IntoElement, SharedString, TestSupportExt as _, WeakEntity,
    Window,
    component::{
        ActiveTheme as _, Sizable as _, WindowExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        radio::Radio,
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use nocterm_plugin_model::PluginId;

use super::section::UserPluginsSection;

/// The width of the dialog.
const WIDTH: f32 = 440.;

/// What a removal takes away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RemovalKind {
    /// The plugin; its data stays.
    Keep,
    /// The plugin and its data.
    WithData,
    /// The data; the plugin stays installed.
    DataOnly,
}

impl RemovalKind {
    const ALL: [Self; 3] = [Self::Keep, Self::WithData, Self::DataOnly];

    /// The label of the choice and of the button that confirms it.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Keep => "Remove",
            Self::WithData => "Remove with its data",
            Self::DataOnly => "Remove its data",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Keep => "user-removal-keep",
            Self::WithData => "user-removal-with-data",
            Self::DataOnly => "user-removal-data",
        }
    }

    /// What the choice does, under its label.
    fn detail(self) -> &'static str {
        match self {
            Self::Keep => "Uninstall it; its settings and stored data stay for a reinstall.",
            Self::WithData => "Uninstall it and delete its settings and stored data.",
            Self::DataOnly => "Delete its settings and stored data; it stays installed.",
        }
    }

    /// The question the dialog asks about the plugin named `name`.
    pub(super) fn question(self, name: &str) -> String {
        match self {
            Self::Keep => format!("Remove {name}? Its data is kept."),
            Self::WithData => format!("Remove {name} and delete its data? This cannot be undone."),
            Self::DataOnly => {
                format!("Delete the data of {name}? The plugin stays. This cannot be undone.")
            }
        }
    }
}

/// Asks what to remove of `plugin`, the least first; nothing goes until the user presses the
/// named button. Escape and Cancel close the dialog and remove nothing; Enter does nothing.
pub(super) fn ask(
    section: WeakEntity<UserPluginsSection>,
    plugin: PluginId,
    name: String,
    window: &mut Window,
    cx: &mut App,
) {
    let dialog = cx.new(|_| RemovalDialog {
        section,
        plugin,
        name: name.clone(),
        kind: RemovalKind::Keep,
    });
    window.open_dialog(cx, move |host, _, cx| {
        let confirm = dialog.clone();
        let kind = dialog.read(cx).kind;
        host.title(format!("Remove {name}"))
            .w(px(WIDTH))
            .child(dialog.clone())
            // Enter neither removes nor dismisses: only the named button removes.
            .on_ok(|_, _, _| false)
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("user-removal-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("user-removal-confirm")
                            .small()
                            .danger()
                            .label(kind.label())
                            .on_click(move |_, window, cx| {
                                confirm.update(cx, |dialog, cx| dialog.confirm(cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

/// The choice a removal dialog holds.
struct RemovalDialog {
    section: WeakEntity<UserPluginsSection>,
    plugin: PluginId,
    name: String,
    kind: RemovalKind,
}

impl RemovalDialog {
    fn choose(&mut self, kind: RemovalKind, cx: &mut Context<Self>) {
        self.kind = kind;
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let (plugin, kind) = (self.plugin.clone(), self.kind);
        let _ = self
            .section
            .update(cx, |section, cx| section.remove(&plugin, kind, cx));
    }
}

impl Render for RemovalDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let question = self.kind.question(&self.name);
        v_flex()
            .id("user-removal")
            .aria_label(SharedString::from(question))
            .test_support()
            .gap_2()
            .children(RemovalKind::ALL.into_iter().map(|kind| {
                Radio::new(kind.id())
                    .small()
                    .label(kind.label())
                    .checked(self.kind == kind)
                    .w_full()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(kind.detail()),
                    )
                    .on_change(cx.listener(move |this, _, _, cx| this.choose(kind, cx)))
            }))
    }
}
