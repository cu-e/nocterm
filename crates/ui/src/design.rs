use gpui_kit::{App, Global};
use nocterm_design::DesignTokens;

/// The design tokens in force, as an application global.
///
/// Replace it with `cx.set_global` to restyle the running application.
pub struct Design {
    tokens: DesignTokens,
}

impl Design {
    pub fn new(tokens: DesignTokens) -> Self {
        Self { tokens }
    }
}

impl Global for Design {}

/// Read access to the design tokens from any context.
pub trait ActiveDesign {
    fn design(&self) -> &DesignTokens;
}

impl ActiveDesign for App {
    fn design(&self) -> &DesignTokens {
        &self.global::<Design>().tokens
    }
}
