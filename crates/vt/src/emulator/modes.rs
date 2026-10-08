use super::*;

/// The switches a running program has flipped that change how input is encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    /// Arrow keys use the application (`SS3`) encoding.
    pub app_cursor: bool,
    /// Pastes are wrapped so the program can tell them from typing.
    pub bracketed_paste: bool,
    /// The alternate screen is showing (a full-screen program is running).
    pub alt_screen: bool,
    /// The wheel moves the cursor instead of the scrollback on the alternate screen.
    pub alternate_scroll: bool,
    /// The program wants to be told when the terminal gains or loses focus.
    pub focus_events: bool,
    /// Button presses and releases are reported.
    pub mouse_clicks: bool,
    /// Pointer movement with a button held is reported.
    pub mouse_drag: bool,
    /// All pointer movement is reported.
    pub mouse_motion: bool,
    /// Mouse reports use the `SGR` encoding.
    pub mouse_sgr: bool,
    /// Mouse reports use the UTF-8 extension of the legacy encoding.
    pub mouse_utf8: bool,
    /// Mouse tracking negotiation identity, independent of host viewport scrolling.
    pub mouse_tracking_epoch: u64,
    /// Keyboard protocol features negotiated by the running application.
    pub keyboard: crate::KeyboardState,
    /// Negotiation/reset identity, independent of ordinary terminal output.
    pub keyboard_protocol_epoch: u64,
}

impl Modes {
    /// Whether the program takes any mouse input at all.
    pub fn reports_mouse(&self) -> bool {
        self.mouse_clicks || self.mouse_drag || self.mouse_motion
    }
}

impl Emulator {
    pub fn modes(&self) -> Modes {
        let mode = *self.term.mode();
        Modes {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
            mouse_clicks: mode.contains(TermMode::MOUSE_REPORT_CLICK),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            mouse_utf8: mode.contains(TermMode::UTF8_MOUSE),
            mouse_tracking_epoch: self.mouse_tracking_epoch,
            keyboard_protocol_epoch: self.term.keyboard_protocol_epoch(),
            keyboard: crate::KeyboardState {
                disambiguate: mode.contains(TermMode::DISAMBIGUATE_ESC_CODES),
                report_event_types: mode.contains(TermMode::REPORT_EVENT_TYPES),
                report_alternate_keys: mode.contains(TermMode::REPORT_ALTERNATE_KEYS),
                report_all_keys: mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC),
                report_associated_text: mode.contains(TermMode::REPORT_ASSOCIATED_TEXT),
                modify_other_keys: match self.term.modify_other_keys() {
                    alacritty_terminal::vte::ansi::ModifyOtherKeys::Reset => {
                        crate::ModifyOtherKeys::Off
                    }
                    alacritty_terminal::vte::ansi::ModifyOtherKeys::EnableExceptWellDefined => {
                        crate::ModifyOtherKeys::ExceptWellDefined
                    }
                    alacritty_terminal::vte::ansi::ModifyOtherKeys::EnableAll => {
                        crate::ModifyOtherKeys::All
                    }
                },
            },
        }
    }
}
