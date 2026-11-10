//! Language rows own retained version drafts; installation remains behind the setup port.
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Window,
    base::TestSupportExt as _,
    component::{
        ActiveTheme as _, Disableable as _, IndexPath, Selectable as _, Sizable as _,
        StyledExt as _,
        button::{Button, ButtonVariants as _},
        h_flex,
        input::{Input, InputEvent, InputState},
        select::{SearchableVec, Select, SelectEvent, SelectState},
        v_flex,
    },
    div, img,
    prelude::*,
    px, svg,
};
use nocterm_project_providers::{LanguageSelection, ProgrammingLanguage, ProjectSetupError};

type VersionSelect = Entity<SelectState<SearchableVec<&'static str>>>;
struct LanguageRow {
    language: ProgrammingLanguage,
    enabled: bool,
    version: VersionSelect,
    custom: Entity<InputState>,
}
impl LanguageRow {
    fn default_label(language: ProgrammingLanguage) -> &'static str {
        match language {
            ProgrammingLanguage::Rust => "Stable",
            ProgrammingLanguage::Bun => "Latest stable",
            _ => "Distribution default",
        }
    }
    fn example(language: ProgrammingLanguage) -> &'static str {
        match language {
            ProgrammingLanguage::Python => "3.13 or 3.13.9",
            ProgrammingLanguage::Rust => "1.90.0",
            ProgrammingLanguage::Bun => "1.3.3",
            ProgrammingLanguage::Node => "22 or 22.15.0",
            ProgrammingLanguage::Java => "21",
        }
    }
    fn custom_selected(&self, cx: &App) -> bool {
        self.version.read(cx).selected_value() == Some(&"Custom version")
    }
    fn selection(&self, cx: &App) -> Result<LanguageSelection, ProjectSetupError> {
        if self.custom_selected(cx) {
            LanguageSelection::parse(self.language, self.custom.read(cx).value().as_ref())
        } else {
            Ok(LanguageSelection::default_for(self.language))
        }
    }
}
pub(crate) struct LanguagesChanged;
impl EventEmitter<LanguagesChanged> for LanguageChooser {}
pub(crate) struct LanguageChooser {
    rows: Vec<LanguageRow>,
    observed: Option<nocterm_project_providers::EnvironmentSnapshot>,
    settings: bool,
    locked: bool,
    _subscriptions: Vec<Subscription>,
}
impl LanguageChooser {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![];
        let rows = ProgrammingLanguage::ALL
            .into_iter()
            .map(|language| {
                let version = cx.new(|cx| {
                    SelectState::new(
                        SearchableVec::new(vec![
                            LanguageRow::default_label(language),
                            "Custom version",
                        ]),
                        Some(IndexPath::new(0)),
                        window,
                        cx,
                    )
                });
                let custom = cx.new(|cx| {
                    InputState::new(window, cx).placeholder(LanguageRow::example(language))
                });
                subscriptions.push(cx.subscribe(
                    &version,
                    |_, _, _: &SelectEvent<SearchableVec<&'static str>>, cx| {
                        cx.emit(LanguagesChanged);
                        cx.notify();
                    },
                ));
                subscriptions.push(cx.subscribe(&custom, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.emit(LanguagesChanged);
                        cx.notify();
                    }
                }));
                LanguageRow {
                    language,
                    enabled: language == ProgrammingLanguage::Python,
                    version,
                    custom,
                }
            })
            .collect();
        Self {
            rows,
            observed: None,
            settings: false,
            locked: false,
            _subscriptions: subscriptions,
        }
    }
    pub(crate) fn for_settings(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut chooser = Self::new(window, cx);
        chooser.settings = true;
        for row in &mut chooser.rows {
            row.enabled = false;
        }
        chooser
    }
    pub(crate) fn configure(&mut self, locked: bool, cx: &mut Context<Self>) {
        self.locked = locked;
        cx.notify();
    }
    pub(crate) fn observe_environment(
        &mut self,
        snapshot: nocterm_project_providers::EnvironmentSnapshot,
        cx: &mut Context<Self>,
    ) {
        self.observed = Some(snapshot);
        cx.notify();
    }
    pub(crate) fn validation_focus(&self, cx: &App) -> gpui_kit::FocusHandle {
        use gpui_kit::Focusable as _;
        let row = self
            .rows
            .iter()
            .find(|row| row.enabled && row.selection(cx).is_err())
            .unwrap_or(&self.rows[0]);
        if row.custom_selected(cx) {
            row.custom.focus_handle(cx)
        } else {
            row.version.focus_handle(cx)
        }
    }
    pub(crate) fn selections(&self, cx: &App) -> Result<Vec<LanguageSelection>, ProjectSetupError> {
        self.rows
            .iter()
            .filter(|row| row.enabled)
            .map(|row| row.selection(cx))
            .collect()
    }
    fn language_button(&self, row: &LanguageRow, cx: &mut Context<Self>) -> Button {
        let language = row.language;
        let enabled = row.enabled;
        let disabled = self.locked;
        let mark = format!("projects/languages/{}.svg", language.mark());
        Button::new(SharedString::from(format!(
            "project-language-{}",
            language.mark()
        )))
        .ghost()
        .small()
        .w(px(148.))
        .justify_start()
        .selected(enabled)
        .disabled(disabled)
        .role(gpui_kit::Role::CheckBox)
        .toggled(enabled)
        .accessibility_label(language.name())
        .child(
            h_flex()
                .w_full()
                .justify_start()
                .gap_2()
                .child(
                    gpui_kit::component::Icon::new(if enabled {
                        nocterm_ui::IconName::Check
                    } else {
                        nocterm_ui::IconName::Plus
                    })
                    .size_3(),
                )
                .child(if language == ProgrammingLanguage::Rust {
                    svg()
                        .path(mark.clone())
                        .size_4()
                        .text_color(cx.theme().foreground)
                        .into_any_element()
                } else {
                    img(mark).size_4().into_any_element()
                })
                .child(language.name()),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if this.locked {
                return;
            }
            if let Some(row) = this.rows.iter_mut().find(|row| row.language == language) {
                row.enabled = !row.enabled;
            }
            cx.emit(LanguagesChanged);
            cx.notify();
        }))
    }
    fn observation(
        &self,
        snapshot: &nocterm_project_providers::EnvironmentSnapshot,
        row: &LanguageRow,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let language = row.language;
        let installed = snapshot
            .tools
            .iter()
            .find(|tool| tool.language == language)
            .and_then(|tool| tool.version.as_deref())
            .unwrap_or("Not detected");
        let reason = row
            .selection(cx)
            .ok()
            .and_then(|selection| snapshot.support(&selection).err());
        v_flex()
            .gap_1()
            .child(
                div()
                    .id(SharedString::from(format!(
                        "project-installed-{}",
                        language.mark()
                    )))
                    .test_support()
                    .aria_label(format!("Installed: {installed}"))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Installed: {installed}")),
            )
            .when_some(reason, |view, reason| {
                view.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(reason.to_string()),
                )
            })
    }
    fn row(&self, row: &LanguageRow, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let language = row.language;
        let disabled = self.locked;
        let enabled = row.enabled;
        let custom = row.custom_selected(cx);
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .id(SharedString::from(format!(
                        "project-language-row-{}",
                        language.mark()
                    )))
                    .test_support()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(self.language_button(row, cx))
                    .child(
                        div().flex_1().min_w_0().child(
                            Select::new(&row.version)
                                .id(SharedString::from(format!(
                                    "project-version-{}",
                                    language.mark()
                                )))
                                .small()
                                .w_full()
                                .disabled(disabled || !enabled),
                        ),
                    )
                    .when(custom, |view| {
                        view.child(
                            div()
                                .id(SharedString::from(format!(
                                    "project-version-custom-{}",
                                    language.mark()
                                )))
                                .test_support()
                                .w(px(136.))
                                .min_w_0()
                                .child(
                                    Input::new(&row.custom)
                                        .id("input")
                                        .aria_label(format!("{} version", language.name()))
                                        .small()
                                        .disabled(disabled || !enabled),
                                ),
                        )
                    }),
            )
            .when_some(self.observed.as_ref(), |view, snapshot| {
                view.child(self.observation(snapshot, row, cx))
            })
            .when(enabled && custom && row.selection(cx).is_err(), |view| {
                view.child(div().text_xs().text_color(cx.theme().danger).child(format!(
                    "Enter a version such as {}.",
                    LanguageRow::example(language)
                )))
            })
    }
}
impl Render for LanguageChooser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().id("project-languages").test_support().w_full().min_w_0().gap_1()
            .child(div().text_sm().font_medium().child("Programming languages"))
            .children(self.rows.iter().map(|row| self.row(row, cx)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(if self.settings {
                "Unchecked languages stay installed. Open a new workspace terminal after installing tools."
            } else { "Select languages and versions to install in the new container." }))
    }
}
