//! The colours that mark a server, drawn in the current theme.

use gpui_kit::{App, Hsla, component::ActiveTheme as _};
use nocterm_hubs_model::HubColor;

/// The shade of `color` in the current theme.
pub(crate) fn tint(color: HubColor, cx: &App) -> Hsla {
    let theme = cx.theme();
    match color {
        HubColor::Red => theme.red,
        HubColor::Yellow => theme.yellow,
        HubColor::Green => theme.green,
        HubColor::Cyan => theme.cyan,
        HubColor::Blue => theme.blue,
        HubColor::Magenta => theme.magenta,
    }
}

#[cfg(test)]
mod tests {
    use super::tint;
    use crate::test_support::{fixture, server};
    use gpui_kit::{Hsla, TestAppContext};
    use nocterm_hubs_model::{HubColor, testing::FakeHub};

    #[gpui_kit::test]
    fn every_colour_has_its_own_shade_in_the_theme(cx: &mut TestAppContext) {
        let _window = fixture(cx, FakeHub::new().with_server(server()), Vec::new(), None);
        let shades: Vec<Hsla> = cx.update(|cx| {
            HubColor::ALL
                .into_iter()
                .map(|color| tint(color, cx))
                .collect()
        });
        for (index, shade) in shades.iter().enumerate() {
            assert!(
                !shades[..index].contains(shade),
                "{} repeats an earlier colour",
                HubColor::ALL[index].name()
            );
        }
    }
}
