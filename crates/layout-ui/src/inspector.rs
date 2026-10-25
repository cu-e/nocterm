//! The selected item: where it is, how it opens, the plugin it belongs to and its settings.

use std::{rc::Rc, sync::Arc};

use gpui_kit::{
    AnyElement, App, Entity, IntoElement, RenderOnce, SharedString, TestSupportExt as _, Window,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        switch::Switch,
        v_flex,
    },
    div,
    prelude::*,
};
use nocterm_plugin_model::{
    ContributionId, ContributionKind, ContributionRef, GroupId, PluginSet, SetOwner, Side, Zone,
};
use nocterm_ui::{PluginSettingsForm, form};
use nocterm_window_layout::{EditOp, Subject};
use nocterm_workspace::OpenPlugins;

use crate::{commands::Commands, parts::action, snapshot::Snapshot};

/// The narrowest the inspector is drawn. Next to the lists it takes what they leave; when the
/// two do not fit side by side it goes below them, across the width.
pub(crate) const INSPECTOR_MIN_REMS: f32 = 16.;

#[derive(IntoElement)]
pub(crate) struct Inspector {
    snapshot: Rc<Snapshot>,
    catalog: Arc<PluginSet>,
    selected: Option<Subject>,
    form: Option<Entity<PluginSettingsForm>>,
    commands: Commands,
}

impl Inspector {
    pub(crate) fn new(
        snapshot: Rc<Snapshot>,
        catalog: Arc<PluginSet>,
        selected: Option<Subject>,
        form: Option<Entity<PluginSettingsForm>>,
        commands: Commands,
    ) -> Self {
        Self {
            snapshot,
            catalog,
            selected,
            form,
            commands,
        }
    }

    fn contribution(&self, id: &ContributionId, cx: &App) -> AnyElement {
        let Some(contribution) = self.catalog.contribution(id) else {
            return form::note("This item is not known to this application.", cx);
        };
        let kind = match contribution.kind() {
            ContributionKind::Widget => "Status bar widget",
            ContributionKind::Panel => "Panel",
        };
        v_flex()
            .id(SharedString::from(format!("layout-inspector:{id}")))
            .aria_label(contribution.title())
            .test_support()
            .gap_3()
            .child(heading(contribution.title(), kind, cx))
            .child(self.owner(id, cx))
            .when(self.snapshot.editable(), |column| {
                column.child(self.controls(contribution, cx))
            })
            .child(self.settings(cx))
            .into_any_element()
    }

    /// "Built in", or the plugin with the way to manage it.
    fn owner(&self, id: &ContributionId, cx: &App) -> AnyElement {
        let Some(owner) = self.catalog.owner(id) else {
            return div().into_any_element();
        };
        match owner {
            SetOwner::Core => line("layout-owner", "Built in", cx),
            SetOwner::Plugin(plugin) => {
                let disabled = self.snapshot.disabled().contains(plugin.id());
                let target: SharedString = plugin.id().as_str().to_owned().into();
                let (label, button) = if disabled {
                    ("Disabled", "layout-enable-plugin")
                } else {
                    ("", "layout-manage-plugin")
                };
                let action_label = if disabled {
                    "Enable in Plugins\u{2026}"
                } else {
                    "Manage plugin\u{2026}"
                };
                let text = if disabled {
                    format!("{label} \u{b7} Part of {}", plugin.name())
                } else {
                    format!("Part of {}", plugin.name())
                };
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(line("layout-owner", text, cx))
                    .child(
                        Button::new(button)
                            .small()
                            .ghost()
                            .label(action_label)
                            .on_click(move |_, window, cx| {
                                let action = OpenPlugins {
                                    plugin: Some(target.clone()),
                                };
                                window.dispatch_action(Box::new(action), cx);
                            }),
                    )
                    .into_any_element()
            }
        }
    }

    fn controls(&self, contribution: ContributionRef<'_>, cx: &App) -> AnyElement {
        let id = contribution.id().clone();
        let subject = Subject::Contribution(id.clone());
        let placed = self.snapshot.layout().is_placed(&id);
        let mut column = v_flex().id("layout-controls").test_support().gap_2();
        if !placed {
            let (commands, restore) = (self.commands.clone(), id.clone());
            return column
                .child(action(
                    "layout-op-add",
                    "Add to the bar",
                    true,
                    move |window, cx| {
                        let id = restore.clone();
                        commands.run(window, cx, move |section, _, cx| {
                            section.edit(EditOp::Restore { id }, cx);
                        });
                    },
                ))
                .into_any_element();
        }
        column = column.child(self.arrange(&subject));
        match contribution.kind() {
            ContributionKind::Widget => {
                column = column.child(self.zones(subject.clone()));
            }
            ContributionKind::Panel => {
                column = column.child(self.panel(&id, cx));
            }
        }
        column
            .child(self.separator_and_remove(&subject, contribution))
            .into_any_element()
    }

    /// Earlier and later in the zone or group.
    fn arrange(&self, subject: &Subject) -> AnyElement {
        let step = |id: &'static str, label: &'static str, by: i32| {
            let (commands, subject) = (self.commands.clone(), subject.clone());
            action(id, label, true, move |window, cx| {
                let subject = subject.clone();
                commands.run(window, cx, move |section, _, cx| {
                    section.edit(EditOp::Move { subject, by }, cx);
                });
            })
        };
        h_flex()
            .flex_wrap()
            .gap_2()
            .child(step("layout-op-earlier", "Move earlier", -1))
            .child(step("layout-op-later", "Move later", 1))
            .into_any_element()
    }

    /// The three zones as buttons; the current one is marked.
    fn zones(&self, subject: Subject) -> AnyElement {
        let current = self.snapshot.zone_of(&subject);
        let buttons = [
            (Zone::Left, "layout-op-zone-left", "Left"),
            (Zone::Center, "layout-op-zone-center", "Centre"),
            (Zone::Right, "layout-op-zone-right", "Right"),
        ]
        .map(|(zone, id, label)| {
            let (commands, subject) = (self.commands.clone(), subject.clone());
            Button::new(id)
                .small()
                .ghost()
                .label(label)
                .selected(current == Some(zone))
                .on_click(move |_, window, cx| {
                    let subject = subject.clone();
                    commands.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::ToZone { subject, zone }, cx);
                    });
                })
        });
        v_flex()
            .gap_1()
            .child(label_text("Zone"))
            .child(h_flex().flex_wrap().gap_1().children(buttons))
            .into_any_element()
    }

    /// The group of a panel and whether it opens with the layout.
    fn panel(&self, id: &ContributionId, cx: &App) -> AnyElement {
        let current = self.snapshot.group_of(id).map(|group| group.id().clone());
        let groups = self.snapshot.groups().iter().map(|spec| {
            let (commands, panel, group) = (self.commands.clone(), id.clone(), spec.id.clone());
            Button::new(SharedString::from(format!("layout-op-group:{}", spec.id)))
                .small()
                .ghost()
                .label(spec.title)
                .selected(current.as_ref() == Some(&spec.id))
                .on_click(move |_, window, cx| {
                    let (panel, group) = (panel.clone(), group.clone());
                    commands.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::ToGroup { panel, group }, cx);
                    });
                })
        });
        let open = self.snapshot.layout().open().contains(id);
        let (commands, panel) = (self.commands.clone(), id.clone());
        let switch = Switch::new("layout-open-by-default")
            .checked(open)
            .on_click(move |checked: &bool, window, cx| {
                let (panel, open) = (panel.clone(), *checked);
                commands.run(window, cx, move |section, _, cx| {
                    section.edit(EditOp::SetOpen { panel, open }, cx);
                });
            });
        v_flex()
            .gap_2()
            .child(
                v_flex()
                    .gap_1()
                    .child(label_text("Group"))
                    .child(h_flex().flex_wrap().gap_1().children(groups)),
            )
            .child(form::row(
                "Open by default",
                "The panel is open when this layout is chosen.",
                switch,
                cx,
            ))
            .into_any_element()
    }

    fn separator_and_remove(
        &self,
        subject: &Subject,
        contribution: ContributionRef<'_>,
    ) -> AnyElement {
        let after = self.snapshot.after(subject);
        let (insert, remove) = (self.commands.clone(), self.commands.clone());
        let id = contribution.id().clone();
        h_flex()
            .flex_wrap()
            .gap_2()
            .child(action(
                "layout-op-separator",
                "Add separator after",
                after.is_some(),
                move |window, cx| {
                    let Some((place, index)) = after.clone() else {
                        return;
                    };
                    insert.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::InsertSeparator { place, index }, cx);
                    });
                },
            ))
            .child(action(
                "layout-op-remove",
                if contribution.pinned() {
                    "Always shown"
                } else {
                    "Remove from the bar"
                },
                !contribution.pinned(),
                move |window, cx| {
                    let id = id.clone();
                    remove.run(window, cx, move |section, _, cx| {
                        section.edit(EditOp::RemoveToTray { id }, cx);
                    });
                },
            ))
            .into_any_element()
    }

    fn group(&self, id: &GroupId, cx: &App) -> AnyElement {
        let Some(spec) = self.snapshot.groups().iter().find(|spec| spec.id == *id) else {
            return form::note("This group is not known to this application.", cx);
        };
        let subject = Subject::Group(id.clone());
        let zone = self.snapshot.zone_of_group(id);
        let side = self
            .snapshot
            .layout()
            .groups()
            .find(|group| group.id() == id)
            .map(|group| group.side());
        v_flex()
            .id(SharedString::from(format!("layout-inspector:group:{id}")))
            .aria_label(spec.title)
            .test_support()
            .gap_3()
            .child(heading(spec.title, "Panel group", cx))
            .when(self.snapshot.editable() && zone.is_some(), |column| {
                column
                    .child(self.arrange(&subject))
                    .child(self.zones(subject.clone()))
                    .child(self.side(id, zone, side, cx))
            })
            .into_any_element()
    }

    /// Which side the group's panels open on: derived from the zone, with an override in the
    /// centre.
    fn side(&self, id: &GroupId, zone: Option<Zone>, side: Option<Side>, cx: &App) -> AnyElement {
        let name = match side {
            Some(Side::Left) => "left",
            Some(Side::Right) => "right",
            None => "left",
        };
        if zone != Some(Zone::Center) {
            return line("layout-side", format!("Opens: {name} (from zone)"), cx);
        }
        let overridden = self.snapshot.side_override(id);
        let choices = [
            ("layout-op-side-left", "Left", Some(Side::Left)),
            ("layout-op-side-right", "Right", Some(Side::Right)),
            ("layout-op-side-zone", "From zone", None),
        ]
        .map(|(button, label, wanted)| {
            let (commands, group) = (self.commands.clone(), id.clone());
            Button::new(button)
                .small()
                .ghost()
                .label(label)
                .selected(wanted == overridden)
                .on_click(move |_, window, cx| {
                    let group = group.clone();
                    commands.run(window, cx, move |section, _, cx| {
                        section.edit(
                            EditOp::SetSide {
                                group,
                                side: wanted,
                            },
                            cx,
                        );
                    });
                })
        });
        v_flex()
            .gap_1()
            .child(line("layout-side", format!("Opens: {name}"), cx))
            .child(h_flex().flex_wrap().gap_1().children(choices))
            .into_any_element()
    }

    fn settings(&self, cx: &App) -> AnyElement {
        match &self.form {
            Some(form) => form.clone().into_any_element(),
            None => div()
                .id("layout-no-settings")
                .aria_label("This item has no settings")
                .test_support()
                .child(form::note("This item has no settings", cx))
                .into_any_element(),
        }
    }
}

fn heading(title: &str, kind: &'static str, cx: &App) -> AnyElement {
    v_flex()
        .child(div().text_base().font_semibold().child(title.to_owned()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(kind),
        )
        .into_any_element()
}

fn label_text(text: &'static str) -> AnyElement {
    div()
        .text_xs()
        .font_semibold()
        .child(text)
        .into_any_element()
}

fn line(id: &'static str, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .aria_label(text.clone())
        .test_support()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .into_any_element()
}

impl RenderOnce for Inspector {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let body = match self.selected.clone() {
            None => form::note("Select an item in the bar to see its settings.", cx),
            Some(Subject::Contribution(id)) => self.contribution(&id, cx),
            Some(Subject::Group(id)) => self.group(&id, cx),
        };
        v_flex()
            .id("layout-inspector")
            .test_support()
            .flex_1()
            .min_w(gpui_kit::rems(INSPECTOR_MIN_REMS))
            .gap_3()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(body)
    }
}
