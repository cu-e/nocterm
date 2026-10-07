// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;

#[cfg(test)]
mod tests;

impl Interactivity {
    fn scroll_max(&self, bounds: Bounds<Pixels>, style: &Style, window: &Window) -> Point<Pixels> {
        fn round_to_two_decimals(pixels: Pixels) -> Pixels {
            const ROUNDING_FACTOR: f32 = 100.0;
            (pixels * ROUNDING_FACTOR).round() / ROUNDING_FACTOR
        }
        let rem_size = window.rem_size();
        // Taffy lays the box out with the padding snapped to the device pixel
        // grid (`to_taffy`); recomputed unsnapped, e.g. py_1 at a fractional
        // rem size, it exceeds `bounds` and leaves the box scrollable by the
        // sub-pixel difference.
        let padding = style
            .padding
            .to_pixels(bounds.size.into(), rem_size)
            .map(|edge| window.pixel_snap(*edge));
        let padding_size = size(padding.left + padding.right, padding.top + padding.bottom);
        // The floating point values produced by Taffy and ours often vary
        // slightly after ~5 decimal places. This can lead to cases where after
        // subtracting these, the container becomes scrollable for less than
        // 0.00000x pixels. As we generally don't benefit from a precision that
        // high for the maximum scroll, we round the scroll max to 2 decimal
        // places here.
        let padded_content_size = self.content_size + padding_size;
        Point::from(padded_content_size - bounds.size)
            .map(round_to_two_decimals)
            .max(&Default::default())
    }

    pub(super) fn clamp_scroll_position(
        &self,
        bounds: Bounds<Pixels>,
        style: &Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> Point<Pixels> {
        if let Some(scroll_offset) = self.scroll_offset.as_ref() {
            let mut scroll_to_bottom = false;
            let mut tracked_scroll_handle = self
                .tracked_scroll_handle
                .as_ref()
                .map(|handle| handle.0.borrow_mut());
            if let Some(mut scroll_handle_state) = tracked_scroll_handle.as_deref_mut() {
                scroll_handle_state.overflow = style.overflow;
                scroll_to_bottom = mem::take(&mut scroll_handle_state.scroll_to_bottom);
            }

            let scroll_max = self.scroll_max(bounds, style, window);
            // Clamp scroll offset in case scroll max is smaller now (e.g., if children
            // were removed or the bounds became larger).
            let mut scroll_offset = scroll_offset.borrow_mut();

            scroll_offset.x = scroll_offset.x.clamp(-scroll_max.x, px(0.));
            if scroll_to_bottom {
                scroll_offset.y = -scroll_max.y;
            } else {
                scroll_offset.y = scroll_offset.y.clamp(-scroll_max.y, px(0.));
            }

            if let Some(mut scroll_handle_state) = tracked_scroll_handle {
                scroll_handle_state.max_offset = scroll_max;
                scroll_handle_state.bounds = bounds;
            }

            *scroll_offset
        } else {
            Point::default()
        }
    }

    pub(super) fn paint_scroll_listener(
        &self,
        bounds: Bounds<Pixels>,
        hitbox: &Hitbox,
        style: &Style,
        window: &mut Window,
        _cx: &mut App,
    ) {
        if let Some(scroll_offset) = self.scroll_offset.clone() {
            let scroll_max = self.scroll_max(bounds, style, window);
            let ongoing_scroll = self.ongoing_scroll.clone();
            let overflow = style.overflow;
            let allow_concurrent_scroll = style.allow_concurrent_scroll;
            let restrict_scroll_to_axis = style.restrict_scroll_to_axis;
            let line_height = window.line_height();
            let hitbox = hitbox.clone();
            let current_view = window.current_view();
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                    let mut scroll_offset = scroll_offset.borrow_mut();
                    let old_scroll_offset = *scroll_offset;
                    let mut delta = event.delta.pixel_delta(line_height);

                    if restrict_scroll_to_axis
                        && event.delta.precise()
                        && let Some(ongoing_scroll) = &ongoing_scroll
                    {
                        ongoing_scroll
                            .borrow_mut()
                            .filter(&mut delta, event.touch_phase);
                    }

                    let mut delta_x = match overflow.x {
                        Overflow::Scroll if !delta.x.is_zero() => delta.x,
                        Overflow::Scroll
                            if !restrict_scroll_to_axis && overflow.y != Overflow::Scroll =>
                        {
                            delta.y
                        }
                        _ => Pixels::ZERO,
                    };
                    let mut delta_y = match overflow.y {
                        Overflow::Scroll if !delta.y.is_zero() => delta.y,
                        Overflow::Scroll
                            if !restrict_scroll_to_axis && overflow.x != Overflow::Scroll =>
                        {
                            delta.x
                        }
                        _ => Pixels::ZERO,
                    };
                    if !allow_concurrent_scroll && !delta_x.is_zero() && !delta_y.is_zero() {
                        if delta_x.abs() > delta_y.abs() {
                            delta_y = Pixels::ZERO;
                        } else {
                            delta_x = Pixels::ZERO;
                        }
                    }
                    // Compare the drawable position, not a transient out-of-bounds offset.
                    // Prepaint uses the same padding, pixel snapping and rounding.
                    scroll_offset.y = (scroll_offset.y + delta_y).clamp(-scroll_max.y, px(0.));
                    scroll_offset.x = (scroll_offset.x + delta_x).clamp(-scroll_max.x, px(0.));
                    if *scroll_offset != old_scroll_offset {
                        cx.notify(current_view);
                    }
                }
            });
        }
    }
}
