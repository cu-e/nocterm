//! How the window shell draws what plugins contribute: icons and column widths.
//!
//! The plugin model names icons and width classes as plain data; this module maps them onto
//! the drawable icon set and the design tokens.

use std::ops::Range;

use gpui_kit::{Anchor, App, Pixels, rems};
use nocterm_plugin_model::{Icon, WidthClass, Zone};

use crate::{ActiveDesign as _, IconName};

/// The drawable icon for a contribution's [`Icon`]. Every variant maps, so a new variant does
/// not compile until it has an icon.
pub fn contribution_icon(icon: Icon) -> IconName {
    match icon {
        Icon::Activity => IconName::Activity,
        Icon::Bell => IconName::Bell,
        Icon::Bot => IconName::Bot,
        Icon::Box => IconName::Box,
        Icon::Clock => IconName::Clock,
        Icon::Container => IconName::Container,
        Icon::FolderTree => IconName::FolderTree,
        Icon::Gem => IconName::Gem,
        Icon::Hexagon => IconName::Hexagon,
        Icon::Layers => IconName::Layers,
        Icon::PanelRight => IconName::PanelRight,
        Icon::Plug => IconName::Plug,
        Icon::Puzzle => IconName::Puzzle,
        Icon::ScrollText => IconName::ScrollText,
        Icon::Server => IconName::Server,
        Icon::Shapes => IconName::Shapes,
        Icon::ShieldCheck => IconName::ShieldCheck,
        Icon::SquareTerminal => IconName::SquareTerminal,
        Icon::Upload => IconName::Upload,
    }
}

/// The corner a popover of a status bar item anchors to, so it opens toward the inside of the
/// window: from the left zone it grows rightwards, from the right zone leftwards, and from the
/// centre it grows rightwards too.
pub fn zone_anchor(zone: Zone) -> Anchor {
    match zone {
        Zone::Left | Zone::Center => Anchor::BottomLeft,
        Zone::Right => Anchor::BottomRight,
    }
}

/// The width a class of panel column opens at and the range the user may resize it within,
/// from the design tokens. `rem` is the window's rem size.
pub fn width_range(class: WidthClass, rem: Pixels, cx: &App) -> (Pixels, Range<Pixels>) {
    let layout = &cx.design().layout;
    let column = match class {
        WidthClass::Navigation => &layout.sidebar,
        WidthClass::Assistant => &layout.agent_panel,
    };
    let range = rems(column.min_width).to_pixels(rem)..rems(column.max_width).to_pixels(rem);
    (rems(column.width).to_pixels(rem), range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, px};

    use crate::{Design, DesignTokens};

    #[test]
    fn a_popover_opens_toward_the_inside_of_the_window() {
        assert_eq!(zone_anchor(Zone::Left), Anchor::BottomLeft);
        assert_eq!(zone_anchor(Zone::Center), Anchor::BottomLeft);
        assert_eq!(zone_anchor(Zone::Right), Anchor::BottomRight);
    }

    #[gpui_kit::test]
    fn a_class_opens_inside_its_resize_range(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(Design::new(DesignTokens::builtin()));
            for class in [WidthClass::Navigation, WidthClass::Assistant] {
                let (open, range) = width_range(class, px(16.), cx);
                assert!(range.start <= open && open <= range.end, "{class:?}");
                assert!(range.start > px(0.));
            }
        });
    }

    #[gpui_kit::test]
    fn the_assistant_column_follows_the_agent_panel_tokens(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let tokens = DesignTokens::builtin();
            let agent = tokens.layout.agent_panel.clone();
            cx.set_global(Design::new(tokens));
            let (open, range) = width_range(WidthClass::Assistant, px(10.), cx);
            assert_eq!(open, px(agent.width * 10.));
            assert_eq!(range, px(agent.min_width * 10.)..px(agent.max_width * 10.));
        });
    }

    #[test]
    fn every_icon_has_a_distinct_drawable() {
        use std::collections::HashSet;
        let icons = Icon::ALL;
        let drawn: HashSet<_> = icons
            .into_iter()
            .map(|icon| contribution_icon(icon).path())
            .collect();
        assert_eq!(drawn.len(), icons.len());
    }

    #[test]
    fn every_contribution_icon_is_embedded() {
        use gpui_kit::AssetSource as _;
        let assets = crate::Assets::default();
        for icon in Icon::ALL {
            let path = contribution_icon(icon).path();
            assert!(
                assets.load(&path).unwrap().is_some(),
                "{icon:?} is not embedded at {path}"
            );
        }
    }
}
