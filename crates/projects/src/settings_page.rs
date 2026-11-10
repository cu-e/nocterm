//! Workspace defaults edit the original installed settings through shared checked text receipts.
use gpui_kit::{App, AppContext as _, Context, FocusHandle, Focusable, Window, prelude::*};
use nocterm_project_model::RootPath;
use nocterm_project_providers::WorkspacesSettings;
use nocterm_store::SettingsHandle;
use nocterm_ui::{FieldPresentation, SettingsPage, SettingsPageSpec, SettingsTextField, form};

pub(super) struct WorkspacesPage {
    directory: FieldPresentation,
}
impl WorkspacesPage {
    pub(super) fn new(
        settings: SettingsHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = SettingsTextField::new(
            settings,
            "workspace-default-directory",
            |document| {
                document
                    .get::<WorkspacesSettings>()
                    .container_directory
                    .clone()
            },
            |document, text| {
                let root = RootPath::parse(text).map_err(nocterm_store::StoreError::edit)?;
                document.set(WorkspacesSettings {
                    container_directory: root.as_str().to_owned(),
                });
                Ok(())
            },
            window,
            cx,
        );
        Self {
            directory: FieldPresentation::new(field, window, cx),
        }
    }
}
impl Focusable for WorkspacesPage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.directory.field().focus_handle(cx)
    }
}
impl SettingsPage for WorkspacesPage {
    fn flush(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory.field().flush(window, cx);
    }
    fn on_deactivate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory.field().deactivate(window, cx);
    }
    fn on_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.directory.field().flush_for_close(window, cx);
    }
}
impl Render for WorkspacesPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui_kit::component::v_flex().w_full()
            .child(form::page_header("Workspaces", None, cx))
            .child(form::section("Container defaults", [form::stacked_row(
                "Project directory",
                "Used when the directory is left empty for a new or existing container. Applies to future workspace drafts.",
                self.directory.field().input(), self.directory.field().error(cx), cx,
            )], cx))
    }
}
pub fn settings_page(installed: &crate::Installed) -> SettingsPageSpec {
    let settings = installed.settings().clone();
    SettingsPageSpec::new("workspaces", "Workspaces", move |window, cx| {
        cx.new(|cx| WorkspacesPage::new(settings.clone(), window, cx))
    })
    .with_icon(nocterm_ui::IconName::Folder)
}
