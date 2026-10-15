//! The panel's popover menus: new chat, model and option pickers, modes.
use gpui_kit::{
    Anchor, Context, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        popover::Popover,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ai::acp;
use nocterm_ui::IconName;

use super::{
    AgentPanel, MenuKind,
    widgets::{menu_row, menu_row_with, menu_variant},
};
use crate::runtime::Runtime;

impl AgentPanel {
    fn on_menu_open_changed(&mut self, kind: MenuKind, open: bool, cx: &mut Context<Self>) {
        if open {
            self.menus.open(kind);
        } else {
            self.menus.close_if_same(&kind);
        }
        cx.notify();
    }
    pub(super) fn menu_popover(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        kind: MenuKind,
        trigger: Button,
        anchor: Anchor,
        cx: &mut Context<Self>,
    ) -> Popover {
        let panel = cx.weak_entity();
        let content_panel = panel.clone();
        let open = self.menus.current() == Some(&kind);
        Popover::new(id)
            .anchor(anchor)
            .p_1()
            .bg(cx.theme().muted)
            .trigger(trigger)
            .trigger_style(gpui_kit::StyleRefinement::default().min_w_0().max_w_full())
            .open(open)
            .on_open_change(move |open, _, cx| {
                let _ = panel.update(cx, |this, cx| {
                    this.on_menu_open_changed(kind.clone(), *open, cx);
                });
            })
            .content(move |_, window, cx| {
                content_panel
                    .update(cx, |this, cx| this.render_menu(window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
    }
    fn render_menu(
        &mut self,
        window: &gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let kind = self.menus.current().cloned();
        let heading = self.menu_heading(kind.as_ref(), cx);
        let mut menu = ConfigMenuRenderer::frame(heading, cx);
        match kind {
            Some(MenuKind::Agents) => menu = menu.children(self.agent_choices(window, cx)),
            Some(MenuKind::Context) => menu = menu.child(self.render_attach_menu(cx)),
            Some(MenuKind::Config(id)) => {
                if let Some(thread) = self.current() {
                    let options = thread.read(cx).config_options();
                    if let Some(option) = options.iter().find(|option| option.id.0.as_ref() == id) {
                        let model =
                            option.category == Some(acp::SessionConfigOptionCategory::Model);
                        if model {
                            menu = menu.child(self.menus.query_input());
                        }
                        let query = if model {
                            self.menus.query(cx).to_lowercase()
                        } else {
                            String::new()
                        };
                        let groups = self.config_choices(&thread, option, &query, cx);
                        menu = menu.children(ConfigMenuRenderer::choices(groups, cx));
                    }
                }
            }
            Some(MenuKind::Modes) => menu = menu.children(self.mode_choices(cx)),
            Some(MenuKind::Usage) | None => {}
        }
        menu.test_support().into_any_element()
    }
    fn menu_heading(&self, kind: Option<&MenuKind>, cx: &gpui_kit::App) -> String {
        match kind {
            Some(MenuKind::Agents) => "New agent".into(),
            Some(MenuKind::Context) => "Servers and terminals".into(),
            Some(MenuKind::Modes) => "Agent mode".into(),
            Some(MenuKind::Usage) | None => String::new(),
            Some(MenuKind::Config(id)) => self
                .current()
                .and_then(|thread| {
                    thread
                        .read(cx)
                        .config_options()
                        .iter()
                        .find(|option| option.id.0.as_ref() == id)
                        .map(|option| option.name.clone())
                })
                .unwrap_or_default(),
        }
    }
    fn agent_choices(
        &self,
        window: &gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_kit::AnyElement> {
        let last = self.last_agent(cx);
        let settings = self.settings.setting::<nocterm_ai::AiSettings>(cx).clone();
        nocterm_ai::AgentRegistry::new(&settings)
            .iter()
            .map(|launch| {
                let id = launch.id.clone();
                let shortcut = (last.as_deref() == Some(id.as_str()))
                    .then(|| {
                        gpui_kit::component::kbd::Kbd::binding_for_action_in(
                            &crate::NewThreadWithLastAgent,
                            &self.focus,
                            window,
                        )
                    })
                    .flatten();
                menu_row_with(
                    SharedString::from(format!("new-agent-{id}")),
                    launch.name.clone(),
                    Some(nocterm_ui::agent_icon(&id).small().into_any_element()),
                    cx,
                )
                .children(shortcut)
                .on_click(
                    cx.listener(move |panel, _, window, cx| {
                        panel.new_thread(id.clone(), window, cx)
                    }),
                )
                .into_any_element()
            })
            .collect()
    }
    fn config_choices(
        &self,
        thread: &gpui_kit::Entity<crate::thread::AgentThread>,
        option: &acp::SessionConfigOption,
        query: &str,
        cx: &gpui_kit::App,
    ) -> Vec<(String, Vec<ConfigChoice>)> {
        let agent = thread.read(cx).document().agent_id().to_owned();
        let runtime = Runtime::global(cx);
        let favorites = runtime.read(cx).favorites(cx);
        let model = option.category == Some(acp::SessionConfigOptionCategory::Model);
        match &option.kind {
            acp::SessionConfigKind::Select(select) => {
                let groups = match &select.options {
                    acp::SessionConfigSelectOptions::Ungrouped(options) => {
                        vec![(String::new(), options.clone())]
                    }
                    acp::SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .map(|group| (group.name.clone(), group.options.clone()))
                        .collect(),
                    _ => Vec::new(),
                };
                groups
                    .into_iter()
                    .map(|(name, mut values)| {
                        values.sort_by_key(|value| {
                            !favorites.contains(
                                &agent,
                                option.id.0.as_ref(),
                                value.value.0.as_ref(),
                            )
                        });
                        let choices = values
                            .into_iter()
                            .filter(|value| value.name.to_lowercase().contains(query))
                            .map(|value| {
                                let favorite = favorites.contains(
                                    &agent,
                                    option.id.0.as_ref(),
                                    value.value.0.as_ref(),
                                );
                                ConfigChoice {
                                    id: format!("config-{}-{}", option.id, value.value),
                                    label: value.name,
                                    selected: value.value == select.current_value,
                                    action: ConfigurationSelection {
                                        thread: thread.clone(),
                                        config: option.id.clone(),
                                        value: acp::SessionConfigOptionValue::ValueId {
                                            value: value.value.clone(),
                                        },
                                    },
                                    favorite: model.then(|| FavoriteSelection {
                                        agent: agent.clone(),
                                        config: option.id.to_string(),
                                        value: value.value.to_string(),
                                        selected: favorite,
                                    }),
                                }
                            })
                            .collect();
                        (name, choices)
                    })
                    .collect()
            }
            acp::SessionConfigKind::Boolean(boolean) => vec![(
                String::new(),
                vec![ConfigChoice::boolean(
                    thread.clone(),
                    option.id.clone(),
                    !boolean.current_value,
                )],
            )],
            _ => Vec::new(),
        }
    }
    fn mode_choices(&self, cx: &mut Context<Self>) -> Vec<gpui_kit::AnyElement> {
        let Some(thread) = self.current() else {
            return Vec::new();
        };
        let Some(modes) = thread.read(cx).configuration().modes().cloned() else {
            return Vec::new();
        };
        let current = thread
            .read(cx)
            .configuration()
            .current_mode()
            .cloned()
            .unwrap_or(modes.current_mode_id);
        modes
            .available_modes
            .into_iter()
            .map(|mode| {
                let action = ModeSelection {
                    thread: thread.clone(),
                    mode: mode.id.clone(),
                };
                menu_row(
                    SharedString::from(format!("mode-{}", mode.id)),
                    mode.name,
                    cx,
                )
                .selected(current == mode.id)
                .on_click(
                    cx.listener(move |panel, _, _, cx| panel.on_mode_selected(action.clone(), cx)),
                )
                .into_any_element()
            })
            .collect()
    }
    fn on_configuration_selected(
        &mut self,
        action: ConfigurationSelection,
        cx: &mut Context<Self>,
    ) {
        if self
            .threads
            .iter()
            .any(|thread| thread.entity_id() == action.thread.entity_id())
        {
            action.thread.update(cx, |thread, cx| {
                thread.set_config(action.config, action.value, cx)
            });
        }
        self.menus.reset();
        cx.notify();
    }
    fn on_mode_selected(&mut self, action: ModeSelection, cx: &mut Context<Self>) {
        if self
            .threads
            .iter()
            .any(|thread| thread.entity_id() == action.thread.entity_id())
        {
            action
                .thread
                .update(cx, |thread, cx| thread.set_mode(action.mode, cx));
        }
        self.menus.reset();
        cx.notify();
    }
    fn on_favorite_selected(&mut self, action: FavoriteSelection, cx: &mut Context<Self>) {
        Runtime::global(cx).update(cx, |runtime, cx| {
            runtime.toggle_favorite(&action.agent, &action.config, &action.value, cx)
        });
    }
}

#[derive(Clone)]
struct ConfigurationSelection {
    thread: gpui_kit::Entity<crate::thread::AgentThread>,
    config: acp::SessionConfigId,
    value: acp::SessionConfigOptionValue,
}
#[derive(Clone)]
struct ModeSelection {
    thread: gpui_kit::Entity<crate::thread::AgentThread>,
    mode: acp::SessionModeId,
}
#[derive(Clone)]
struct FavoriteSelection {
    agent: String,
    config: String,
    value: String,
    selected: bool,
}
struct ConfigChoice {
    id: String,
    label: String,
    selected: bool,
    action: ConfigurationSelection,
    favorite: Option<FavoriteSelection>,
}
impl ConfigChoice {
    fn boolean(
        thread: gpui_kit::Entity<crate::thread::AgentThread>,
        config: acp::SessionConfigId,
        value: bool,
    ) -> Self {
        Self {
            id: "config-boolean".into(),
            label: if value { "Enable" } else { "Disable" }.into(),
            selected: false,
            action: ConfigurationSelection {
                thread,
                config,
                value: acp::SessionConfigOptionValue::Boolean { value },
            },
            favorite: None,
        }
    }
}
struct ConfigMenuRenderer;
impl ConfigMenuRenderer {
    fn frame(heading: String, cx: &gpui_kit::App) -> gpui_kit::Stateful<gpui_kit::Div> {
        v_flex()
            .id("agent-picker")
            .gap_1()
            .max_h(rems(18.))
            .overflow_y_scroll()
            .w(rems(18.))
            .child(
                div()
                    .id("agent-menu-heading")
                    .test_support()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(heading),
            )
    }
    fn choices(
        groups: Vec<(String, Vec<ConfigChoice>)>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<gpui_kit::AnyElement> {
        let mut rows = Vec::new();
        for (group, choices) in groups {
            if !group.is_empty() {
                rows.push(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(group)
                        .into_any_element(),
                );
            }
            for choice in choices {
                rows.push(Self::choice(choice, cx));
            }
        }
        rows
    }
    fn choice(choice: ConfigChoice, cx: &mut Context<AgentPanel>) -> gpui_kit::AnyElement {
        let action = choice.action;
        let row = h_flex().w_full().child(
            menu_row(SharedString::from(choice.id), choice.label, cx)
                .flex_1()
                .min_w_0()
                .selected(choice.selected)
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.on_configuration_selected(action.clone(), cx)
                })),
        );
        row.when_some(choice.favorite, |row, favorite| {
            row.child(
                Button::new(SharedString::from(format!(
                    "favorite-{}-{}",
                    favorite.config, favorite.value
                )))
                .custom(menu_variant(cx))
                .small()
                .icon(if favorite.selected {
                    IconName::StarFill
                } else {
                    IconName::Star
                })
                .tooltip("Favorite model")
                .on_click(cx.listener(move |panel, _, _, cx| {
                    panel.on_favorite_selected(favorite.clone(), cx)
                })),
            )
        })
        .into_any_element()
    }
}
