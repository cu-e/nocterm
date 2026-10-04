use gpui_kit::{App, Global};
use nocterm_design::DesignTokens;

/// The base and effective design tokens in force as an application global.
pub struct Design {
    pub(crate) base: DesignTokens,
    pub(crate) tokens: DesignTokens,
    pub(crate) imported: [bool; 2],
}
impl Design {
    pub fn new(tokens: DesignTokens) -> Self {
        Self {
            base: tokens.clone(),
            tokens,
            imported: [false; 2],
        }
    }
    /// Built-in tokens plus theme.toml, before imported palettes replace colours.
    pub fn base(&self) -> &DesignTokens {
        &self.base
    }
    pub fn is_imported(&self, dark: bool) -> bool {
        self.imported[usize::from(dark)]
    }
}
impl Global for Design {}
/// Read access to the effective design tokens from any context.
pub trait ActiveDesign {
    fn design(&self) -> &DesignTokens;
}
impl ActiveDesign for App {
    fn design(&self) -> &DesignTokens {
        &self.global::<Design>().tokens
    }
}
