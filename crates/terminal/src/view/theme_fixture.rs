//! Explicit paint lifetime for standalone terminal fixtures that change appearance.
use super::*;
pub(super) fn projection(
    view: &Entity<TerminalView>,
    cx: &mut TestAppContext,
) -> nocterm_ui::ThemeUi {
    cx.update(|cx| {
        let settings = view
            .read(cx)
            .terminal()
            .read(cx)
            .installed()
            .settings()
            .clone();
        let service = nocterm_theme_service::ThemesHandle::install(
            None,
            None,
            nocterm_themes::ThemeCatalog::default(),
            cx,
        );
        nocterm_ui::ThemeUi::install(settings, service, nocterm_ui::DesignTokens::builtin(), cx)
    })
}
