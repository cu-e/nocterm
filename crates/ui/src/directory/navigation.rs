//! The same stateless Home/Up/Refresh controls serve Explorer and folder forms.
use crate::IconName;
use gpui_kit::{
    App, IntoElement, RenderOnce, SharedString, Window,
    component::{
        Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
    },
    prelude::*,
};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub enum NavigationAction {
    Home,
    Up,
    Refresh,
}
#[derive(IntoElement)]
pub struct DirectoryNavigation {
    prefix: &'static str,
    enabled: bool,
    can_up: bool,
    on_action: Rc<dyn Fn(NavigationAction, &mut Window, &mut App)>,
}
impl DirectoryNavigation {
    pub fn new(
        prefix: &'static str,
        enabled: bool,
        can_up: bool,
        on_action: impl Fn(NavigationAction, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            prefix,
            enabled,
            can_up,
            on_action: Rc::new(on_action),
        }
    }
}
impl RenderOnce for DirectoryNavigation {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let home = self.on_action.clone();
        let up = self.on_action.clone();
        let refresh = self.on_action;
        h_flex()
            .gap_1()
            .child(
                Button::new(SharedString::from(format!("{}-home", self.prefix)))
                    .small()
                    .ghost()
                    .label("Home")
                    .disabled(!self.enabled)
                    .on_click(move |_, window, cx| home(NavigationAction::Home, window, cx)),
            )
            .child(
                Button::new(SharedString::from(format!("{}-parent", self.prefix)))
                    .small()
                    .ghost()
                    .label("Up")
                    .disabled(!self.enabled || !self.can_up)
                    .on_click(move |_, window, cx| up(NavigationAction::Up, window, cx)),
            )
            .child(
                Button::new(SharedString::from(format!("{}-refresh", self.prefix)))
                    .small()
                    .ghost()
                    .icon(IconName::RefreshCw)
                    .tooltip("Refresh directory")
                    .disabled(!self.enabled)
                    .on_click(move |_, window, cx| refresh(NavigationAction::Refresh, window, cx)),
            )
    }
}
