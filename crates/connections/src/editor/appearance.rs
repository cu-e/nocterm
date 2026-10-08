//! The editor's icon, colour and country choices.
use gpui_kit::{
    AnyElement, Context, SharedString,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::Input,
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::{IconName, form};

use super::{ConnectionEditor, ICON_ROWS_HEIGHT};

impl ConnectionEditor {
    /// "Automatic" and every system in the catalog, then the colour and the
    /// country.
    #[expect(clippy::too_many_lines, reason = "predates the limit")]
    pub(super) fn appearance_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (muted, border, accent, radius) = (
            theme.muted_foreground,
            theme.primary,
            theme.accent,
            theme.radius,
        );
        let typed = self.icon_color.read(cx).value().trim().to_owned();
        let typed = if typed.is_empty() || typed.starts_with('#') {
            typed
        } else {
            format!("#{typed}")
        };
        let color = Some(typed).filter(|color| crate::os::is_valid_color(color));
        let detected = self
            .editing
            .then(|| crate::ServerFacts::global(cx).read(cx).os(self.id))
            .flatten();
        let tile = |id: SharedString,
                    choice: Option<&'static str>,
                    os: Option<&'static crate::os::Os>,
                    tooltip: SharedString,
                    cx: &mut Context<Self>| {
            let selected = self.icon.as_deref() == choice;
            div()
                .id(id)
                .test_support()
                .size_7()
                .flex()
                .items_center()
                .justify_center()
                .rounded(radius)
                .border_1()
                .border_color(if selected {
                    border
                } else {
                    gpui_kit::transparent_black()
                })
                .cursor_pointer()
                .hover(|tile| tile.bg(accent))
                .child(crate::panel::server_icon(os, color.as_deref(), muted))
                .tooltip(move |_, cx| {
                    cx.new(|_| gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()))
                        .into()
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.icon = choice.map(str::to_owned);
                    cx.notify();
                }))
        };
        let chosen = self.icon.as_deref().and_then(crate::os::find).or(detected);
        let hint = match chosen {
            Some(os) => format!("Empty: the {} colour, {}.", os.name, os.color),
            None => "Empty: grey.".to_owned(),
        };
        let automatic = match detected {
            Some(os) => format!("Automatic: {}, as detected", os.name),
            None => "Automatic: detected on connecting".to_owned(),
        };
        let mut tiles = vec![tile(
            "editor-icon-auto".into(),
            None,
            detected,
            automatic.into(),
            cx,
        )];
        // Collapsed, the chosen icon comes first so it stays in view.
        let promoted = (!self.icons_expanded)
            .then(|| self.icon.as_deref().and_then(crate::os::find))
            .flatten();
        let others = crate::os::CATALOG
            .iter()
            .filter(|os| promoted.is_none_or(|promoted| promoted.id != os.id));
        for os in promoted.into_iter().chain(others) {
            tiles.push(tile(
                format!("editor-icon-{}", os.id).into(),
                Some(os.id),
                Some(os),
                os.name.into(),
                cx,
            ));
        }
        let background = form::page_background(cx);
        let count = tiles.len();
        let expanded = self.icons_expanded;
        let grid = div()
            .relative()
            .when(!expanded, |grid| {
                grid.h(px(ICON_ROWS_HEIGHT)).overflow_hidden()
            })
            .child(h_flex().flex_wrap().gap_1().children(tiles))
            .when(!expanded, |grid| {
                grid.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .h(px(ICON_ROWS_HEIGHT / 2.))
                        .bg(gpui_kit::linear_gradient(
                            180.,
                            gpui_kit::linear_color_stop(background.opacity(0.), 0.),
                            gpui_kit::linear_color_stop(background, 1.),
                        )),
                )
            });
        let icon = v_flex().gap_1().child(grid).child(
            div().child(
                Button::new("editor-icons-expand")
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
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.icons_expanded = !this.icons_expanded;
                        cx.notify();
                    })),
            ),
        );
        let colour = h_flex()
            .gap_1()
            .child(
                div()
                    .w(rems(8.))
                    .child(Input::new(&self.icon_color).small()),
            )
            .child(
                Button::new("editor-icon-color-reset")
                    .small()
                    .ghost()
                    .label("Reset")
                    .tooltip("Use the system's brand colour")
                    .disabled(self.icon_color.read(cx).value().is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.icon_color
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
            );
        vec![
            form::stacked_row(
                "Icon",
                "Automatic follows the detected system.",
                icon,
                None,
                cx,
            ),
            form::row("Colour", hint, colour, cx),
            self.render_country(cx),
        ]
    }

    fn render_country(&self, cx: &mut Context<Self>) -> AnyElement {
        let border = cx.theme().border;
        let detecting = crate::facts::country_detection_enabled(cx);
        let facts = crate::ServerFacts::global(cx).read(cx);
        let typed = self.country.read(cx).value().trim().to_ascii_lowercase();
        let detected = self
            .editing
            .then(|| facts.country(self.id))
            .flatten()
            .filter(|_| detecting);
        let shown = if typed.is_empty() {
            detected.map(str::to_owned)
        } else {
            Some(typed.clone()).filter(|code| crate::geo::is_country_code(code))
        };
        let flag = shown.as_deref().and_then(|code| facts.flag(code));
        let hint = match (typed.is_empty(), detected) {
            (false, _) => "Two-letter ISO code. Clear it to detect the country again.".to_owned(),
            (true, Some(code)) => {
                format!("Empty: detected automatically, {}.", code.to_uppercase())
            }
            (true, None) if detecting => {
                "Empty: detected from the server's public address on connecting.".to_owned()
            }
            (true, None) => "Empty: no flag. Detection is off in Settings › Appearance.".to_owned(),
        };
        let control = h_flex()
            .gap_2()
            .when_some(flag, |row, flag| {
                row.child(
                    div()
                        .rounded_xs()
                        .overflow_hidden()
                        .border_1()
                        .border_color(border)
                        .child(
                            gpui_kit::img(flag)
                                .w(px(24.))
                                .h(px(16.))
                                .object_fit(gpui_kit::ObjectFit::Cover),
                        ),
                )
            })
            .child(div().w(rems(5.)).child(Input::new(&self.country).small()));
        form::row("Country", hint, control, cx)
    }
}
