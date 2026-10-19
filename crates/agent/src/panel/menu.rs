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
use nocterm_ai::{
    model::{
        controls::{ConfigCategory, ConfigKind, ConfigOption, SelectChoices},
        ids::{ConfigId, ModeId},
    },
    session_config::ConfigValue,
};
use nocterm_ui::IconName;

use super::{
    AgentPanel, MenuKind,
    widgets::{menu_row, menu_row_with, menu_variant},
};

#[derive(Clone)]
pub(super) struct ConfigurationSelection {
    thread: gpui_kit::Entity<crate::thread::AgentThread>,
    config: ConfigId,
    value: ConfigValue,
}
#[derive(Clone)]
pub(super) struct ModeSelection {
    thread: gpui_kit::Entity<crate::thread::AgentThread>,
    mode: ModeId,
}
#[derive(Clone)]
pub(super) struct FavoriteSelection {
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
        config: ConfigId,
        value: bool,
    ) -> Self {
        Self {
            id: "config-boolean".into(),
            label: if value { "Enable" } else { "Disable" }.into(),
            selected: false,
            action: ConfigurationSelection {
                thread,
                config,
                value: ConfigValue::Flag(value),
            },
            favorite: None,
        }
    }
}
pub(super) struct ConfigMenuRenderer;
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

impl ConfigMenuRenderer {
    pub(super) fn popover(
        menus: &super::menu_view::MenuView,
        id: impl Into<gpui_kit::ElementId>,
        kind: MenuKind,
        trigger: Button,
        anchor: Anchor,
        cx: &mut Context<AgentPanel>,
    ) -> Popover {
        let panel = cx.weak_entity();
        let content_panel = panel.clone();
        let open = menus.current() == Some(&kind);
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
}

impl ConfigMenuRenderer {
    pub(super) fn render_menu(
        mut frame: MenuFrame,
        window: &gpui_kit::Window,
        cx: &mut Context<AgentPanel>,
    ) -> gpui_kit::AnyElement {
        let kind = frame.kind.clone();
        let heading = Self::menu_heading(kind.as_ref(), frame.thread.as_ref(), cx);
        let mut menu = ConfigMenuRenderer::frame(heading, cx);
        match kind {
            Some(MenuKind::Agents) => {
                menu = menu.children(Self::agent_choices(
                    &frame.settings,
                    frame.last.as_deref(),
                    &frame.focus,
                    window,
                    cx,
                ))
            }
            Some(MenuKind::Context) => {
                menu = menu.child(
                    frame
                        .attachment
                        .take()
                        .unwrap_or_else(|| div().into_any_element()),
                )
            }
            Some(MenuKind::Config(id)) => {
                if let Some(thread) = frame.thread.clone() {
                    let options = thread.read(cx).config_options();
                    if let Some(option) = options.iter().find(|option| option.id.as_str() == id) {
                        let model = option.category == Some(ConfigCategory::Model);
                        if model {
                            menu = menu.child(frame.query_input);
                        }
                        let query = if model { frame.query } else { String::new() };
                        let groups =
                            Self::config_choices(&thread, option, &query, &frame.favorites, cx);
                        menu = menu.children(ConfigMenuRenderer::choices(groups, cx));
                    }
                }
            }
            Some(MenuKind::Modes) => {
                menu = menu.children(Self::mode_choices(frame.thread.as_ref(), cx))
            }
            Some(MenuKind::Usage) | None => {}
        }
        menu.test_support().into_any_element()
    }
}

impl ConfigMenuRenderer {
    fn menu_heading(
        kind: Option<&MenuKind>,
        thread: Option<&gpui_kit::Entity<crate::thread::AgentThread>>,
        cx: &gpui_kit::App,
    ) -> String {
        match kind {
            Some(MenuKind::Agents) => "New agent".into(),
            Some(MenuKind::Context) => "Servers and terminals".into(),
            Some(MenuKind::Modes) => "Agent mode".into(),
            Some(MenuKind::Usage) | None => String::new(),
            Some(MenuKind::Config(id)) => thread
                .and_then(|thread| {
                    thread
                        .read(cx)
                        .config_options()
                        .iter()
                        .find(|option| option.id.as_str() == id)
                        .map(|option| option.name.clone())
                })
                .unwrap_or_default(),
        }
    }
}

impl ConfigMenuRenderer {
    fn agent_choices(
        settings: &nocterm_ai::AiSettings,
        last: Option<&str>,
        focus: &gpui_kit::FocusHandle,
        window: &gpui_kit::Window,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<gpui_kit::AnyElement> {
        nocterm_ai::AgentRegistry::new(settings)
            .iter()
            .map(|launch| {
                let id = launch.id.clone();
                let shortcut = (last == Some(id.as_str()))
                    .then(|| {
                        gpui_kit::component::kbd::Kbd::binding_for_action_in(
                            &crate::NewThreadWithLastAgent,
                            focus,
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
}

impl ConfigMenuRenderer {
    fn config_choices(
        thread: &gpui_kit::Entity<crate::thread::AgentThread>,
        option: &ConfigOption,
        query: &str,
        favorites: &nocterm_ai::favorites::AgentStateFile,
        cx: &gpui_kit::App,
    ) -> Vec<(String, Vec<ConfigChoice>)> {
        let Some(agent) = thread
            .read(cx)
            .document()
            .agent_id()
            .map(|id| id.to_string())
        else {
            return Vec::new();
        };
        let model = option.category == Some(ConfigCategory::Model);
        match &option.kind {
            ConfigKind::Select { current, choices } => {
                let groups = match choices {
                    SelectChoices::Flat(options) => {
                        vec![(String::new(), options.clone())]
                    }
                    SelectChoices::Groups(groups) => groups
                        .iter()
                        .map(|group| (group.name.clone(), group.choices.clone()))
                        .collect(),
                };
                groups
                    .into_iter()
                    .map(|(name, mut values)| {
                        values.sort_by_key(|value| {
                            !favorites.contains(&agent, option.id.as_str(), value.id.as_str())
                        });
                        let choices = values
                            .into_iter()
                            .filter(|value| value.name.to_lowercase().contains(query))
                            .map(|value| {
                                let favorite = favorites.contains(
                                    &agent,
                                    option.id.as_str(),
                                    value.id.as_str(),
                                );
                                ConfigChoice {
                                    id: format!("config-{}-{}", option.id, value.id),
                                    label: value.name,
                                    selected: value.id == *current,
                                    action: ConfigurationSelection {
                                        thread: thread.clone(),
                                        config: option.id.clone(),
                                        value: ConfigValue::Value(value.id.to_string()),
                                    },
                                    favorite: model.then(|| FavoriteSelection {
                                        agent: agent.clone(),
                                        config: option.id.to_string(),
                                        value: value.id.to_string(),
                                        selected: favorite,
                                    }),
                                }
                            })
                            .collect();
                        (name, choices)
                    })
                    .collect()
            }
            ConfigKind::Boolean(boolean) => vec![(
                String::new(),
                vec![ConfigChoice::boolean(
                    thread.clone(),
                    option.id.clone(),
                    !*boolean,
                )],
            )],
            _ => Vec::new(),
        }
    }
}

impl ConfigMenuRenderer {
    fn mode_choices(
        thread: Option<&gpui_kit::Entity<crate::thread::AgentThread>>,
        cx: &mut Context<AgentPanel>,
    ) -> Vec<gpui_kit::AnyElement> {
        let Some(thread) = thread else {
            return Vec::new();
        };
        let Some(modes) = thread.read(cx).configuration().modes() else {
            return Vec::new();
        };
        let current = thread
            .read(cx)
            .configuration()
            .current_mode()
            .unwrap_or(modes.current);
        modes
            .choices
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
}

pub(super) struct MenuFrame {
    kind: Option<MenuKind>,
    thread: Option<gpui_kit::Entity<crate::thread::AgentThread>>,
    query_input: gpui_kit::component::input::Input,
    query: String,
    settings: nocterm_ai::AiSettings,
    last: Option<String>,
    focus: gpui_kit::FocusHandle,
    favorites: nocterm_ai::favorites::AgentStateFile,
    attachment: Option<gpui_kit::AnyElement>,
}

impl ConfigurationSelection {
    pub(super) fn into_parts(
        self,
    ) -> (
        gpui_kit::Entity<crate::thread::AgentThread>,
        ConfigId,
        ConfigValue,
    ) {
        (self.thread, self.config, self.value)
    }
}

impl ModeSelection {
    pub(super) fn into_parts(self) -> (gpui_kit::Entity<crate::thread::AgentThread>, ModeId) {
        (self.thread, self.mode)
    }
}

impl FavoriteSelection {
    pub(super) fn into_parts(self) -> (String, String, String, bool) {
        (self.agent, self.config, self.value, self.selected)
    }
}

impl MenuFrame {
    pub(super) fn new(
        kind: Option<MenuKind>,
        thread: Option<gpui_kit::Entity<crate::thread::AgentThread>>,
        query_input: gpui_kit::component::input::Input,
        query: String,
        settings: nocterm_ai::AiSettings,
        last: Option<String>,
        focus: gpui_kit::FocusHandle,
        favorites: nocterm_ai::favorites::AgentStateFile,
        attachment: Option<gpui_kit::AnyElement>,
    ) -> Self {
        Self {
            kind,
            thread,
            query_input,
            query,
            settings,
            last,
            focus,
            favorites,
            attachment,
        }
    }
}
