//! The colours a plugin view is drawn with.

use gpui::Hsla;
use nocterm_plugin_api::Tone;

/// Plain colours that the application takes from its theme.
///
/// The caution tone is a colour the application takes from its theme, like the others. What
/// tells a plugin from the host is the frame (a marker in the bar, the provenance the shell
/// writes beside a panel's title), never a colour.
#[derive(Clone, Copy, Debug)]
pub struct ViewStyle {
    /// Text and icons of the default tone.
    pub text: Hsla,
    pub muted: Hsla,
    pub success: Hsla,
    pub caution: Hsla,
    /// The frame border, dividers and the meter track.
    pub border: Hsla,
    /// A hovered row or control, as the application's own panels hover theirs.
    pub hover: Hsla,
    /// The selected row of a list.
    pub selected: Hsla,
    /// The marker of the frame.
    pub marker: Hsla,
    /// The colours of a chart's first and second series, the application's chart colours.
    pub chart: [Hsla; 2],
}

impl ViewStyle {
    pub(crate) fn tone(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Default => self.text,
            Tone::Muted => self.muted,
            Tone::Success => self.success,
            Tone::Caution => self.caution,
        }
    }
}
