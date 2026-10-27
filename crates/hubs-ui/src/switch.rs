//! Moving the window from one environment to another, or opening one beside it.

use std::rc::Rc;

use gpui_kit::{
    App, ClickEvent, WeakEntity, Window,
    component::{
        WindowExt as _,
        button::{Button, ButtonVariants as _},
        dialog::DialogFooter,
    },
    prelude::*,
};
use nocterm_hubs_model::Environment;
use nocterm_workspace::Workspace;

use crate::ports::{EnvironmentOpener, Placement};

/// Opens environments for the window it belongs to, asking first when switching would end
/// tabs that cannot be taken back.
#[derive(Clone)]
pub(crate) struct Switch {
    opener: Rc<dyn EnvironmentOpener>,
    workspace: WeakEntity<Workspace>,
    current: Environment,
}

impl Switch {
    pub(crate) fn new(
        opener: Rc<dyn EnvironmentOpener>,
        workspace: WeakEntity<Workspace>,
        current: Environment,
    ) -> Self {
        Self {
            opener,
            workspace,
            current,
        }
    }

    pub(crate) fn current(&self) -> &Environment {
        &self.current
    }

    /// Opens `target`. Switching this window away from local tabs asks first: their sessions
    /// end with the window's contents, while the tabs of a hub window only detach.
    pub(crate) fn open(
        &self,
        target: Environment,
        placement: Placement,
        window: &mut Window,
        cx: &mut App,
    ) {
        if placement == Placement::ThisWindow && target == self.current {
            return;
        }
        let tabs = self.local_tabs(cx);
        if placement == Placement::ThisWindow && tabs > 0 {
            self.confirm(target, tabs, window, cx);
        } else {
            self.perform(&target, placement, window, cx);
        }
    }

    fn local_tabs(&self, cx: &App) -> usize {
        if !self.current.is_local() {
            return 0;
        }
        self.workspace
            .read_with(cx, |workspace, cx| workspace.items(cx).count())
            .unwrap_or(0)
    }

    fn perform(
        &self,
        target: &Environment,
        placement: Placement,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Err(error) = self.opener.open(target.clone(), placement, window, cx) {
            nocterm_ui::notice::error(
                window,
                cx,
                "environment-open",
                "Could not open the window",
                error.to_string(),
            );
        }
    }

    /// "N tabs are open in this window. Local sessions will end."
    fn confirm(&self, target: Environment, tabs: usize, window: &mut Window, cx: &mut App) {
        let switch = self.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let button = |id: &'static str, label: &'static str, placement, close: bool| {
                let (switch, target) = (switch.clone(), target.clone());
                Button::new(id).label(label).on_click(
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        window.close_dialog(cx);
                        if close {
                            switch.perform(&target, placement, window, cx);
                        }
                    },
                )
            };
            let new_window = {
                let (switch, target) = (switch.clone(), target.clone());
                Button::new("switch-new-window")
                    .primary()
                    .label("Open in a new window")
                    .on_click(move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        window.close_dialog(cx);
                        switch.perform(&target, Placement::NewWindow, window, cx);
                    })
            };
            let ok = {
                let (switch, target) = (switch.clone(), target.clone());
                move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                    switch.perform(&target, Placement::NewWindow, window, cx);
                    true
                }
            };
            alert
                .title("Switch this window?")
                .description(format!(
                    "{} open in this window. Their local sessions end if the window switches.",
                    if tabs == 1 {
                        "1 tab is".to_owned()
                    } else {
                        format!("{tabs} tabs are")
                    }
                ))
                // Enter takes the safe choice, which keeps the tabs.
                .on_ok(ok)
                .footer(
                    DialogFooter::new()
                        .child(button(
                            "switch-cancel",
                            "Cancel",
                            Placement::ThisWindow,
                            false,
                        ))
                        .child(button(
                            "switch-here",
                            "Switch and close tabs",
                            Placement::ThisWindow,
                            true,
                        ))
                        .child(new_window),
                )
        });
    }
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
