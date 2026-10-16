//! The editor's icon, colour and country choices.
use gpui_kit::{
    AnyElement, App, Context, Entity, SharedString, Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, Sizable as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px, rems,
};
use nocterm_ui::{IconName, form};

use super::{ConnectionEditor, ICON_ROWS_HEIGHT, field, text};
use nocterm_connections_model::draft::AppearanceDraft;
use nocterm_connections_model::profiles::{Profile, ProfileId};
pub(super) struct AppearanceFields {
    icon: Option<String>,
    color: Entity<InputState>,
    expanded: bool,
    country: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl AppearanceFields {
    pub(super) fn new(
        profile: Option<&Profile>,
        installed: &crate::Installed,
        window: &mut Window,
        cx: &mut Context<ConnectionEditor>,
    ) -> Self {
        let color = field(
            "Brand colour",
            profile
                .and_then(|p| p.icon_color.clone())
                .unwrap_or_default(),
            window,
            cx,
        );
        let country = field(
            "Auto",
            profile
                .and_then(|p| p.country.clone())
                .map(|code| code.to_uppercase())
                .unwrap_or_default(),
            window,
            cx,
        );
        let color_subscription = cx.subscribe_in(&color, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.error = None;
                cx.notify();
            }
        });
        let flags = installed.facts().downgrade();
        let country_subscription =
            cx.subscribe_in(&country, window, move |this, input, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    let code = input.read(cx).value().trim().to_ascii_lowercase();
                    if let (Some(facts), Ok(code)) = (
                        flags.upgrade(),
                        nocterm_connections_model::observation::CountryCode::new(&code),
                    ) {
                        let _ = facts.update(cx, |facts, cx| facts.request_flag(code, cx));
                    }
                    cx.notify();
                }
            });
        Self {
            icon: profile.and_then(|p| p.icon.clone()),
            color,
            expanded: false,
            country,
            _subscriptions: vec![color_subscription, country_subscription],
        }
    }
    pub(super) fn color(&self) -> &Entity<InputState> {
        &self.color
    }
    pub(super) fn country(&self) -> &Entity<InputState> {
        &self.country
    }
    pub(super) fn choose(&mut self, icon: Option<&str>) {
        self.icon = icon.map(str::to_owned);
    }
    pub(super) fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }
    pub(super) fn snapshot(&self, cx: &App) -> AppearanceDraft {
        AppearanceDraft {
            icon: self.icon.clone().unwrap_or_default(),
            icon_color: text(&self.color, cx),
            country: text(&self.country, cx),
        }
    }
}

impl AppearanceFields {
    /// "Automatic" and every system in the catalog, then the colour and the
    /// country.
    pub(super) fn rows(
        &self,
        id: ProfileId,
        editing: bool,
        installed: &crate::Installed,
        cx: &mut Context<ConnectionEditor>,
    ) -> Vec<AnyElement> {
        let color = self.preview_color(cx);
        let detected = editing.then(|| installed.os(id, cx)).flatten();
        let chosen = self.icon.as_deref().and_then(crate::os::find).or(detected);
        let hint = match chosen {
            Some(os) => format!("Empty: the {} colour, {}.", os.name, os.color),
            None => "Empty: grey.".to_owned(),
        };
        let tiles = self.tiles(detected, color, cx);
        let background = nocterm_ui::hsla(
            nocterm_ui::TerminalStyle::with_settings(
                installed
                    .settings()
                    .setting::<nocterm_ui::TerminalSettings>(cx),
                cx,
            )
            .background,
        );
        let icon = self.icon_grid(tiles, background, cx);
        let colour = self.color_control(cx);
        vec![
            form::stacked_row(
                "Icon",
                "Automatic follows the detected system.",
                icon,
                None,
                cx,
            ),
            form::row("Colour", hint, colour, cx),
            self.render_country(id, editing, installed, cx),
        ]
    }

    fn preview_color(&self, cx: &App) -> Option<String> {
        let typed = self.color.read(cx).value().trim().to_owned();
        let typed = if typed.is_empty() || typed.starts_with('#') {
            typed
        } else {
            format!("#{typed}")
        };
        Some(typed).filter(|color| crate::os::is_valid_color(color))
    }
    fn tiles(
        &self,
        detected: Option<&'static crate::os::Os>,
        color: Option<String>,
        cx: &mut Context<ConnectionEditor>,
    ) -> Vec<AnyElement> {
        let theme = cx.theme();
        let (muted, border, accent, radius) = (
            theme.muted_foreground,
            theme.primary,
            theme.accent,
            theme.radius,
        );
        let tile = |id: SharedString,
                    choice: Option<&'static str>,
                    os: Option<&'static crate::os::Os>,
                    tooltip: SharedString,
                    cx: &mut Context<ConnectionEditor>| {
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
                    this.appearance.choose(choice);
                    cx.notify();
                }))
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
        let promoted = (!self.expanded)
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
        tiles
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect()
    }
    fn icon_grid(
        &self,
        tiles: Vec<AnyElement>,
        background: gpui_kit::Hsla,
        cx: &mut Context<ConnectionEditor>,
    ) -> AnyElement {
        let count = tiles.len();
        let expanded = self.expanded;
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
                        this.appearance.toggle_expanded();
                        cx.notify();
                    })),
            ),
        );
        icon.into_any_element()
    }
    fn color_control(&self, cx: &mut Context<ConnectionEditor>) -> AnyElement {
        let colour = h_flex()
            .gap_1()
            .child(div().w(rems(8.)).child(Input::new(&self.color).small()))
            .child(
                Button::new("editor-icon-color-reset")
                    .small()
                    .ghost()
                    .label("Reset")
                    .tooltip("Use the system's brand colour")
                    .disabled(self.color.read(cx).value().is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.appearance
                            .color()
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        cx.notify();
                    })),
            );
        colour.into_any_element()
    }

    fn render_country(
        &self,
        id: ProfileId,
        editing: bool,
        installed: &crate::Installed,
        cx: &mut Context<ConnectionEditor>,
    ) -> AnyElement {
        let border = cx.theme().border;
        let detecting = installed.detecting_country(cx);
        let facts = installed.facts().read(cx);
        let typed = self.country.read(cx).value().trim().to_ascii_lowercase();
        let detected = editing
            .then(|| facts.country(id, cx))
            .flatten()
            .filter(|_| detecting);
        let shown = if typed.is_empty() {
            detected.map(str::to_owned)
        } else {
            Some(typed.clone()).filter(|code| crate::geo::is_country_code(code))
        };
        let flag = shown.as_deref().and_then(|code| installed.flag(code, cx));
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
            .child(
                div()
                    .w(rems(5.))
                    .child(Input::new(&self.country).id("editor-country").small()),
            );
        form::row("Country", hint, control, cx)
    }
}
