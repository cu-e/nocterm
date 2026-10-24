//! How much of each chip of the bar preview is drawn, so that the bar stays on one line.
//!
//! The box of the preview takes the width it is given. What the bar needs is estimated from
//! its entries. The bar is drawn the way the real one is, with the centre in the middle, while
//! that fits; then with its zones packed side by side; and compact chips scroll sideways when
//! even those do not fit.

use std::{cell::Cell, rc::Rc};

use gpui_kit::{Pixels, px};
use nocterm_plugin_model::{PluginCatalog, Zone};
use nocterm_window_layout::{ResolvedItem, ResolvedMember};

use crate::snapshot::Snapshot;

/// What a chip shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Density {
    /// The icon and the sample text of a widget, the title of a group.
    Full,
    /// The icon only; the text is a tooltip.
    Compact,
}

/// How the zones share the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spread {
    /// The sides share what the centre leaves, as in the bar, so the centre is in the middle.
    Centred,
    /// Each zone takes what it needs, side by side.
    Packed,
}

/// How the bar is drawn in its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    density: Density,
    spread: Spread,
    /// The width the compact bar needs, when that is more than the box has and the bar scrolls.
    scroll: Option<Pixels>,
}

impl Plan {
    pub(crate) fn density(&self) -> Density {
        self.density
    }

    pub(crate) fn spread(&self) -> Spread {
        self.spread
    }

    pub(crate) fn scroll(&self) -> Option<Pixels> {
        self.scroll
    }
}

/// The width the preview's box was drawn with last time.
///
/// The estimate of a chip is generous, so that a bar that is said to fit does.
#[derive(Clone, Default)]
pub(crate) struct Fit {
    width: Rc<Cell<Option<Pixels>>>,
}

/// An average advance of the small text of a chip.
const CHAR: f32 = 7.;
/// The icon of a chip.
const ICON: f32 = 16.;
/// A chip's border and the padding on both sides.
const CHIP_FRAME: f32 = 14.;
/// The gap inside a chip and in a group's box.
const INNER_GAP: f32 = 4.;
/// A group's border and padding.
const BOX_FRAME: f32 = 10.;
/// The gap between entries of a zone.
const ENTRY_GAP: f32 = 8.;
/// The gap between zones.
const ZONE_GAP: f32 = 16.;
/// The box's border and padding.
const BOX_PADDING: f32 = 26.;
/// The tag that marks a disabled chip.
const TAG: f32 = 64.;
/// A zone with nothing in it draws a placeholder.
const EMPTY: f32 = 80.;
/// The padding on both sides of a zone.
const ZONE_PAD: f32 = 8.;

impl Fit {
    /// Remembers the width of the box. Returns whether it changed, which asks for another draw.
    pub(crate) fn observe(&self, width: Pixels) -> bool {
        let changed = self
            .width
            .get()
            .is_none_or(|seen| (f32::from(seen) - f32::from(width)).abs() >= 1.);
        self.width.set(Some(width));
        changed
    }

    /// How to draw the bar in a box as wide as the last one seen. Before any is seen, in full.
    pub(crate) fn plan(&self, snapshot: &Snapshot, catalog: &PluginCatalog) -> Plan {
        let Some(width) = self.width.get() else {
            return Plan {
                density: Density::Full,
                spread: Spread::Centred,
                scroll: None,
            };
        };
        let room = f32::from(width);
        let fits = |density, spread| needed(snapshot, catalog, density, spread) <= room;
        let (density, spread) = [
            (Density::Full, Spread::Centred),
            (Density::Compact, Spread::Centred),
            (Density::Compact, Spread::Packed),
        ]
        .into_iter()
        .find(|(density, spread)| fits(*density, *spread))
        .unwrap_or((Density::Compact, Spread::Packed));
        let need = needed(snapshot, catalog, density, spread);
        Plan {
            density,
            spread,
            scroll: (need > room).then(|| px(need)),
        }
    }
}

/// The width the bar takes at `density`. Centred, the wider of the two sides counts on both, as
/// the real bar centres its middle.
fn needed(snapshot: &Snapshot, catalog: &PluginCatalog, density: Density, spread: Spread) -> f32 {
    let width = |zone| zone_width(snapshot, catalog, zone, density);
    let (left, centre, right) = (width(Zone::Left), width(Zone::Center), width(Zone::Right));
    let sides = match spread {
        Spread::Centred => 2. * left.max(right),
        Spread::Packed => left + right,
    };
    BOX_PADDING + sides + centre + 2. * ZONE_GAP
}

fn zone_width(snapshot: &Snapshot, catalog: &PluginCatalog, zone: Zone, density: Density) -> f32 {
    let items = snapshot.layout().zone(zone);
    if items.is_empty() {
        return EMPTY;
    }
    let widths: Vec<f32> = items
        .iter()
        .map(|item| match item {
            ResolvedItem::Widget(id) => {
                let text = catalog
                    .contribution(id)
                    .map_or(0, |spec| spec.preview.unwrap_or(spec.title).chars().count());
                chip(density, Some(text), snapshot.is_disabled(id, catalog))
            }
            ResolvedItem::Separator => 1.,
            ResolvedItem::Group(group) => {
                let title = snapshot
                    .groups()
                    .iter()
                    .find(|spec| spec.id == *group.id())
                    .map_or(0, |spec| spec.title.chars().count());
                let members: f32 = group
                    .members()
                    .iter()
                    .map(|member| match member {
                        ResolvedMember::Panel(_) => chip(density, None, false) + INNER_GAP,
                        ResolvedMember::Separator => 1. + INNER_GAP,
                    })
                    .sum();
                let title = match density {
                    Density::Full => title as f32 * CHAR + INNER_GAP,
                    Density::Compact => 0.,
                };
                BOX_FRAME + title + members
            }
        })
        .collect();
    let gaps = widths.len().saturating_sub(1) as f32 * ENTRY_GAP;
    ZONE_PAD + widths.iter().sum::<f32>() + gaps
}

/// One chip: an icon, and the text of a widget when the density shows it.
fn chip(density: Density, text: Option<usize>, disabled: bool) -> f32 {
    let label = match (density, text) {
        (Density::Full, Some(chars)) => INNER_GAP + chars as f32 * CHAR,
        _ => 0.,
    };
    let tag = if disabled && density == Density::Full {
        TAG
    } else {
        0.
    };
    CHIP_FRAME + ICON + label + tag
}
