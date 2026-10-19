//! The history split retains its shared layout and reversible widening amount.
use super::AgentPanel;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, Pixels, Subscription, TestSupportExt as _,
    Window,
    component::{
        ActiveTheme as _, ResizablePanelEvent, ResizableState, h_resizable, resizable_panel,
    },
    div,
    prelude::*,
    px,
};
use nocterm_store::StoreError;
use nocterm_ui::LayoutHandle;
use std::sync::Arc;

const HISTORY_WIDTH: &str = "agent.history";
const DEFAULT_WIDTH: Pixels = px(280.);

#[derive(Default)]
enum SplitRefresh {
    #[default]
    Idle,
    Requested(Arc<()>),
    Settling {
        container: Pixels,
        ticket: Arc<()>,
    },
    Confirming {
        container: Pixels,
        ticket: Arc<()>,
    },
}
impl SplitRefresh {
    fn observe_container(&mut self, container: Pixels) -> Option<(bool, Arc<()>)> {
        match self {
            Self::Idle => None,
            Self::Confirming {
                container: previous,
                ..
            } if *previous == container => {
                *self = Self::Idle;
                None
            }
            Self::Settling {
                container: previous,
                ticket,
            } if *previous == container => {
                let ticket = ticket.clone();
                *self = Self::Confirming {
                    container,
                    ticket: ticket.clone(),
                };
                Some((false, ticket))
            }
            Self::Requested(ticket)
            | Self::Settling { ticket, .. }
            | Self::Confirming { ticket, .. } => {
                let ticket = ticket.clone();
                *self = Self::Settling {
                    container,
                    ticket: ticket.clone(),
                };
                Some((true, ticket))
            }
        }
    }
    fn current(&self, ticket: &Arc<()>) -> bool {
        match self {
            Self::Settling {
                ticket: current, ..
            }
            | Self::Confirming {
                ticket: current, ..
            } => Arc::ptr_eq(current, ticket),
            Self::Idle | Self::Requested(_) => false,
        }
    }
}

pub(super) struct PanelLayout {
    history: bool,
    widened: Option<Pixels>,
    splits: [Entity<ResizableState>; 2],
    docked_left: bool,
    maximized: bool,
    layout: LayoutHandle,
    _bindings: Vec<Subscription>,
    refresh: [SplitRefresh; 2],
}
impl PanelLayout {
    pub(super) fn new(layout: LayoutHandle, cx: &mut Context<AgentPanel>) -> Self {
        let mut bindings = Vec::new();
        let splits = [1, 0].map(|index| {
            let split = cx.new(|_| ResizableState::default());
            bindings.push(cx.subscribe(
                &split,
                move |panel, split, _: &ResizablePanelEvent, cx| {
                    let size = split.read(cx).sizes().get(index).copied();
                    if let Some(size) = size {
                        panel.on_history_split_resized(index, size, cx);
                    }
                },
            ));
            split
        });
        let panel = cx.weak_entity();
        bindings.push(layout.observe_size(HISTORY_WIDTH, cx, move |value, cx| {
            let _ = panel.update(cx, |panel, cx| {
                panel.on_shared_history_width_changed(value, cx)
            });
        }));
        Self {
            history: true,
            widened: None,
            splits,
            docked_left: false,
            maximized: false,
            layout,
            _bindings: bindings,
            refresh: std::array::from_fn(|_| SplitRefresh::Idle),
        }
    }
    pub(super) fn history(&self) -> bool {
        self.history
    }
    pub(super) fn maximized(&self) -> bool {
        self.maximized
    }
    pub(super) fn set_maximized(&mut self, value: bool) {
        self.maximized = value;
    }
    pub(super) fn set_docked_left(&mut self, value: bool) {
        if self.docked_left != value {
            self.docked_left = value;
            self.refresh[value as usize] = SplitRefresh::Requested(Arc::new(()));
        }
    }
    pub(super) fn history_width(&self, cx: &App) -> Pixels {
        self.layout.get(HISTORY_WIDTH, cx).unwrap_or(DEFAULT_WIDTH)
    }
    pub(super) fn set_history(&mut self, open: bool, has_chat: bool, cx: &App) -> Option<Pixels> {
        if self.history == open {
            return None;
        }
        self.history = open;
        if open {
            self.refresh[self.docked_left as usize] = SplitRefresh::Requested(Arc::new(()));
        }
        if open && has_chat && !self.maximized {
            let width = self.history_width(cx);
            self.widened = Some(width);
            Some(width)
        } else {
            self.widened.take().map(|width| -width)
        }
    }
    pub(super) fn on_split_resized(&self, size: Pixels, cx: &mut App) -> Result<(), StoreError> {
        self.layout.set(HISTORY_WIDTH, size, cx)
    }
    pub(super) fn on_shared_width_changed(&mut self, value: Option<Pixels>, cx: &App) -> bool {
        let width = value.unwrap_or(DEFAULT_WIDTH);
        let mut changed = false;
        for (slot, history) in [1, 0].into_iter().enumerate() {
            let size = self.splits[slot].read(cx).sizes().get(history).copied();
            if size.is_none_or(|size| f32::from(size).round() != f32::from(width)) {
                self.refresh[slot] = SplitRefresh::Requested(Arc::new(()));
                changed = true;
            }
        }
        changed
    }
    pub(super) fn settling_current(&self, slot: usize, ticket: &Arc<()>) -> bool {
        self.history && slot == self.docked_left as usize && self.refresh[slot].current(ticket)
    }
    pub(super) fn render_split(
        &mut self,
        chat: AnyElement,
        history: AnyElement,
        window: &mut Window,
        cx: &mut Context<AgentPanel>,
    ) -> AnyElement {
        let slot = self.docked_left as usize;
        let container = self.splits[slot].read(cx).container_size();
        if let Some((reset, ticket)) = self.refresh[slot].observe_container(container) {
            if reset {
                self.splits[slot].update(cx, |state, _| *state = ResizableState::default());
            }
            let panel = cx.weak_entity();
            let origin = window.window_handle();
            window.on_next_frame(move |window, cx| {
                if window.window_handle() == origin {
                    let _ = panel.update(cx, |panel, cx| {
                        panel.on_history_split_settling(slot, &ticket, cx);
                    });
                }
            });
        }
        let history = resizable_panel()
            .size(self.history_width(cx))
            .size_range(px(180.)..px(640.))
            .child(
                div()
                    .id("agent-history-column")
                    .test_support()
                    .size_full()
                    .flex()
                    .flex_col()
                    .map(|column| {
                        if self.docked_left {
                            column.border_r_1()
                        } else {
                            column.border_l_1()
                        }
                    })
                    .border_color(cx.theme().border)
                    .child(history),
            );
        let chat = resizable_panel()
            .size_range(px(240.)..Pixels::MAX)
            .child(div().size_full().flex().flex_col().child(chat));
        let split = h_resizable("agent-split").with_state(&self.splits[self.docked_left as usize]);
        let split = if self.docked_left {
            split.child(history).child(chat)
        } else {
            split.child(chat).child(history)
        };
        div().flex_1().min_h_0().child(split).into_any_element()
    }
}
