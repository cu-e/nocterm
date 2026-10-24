//! A bar too narrow for its entries moves the least important into a menu.
//!
//! [`BarOverflow`] keeps what the row measured: the width of each entry and of the row itself.
//! From those it plans which entries to hide (see [`plan`]) and hands the rest to the row. The
//! measures arrive one frame late, from the row's own paint, so a bar is drawn whole until it
//! has been measured and settles one frame after its width or its entries change.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use gpui_kit::{Pixels, px};
use nocterm_plugin_model::ContributionId;

use super::{BarEntry, BarInputs};
use crate::plugin::NOTIFICATIONS;
use plan::{Kind, Row, Slot, footprint, plan};

mod menu;
mod plan;

pub(super) use menu::OverflowButton;

/// The space between the children of a slot, which `gap_1` draws.
pub(super) const SLOT_GAP: Pixels = px(4.);
/// The width of a toggle button. A badge beside the icon makes the toggle wider than this.
pub(super) const TOGGLE_WIDTH: Pixels = px(24.);
/// The width of a separator.
pub(super) const SEPARATOR_WIDTH: Pixels = px(1.);
/// What the notice line keeps before an entry goes into the menu: the line truncates to this
/// first, with its bell.
const NOTICE_RESERVE: Pixels = px(96.);
/// A measure below this is no measure: the entry draws nothing.
const EMPTY: Pixels = px(0.5);

/// The ranks of what may be hidden, lowest first.
mod rank {
    pub(super) const WIDGET: u8 = 0;
    pub(super) const TERMINAL: u8 = 1;
    pub(super) const TOGGLE: u8 = 2;
    pub(super) const OPEN_TOGGLE: u8 = 3;
}

/// What the row learned of the window and its design.
#[derive(Clone, Copy, Debug)]
pub(super) struct Frame {
    /// The padding on both sides of the row together.
    pub(super) padding: Pixels,
    /// The widest the centre grows.
    pub(super) centre_max: Pixels,
}

#[derive(Default)]
struct State {
    widths: HashMap<ContributionId, Pixels>,
    row: Option<Pixels>,
    open: bool,
}

/// The measures of a window's bar and whether its overflow menu is open. Clones share one state:
/// the inputs of each frame, the row that measures and the click handlers see the same.
#[derive(Clone, Default)]
pub(in crate::workspace) struct BarOverflow(Rc<RefCell<State>>);

impl BarOverflow {
    /// Records the width an entry was drawn at; whether the plan may change with it.
    pub(super) fn observe_entry(&self, id: &ContributionId, width: Pixels) -> bool {
        let mut state = self.0.borrow_mut();
        let known = state.widths.insert(id.clone(), width);
        known.is_none_or(|known| differs(known, width))
    }

    /// Records the width of the row; whether the plan may change with it.
    pub(super) fn observe_row(&self, width: Pixels) -> bool {
        let known = self.0.borrow_mut().row.replace(width);
        known.is_none_or(|known| differs(known, width))
    }

    pub(super) fn width_of(&self, id: &ContributionId) -> Option<Pixels> {
        self.0.borrow().widths.get(id).copied()
    }

    pub(super) fn is_open(&self) -> bool {
        self.0.borrow().open
    }

    pub(super) fn set_open(&self, open: bool) {
        self.0.borrow_mut().open = open;
    }

    /// Moves the entries that do not fit into the menu. The bar is as it was drawn until its row
    /// has been measured.
    pub(super) fn collapse(&self, inputs: BarInputs, frame: Frame) -> BarInputs {
        let Some(row) = self.0.borrow().row else {
            return inputs;
        };
        let inner = (row - frame.padding).max(px(0.));
        let hidden = if inputs.is_centred() {
            self.centred(&inputs, inner, frame)
        } else {
            self.flat(&inputs, inner)
        };
        if !any(&hidden) {
            self.set_open(false);
            return inputs;
        }
        let [left, centre, right] = hidden;
        inputs.collapsed(&left, &centre, &right)
    }

    /// One row: the left entries, a spacer, the button and the right entries share the width.
    fn flat(&self, inputs: &BarInputs, inner: Pixels) -> [Vec<bool>; 3] {
        let (left, right) = (self.slots(inputs.left()), self.slots(inputs.right()));
        let zones: [&[Slot]; 2] = [&left, &right];
        let row = Row {
            gap: SLOT_GAP,
            spare: 1,
        };
        let hidden = plan(&zones, |hidden| {
            let button = any(hidden).then_some(TOGGLE_WIDTH);
            if footprint(&zones, hidden, row, button) > inner {
                vec![0, 1]
            } else {
                Vec::new()
            }
        });
        let mut hidden = hidden.into_iter();
        let (left, right) = (
            hidden.next().unwrap_or_default(),
            hidden.next().unwrap_or_default(),
        );
        [left, Vec::new(), right]
    }

    /// Three slots: the centre takes what it needs up to its maximum, the outer slots share the
    /// rest equally, as the row lays them out, and the button sits in the right slot.
    ///
    /// A centre that is too wide gives way, and so does an outer slot that is. The centre goes
    /// by its rank like the others: hiding its entries gives the outer slots more room.
    fn centred(&self, inputs: &BarInputs, inner: Pixels, frame: Frame) -> [Vec<bool>; 3] {
        let (left, centre, right) = (
            self.slots(inputs.left()),
            self.slots(inputs.center()),
            self.slots(inputs.right()),
        );
        let zones: [&[Slot]; 3] = [&left, &centre, &right];
        let (outer, spacer) = (
            Row {
                gap: SLOT_GAP,
                spare: 0,
            },
            Row {
                gap: SLOT_GAP,
                spare: 1,
            },
        );
        let centre_room = frame.centre_max.min(inner);
        let hidden = plan(&zones, |hidden| {
            let button = any(hidden).then_some(TOGGLE_WIDTH);
            let centre_width = footprint(&[&centre], &hidden[1..2], outer, None);
            let side = ((inner - centre_width.min(centre_room) - SLOT_GAP * 2.) / 2.).max(px(0.));
            let mut over = Vec::new();
            if footprint(&[&left], &hidden[0..1], outer, None) > side {
                over.extend([0, 1]);
            }
            if centre_width > centre_room {
                over.push(1);
            }
            if footprint(&[&right], &hidden[2..3], spacer, button) > side {
                over.extend([1, 2]);
            }
            over
        });
        let mut hidden = hidden.into_iter();
        let mut next = || hidden.next().unwrap_or_default();
        [next(), next(), next()]
    }

    fn slots(&self, entries: &[BarEntry]) -> Vec<Slot> {
        entries.iter().map(|entry| self.slot(entry)).collect()
    }

    fn slot(&self, entry: &BarEntry) -> Slot {
        let measured = |id: &ContributionId| self.width_of(id);
        match entry {
            BarEntry::Separator => Slot {
                width: SEPARATOR_WIDTH,
                kind: Kind::Separator,
            },
            BarEntry::Notices => {
                let shown = measured(&NOTIFICATIONS.id).is_some_and(|width| width > EMPTY);
                Slot {
                    width: if shown { NOTICE_RESERVE } else { px(0.) },
                    kind: Kind::Fixed,
                }
            }
            BarEntry::Panel(toggle) => Slot {
                width: measured(toggle.id()).unwrap_or(TOGGLE_WIDTH),
                kind: if toggle.has_attention() {
                    Kind::Fixed
                } else if toggle.is_selected() {
                    Kind::Hideable(rank::OPEN_TOGGLE)
                } else {
                    Kind::Hideable(rank::TOGGLE)
                },
            },
            BarEntry::LocalTerminal(_) => Slot {
                width: entry
                    .id()
                    .and_then(|id| measured(&id))
                    .unwrap_or(TOGGLE_WIDTH),
                kind: Kind::Hideable(rank::TERMINAL),
            },
            BarEntry::Widget(id, _) => match measured(id) {
                Some(width) if width > EMPTY => Slot {
                    width,
                    kind: Kind::Hideable(rank::WIDGET),
                },
                // Not drawn yet, or drawing nothing: there is nothing to move.
                _ => Slot {
                    width: px(0.),
                    kind: Kind::Fixed,
                },
            },
        }
    }
}

/// Whether the plan hides anything.
fn any(hidden: &[Vec<bool>]) -> bool {
    hidden.iter().flatten().any(|hidden| *hidden)
}

/// Whether two measures differ by more than rounding.
fn differs(a: Pixels, b: Pixels) -> bool {
    (a - b).abs() > EMPTY
}
