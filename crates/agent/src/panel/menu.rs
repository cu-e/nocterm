//! The panel's popover menus: new chat, model and option pickers, modes.
use gpui_kit::{
    Anchor, Context, SharedString, TestSupportExt as _,
    component::{
        ActiveTheme as _, Selectable as _, Sizable as _, StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        popover::Popover,
        v_flex,
    },
    div,
    prelude::*,
    rems,
};
use nocterm_ai::acp;
use nocterm_ui::{IconName, SettingsExt as _};

use super::{
    AgentPanel, MenuKind,
    widgets::{menu_row, menu_row_with, menu_variant},
};
use crate::runtime::Runtime;

impl AgentPanel {
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
        let open = self.menu.as_ref() == Some(&kind);
        Popover::new(id)
            .anchor(anchor)
            .p_1()
            .bg(cx.theme().muted)
            .trigger(trigger)
            .trigger_style(gpui_kit::StyleRefinement::default().min_w_0().max_w_full())
            .open(open)
            .on_open_change(move |open, _, cx| {
                let _ = panel.update(cx, |this, cx| {
                    if *open {
                        this.menu = Some(kind.clone());
                    } else if this.menu.as_ref() == Some(&kind) {
                        this.menu = None;
                    }
                    cx.notify();
                });
            })
            .content(move |_, window, cx| {
                content_panel
                    .update(cx, |this, cx| this.render_menu(window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            })
    }
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    fn render_menu(
        &mut self,
        window: &gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let mut menu = v_flex()
            .id("agent-picker")
            .test_support()
            .gap_1()
            .max_h(rems(18.))
            .overflow_y_scroll()
            .w(rems(18.));
        let heading = match self.menu.as_ref() {
            Some(MenuKind::Agents) => "New agent".into(),
            Some(MenuKind::Context) => "Servers and terminals".into(),
            Some(MenuKind::Modes) => "Agent mode".into(),
            Some(MenuKind::Usage) | None => String::new(),
            Some(MenuKind::Config(id)) => self
                .current()
                .and_then(|thread| {
                    thread
                        .read(cx)
                        .state
                        .config_options
                        .iter()
                        .find(|option| option.id.0.as_ref() == id)
                        .map(|option| option.name.clone())
                })
                .unwrap_or_default(),
        };
        menu = menu.child(
            div()
                .id("agent-menu-heading")
                .test_support()
                .px_2()
                .py_1()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(heading),
        );
        match self.menu.clone() {
            Some(MenuKind::Agents) => {
                let last = self.last_agent(cx);
                for launch in
                    nocterm_ai::AgentRegistry::new(cx.setting::<nocterm_ai::AiSettings>()).iter()
                {
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
                    menu = menu.child(
                        menu_row_with(
                            SharedString::from(format!("new-agent-{id}")),
                            launch.name.clone(),
                            Some(nocterm_ui::agent_icon(&id).small().into_any_element()),
                            cx,
                        )
                        .children(shortcut)
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.new_thread(id.clone(), window, cx),
                        )),
                    );
                }
            }
            Some(MenuKind::Context) => {
                menu = menu.child(self.render_attach_menu(cx));
            }
            Some(MenuKind::Config(id)) => {
                if let Some(thread) = self.current() {
                    let options = thread.read(cx).state.config_options.clone();
                    if let Some(option) = options.iter().find(|option| option.id.0.as_ref() == id) {
                        if option.category == Some(acp::SessionConfigOptionCategory::Model) {
                            menu = menu.child(Input::new(&self.search).small());
                        }
                        let query =
                            if option.category == Some(acp::SessionConfigOptionCategory::Model) {
                                self.search.read(cx).value().to_lowercase()
                            } else {
                                String::new()
                            };
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
                                for (group, mut values) in groups {
                                    if !group.is_empty() {
                                        menu = menu.child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(group),
                                        );
                                    }
                                    values.sort_by_key(|value| {
                                        !Runtime::global(cx).read(cx).favorites.contains(
                                            &thread.read(cx).agent_id,
                                            &id,
                                            value.value.0.as_ref(),
                                        )
                                    });
                                    for value in values
                                        .into_iter()
                                        .filter(|value| value.name.to_lowercase().contains(&query))
                                    {
                                        let favorite =
                                            Runtime::global(cx).read(cx).favorites.contains(
                                                &thread.read(cx).agent_id,
                                                &id,
                                                value.value.0.as_ref(),
                                            );
                                        let value_id = value.value.clone();
                                        let config = option.id.clone();
                                        let star_value = value.value.to_string();
                                        let agent = thread.read(cx).agent_id.clone();
                                        let star_id = id.clone();
                                        menu=menu.child(h_flex().w_full().child(menu_row(SharedString::from(format!("config-{id}-{value_id}")), value.name, cx).flex_1().min_w_0().selected(value.value==select.current_value).on_click(cx.listener(move|this,_,_,cx|{if let Some(thread)=this.current(){thread.update(cx,|thread,cx|thread.set_config(config.clone(),acp::SessionConfigOptionValue::ValueId{value:value_id.clone()},cx));}this.menu=None;cx.notify();})))
                                .when(option.category==Some(acp::SessionConfigOptionCategory::Model),|row|row.child(Button::new(SharedString::from(format!("favorite-{star_id}-{star_value}"))).custom(menu_variant(cx)).small().icon(if favorite{IconName::StarFill}else{IconName::Star}).tooltip("Favorite model").on_click(move|_,_,cx|Runtime::global(cx).update(cx,|runtime,cx|runtime.toggle_favorite(&agent,&star_id,&star_value,cx))))));
                                    }
                                }
                            }
                            acp::SessionConfigKind::Boolean(boolean) => {
                                let config = option.id.clone();
                                let value = !boolean.current_value;
                                menu = menu.child(
                                    menu_row(
                                        "config-boolean",
                                        if value { "Enable" } else { "Disable" },
                                        cx,
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            if let Some(thread) = this.current() {
                                                thread.update(cx, |thread, cx| {
                                                    thread.set_config(
                                                        config.clone(),
                                                        acp::SessionConfigOptionValue::Boolean {
                                                            value,
                                                        },
                                                        cx,
                                                    )
                                                });
                                            }
                                            this.menu = None;
                                            cx.notify();
                                        },
                                    )),
                                );
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some(MenuKind::Modes) => {
                if let Some(thread) = self.current()
                    && let Some(modes) = &thread.read(cx).state.modes
                {
                    for mode in &modes.available_modes {
                        let id = mode.id.clone();
                        menu = menu.child(
                            menu_row(
                                SharedString::from(format!("mode-{id}")),
                                mode.name.clone(),
                                cx,
                            )
                            .selected(
                                thread
                                    .read(cx)
                                    .state
                                    .current_mode
                                    .as_ref()
                                    .unwrap_or(&modes.current_mode_id)
                                    == &id,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(thread) = this.current() {
                                        thread.update(cx, |thread, cx| {
                                            thread.set_mode(id.clone(), cx)
                                        });
                                    }
                                    this.menu = None;
                                    cx.notify();
                                },
                            )),
                        );
                    }
                }
            }
            Some(MenuKind::Usage) | None => {}
        }
        menu.into_any_element()
    }
}
