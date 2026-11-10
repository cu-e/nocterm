//! A fold: a list or a grid cut to its first rows, the last of them fading out, with a button
//! under it that shows all of it or fewer, as the server icon picker does. The caller keeps
//! whether it is open.
use gpui_kit::{
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, Pixels, Window,
    component::{
        ActiveTheme as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        v_flex,
    },
    div, linear_color_stop, linear_gradient,
    prelude::*,
    px,
};

use crate::IconName;

/// The height of the fade over the last row of a folded list.
const LIST_FADE: f32 = 14.;

/// The button under a fold: "Show all N" with a chevron down, or "Show fewer" with a chevron up.
pub fn fold_button(id: impl Into<ElementId>, expanded: bool, count: usize) -> Button {
    Button::new(id)
        .xsmall()
        .ghost()
        .icon(if expanded {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        })
        .label(if expanded {
            "Show fewer".to_owned()
        } else {
            format!("Show all {count}")
        })
}

/// The gradient that fades the bottom `height` of a folded block into `background`. The block
/// it is a child of is `relative`.
pub fn fade(background: Hsla, height: Pixels) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(height)
        .bg(linear_gradient(
            180.,
            linear_color_stop(background.opacity(0.), 0.),
            linear_color_stop(background, 1.),
        ))
}

/// `items` one under the other. When there are more than `folded`, only the first `folded` show
/// while it is folded, the last of them fading out, and the fold button under them calls
/// `toggle`; open, all of them show and the button folds them again.
pub fn fold_list(
    id: impl Into<ElementId>,
    items: Vec<AnyElement>,
    folded: usize,
    expanded: bool,
    toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let count = items.len();
    if count <= folded {
        return v_flex().w_full().children(items).into_any_element();
    }
    let shown: Vec<AnyElement> = if expanded {
        items
    } else {
        items.into_iter().take(folded).collect()
    };
    let background = cx.theme().background;
    v_flex()
        .w_full()
        .child(
            div()
                .relative()
                .w_full()
                .child(v_flex().w_full().children(shown))
                .when(!expanded, |list| {
                    list.child(fade(background, px(LIST_FADE)))
                }),
        )
        .child(div().child(fold_button(id, expanded, count).on_click(toggle)))
        .into_any_element()
}
