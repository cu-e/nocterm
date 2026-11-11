// Nocterm modifications, licensed under the upstream Apache-2.0 license.
//! Relative wheel movement uses measured rows, never stale or missing height hints.
use super::{Count, ListItem, ListItemSummary, ListOffset};
use crate::{Pixels, Size, px};
use sum_tree::{Bias, SumTree};

/// Resolve as much of a relative movement as the caller's measurement budget allows.
/// The remainder stays relative to the returned anchor, so later frames need no
/// absolute content-height estimate and no queue of input events.
pub(super) fn advance(
    items: &mut SumTree<ListItem>,
    mut anchor: ListOffset,
    mut distance: Pixels,
    mut measure: impl FnMut(usize) -> Option<Size<Pixels>>,
) -> (ListOffset, Pixels) {
    let count = items.summary().count;
    while distance != px(0.) {
        if distance < px(0.) {
            let consumed = anchor.offset_in_item.min(-distance).max(px(0.));
            anchor.offset_in_item -= consumed;
            distance += consumed;
            if distance == px(0.) || anchor.item_ix == 0 {
                return (anchor, px(0.));
            }
            if skip_measured(items, &mut anchor, &mut distance) {
                continue;
            }
            let previous = anchor.item_ix - 1;
            let Some(height) = row_height(items, previous, &mut measure) else {
                break;
            };
            anchor.item_ix = previous;
            anchor.offset_in_item = height;
        } else {
            if anchor.item_ix == count {
                return (anchor, px(0.));
            }
            if skip_measured(items, &mut anchor, &mut distance) {
                continue;
            }
            let Some(height) = row_height(items, anchor.item_ix, &mut measure) else {
                break;
            };
            let consumed = (height - anchor.offset_in_item).max(px(0.)).min(distance);
            anchor.offset_in_item += consumed;
            distance -= consumed;
            if distance == px(0.) {
                if anchor.offset_in_item == height {
                    anchor.item_ix += 1;
                    anchor.offset_in_item = px(0.);
                }
                break;
            }
            anchor.item_ix += 1;
            anchor.offset_in_item = px(0.);
        }
    }
    (anchor, distance)
}

/// Measured subtrees retain logarithmic seeks, even for very large deltas.
fn skip_measured(
    items: &SumTree<ListItem>,
    anchor: &mut ListOffset,
    distance: &mut Pixels,
) -> bool {
    let mut cursor = items.cursor::<ListItemSummary>(());
    cursor.seek(&Count(anchor.item_ix), Bias::Right);
    let origin = cursor.start().height + anchor.offset_in_item;
    let upward = *distance < px(0.);
    let (boundary, extent) = if upward {
        cursor.search_backward(|summary| summary.unrendered_count > 0);
        cursor
            .item()
            .map_or((0, px(0.)), |_| (cursor.end().count, cursor.end().height))
    } else {
        if cursor.item().is_some_and(|item| item.size().is_none()) {
            return false;
        }
        cursor.search_forward(|summary| summary.unrendered_count > 0);
        (cursor.start().count, cursor.start().height)
    };
    if boundary == anchor.item_ix {
        return false;
    }
    let available = extent - origin;
    if distance.abs() >= available.abs() {
        *anchor = ListOffset {
            item_ix: boundary,
            offset_in_item: px(0.),
        };
        *distance -= available;
    } else {
        let target = origin + *distance;
        let (start, ..) = items.find::<ListItemSummary, _>((), &super::Height(target), Bias::Right);
        *anchor = ListOffset {
            item_ix: start.count,
            offset_in_item: target - start.height,
        };
        *distance = px(0.);
    }
    true
}

fn row_height(
    items: &mut SumTree<ListItem>,
    index: usize,
    measure: &mut impl FnMut(usize) -> Option<Size<Pixels>>,
) -> Option<Pixels> {
    let mut cursor = items.cursor::<Count>(());
    cursor.seek(&Count(index), Bias::Right);
    let item = cursor.item()?;
    if let Some(size) = item.size() {
        return Some(size.height);
    }
    let focus_handle = item.focus_handle();
    drop(cursor);
    let size = measure(index)?;
    let mut cursor = items.cursor::<Count>(());
    let mut replacement = cursor.slice(&Count(index), Bias::Right);
    cursor.next();
    replacement.extend([ListItem::Measured { size, focus_handle }], ());
    replacement.append(cursor.suffix(), ());
    drop(cursor);
    *items = replacement;
    Some(size.height)
}

/// A pending resize/remeasure owns clamping within its original top row.
pub(super) fn within_invalidated_row(
    items: &SumTree<ListItem>,
    anchor: ListOffset,
    distance: Pixels,
) -> Option<ListOffset> {
    let mut cursor = items.cursor::<ListItemSummary>(());
    cursor.seek(&Count(anchor.item_ix), Bias::Right);
    let height = cursor.item()?.size_hint()?.height;
    let offset = anchor.offset_in_item + distance;
    (offset >= px(0.) && offset <= height).then_some(ListOffset {
        item_ix: anchor.item_ix,
        offset_in_item: offset,
    })
}

pub(super) fn at_end(items: &SumTree<ListItem>, anchor: ListOffset, available: Pixels) -> bool {
    let mut cursor = items.cursor::<Count>(());
    cursor.seek(&Count(anchor.item_ix), Bias::Right);
    let suffix = cursor.suffix();
    suffix.summary().unrendered_count == 0
        && suffix.summary().height - anchor.offset_in_item <= available
}

/// Preserve the caller's explicit approximate-uniform measuring policy.
pub(super) fn estimated(
    items: &SumTree<ListItem>,
    anchor: ListOffset,
    distance: Pixels,
    maximum: Pixels,
) -> ListOffset {
    let mut cursor = items.cursor::<ListItemSummary>(());
    cursor.seek(&Count(anchor.item_ix), Bias::Right);
    let target = (cursor.start().height + anchor.offset_in_item + distance).clamp(px(0.), maximum);
    let (start, ..) = items.find::<ListItemSummary, _>((), &super::Height(target), Bias::Right);
    ListOffset {
        item_ix: start.count,
        offset_in_item: target - start.height,
    }
}
