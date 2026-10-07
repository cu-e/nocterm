// Modified by Nocterm contributors; see NOCTERM.md. Licensed under Apache-2.0.
//! Effective kitty flags and bounded saved states, independent per screen.
use super::*;

#[derive(Default)]
pub(super) struct KeyboardProtocol {
    current: KeyboardModes,
    saved: Vec<KeyboardModes>,
}

impl<T> Term<T> {
    /// Changes only at keyboard negotiation, screen transitions, and reset.
    pub fn keyboard_protocol_epoch(&self) -> u64 {
        self.keyboard_protocol_epoch
    }

    pub(super) fn advance_keyboard_epoch(&mut self) {
        self.keyboard_protocol_epoch = self.keyboard_protocol_epoch.wrapping_add(1);
    }

    pub fn keyboard_mode(&self) -> KeyboardModes {
        self.keyboard_protocol.current
    }
    pub fn modify_other_keys(&self) -> ModifyOtherKeys {
        self.modify_other_keys
    }

    pub(super) fn sync_keyboard_mode(&mut self) {
        self.mode.remove(TermMode::KITTY_KEYBOARD_PROTOCOL);
        self.mode.insert(self.keyboard_protocol.current.into());
    }
}

impl<T: EventListener> Term<T> {
    pub(super) fn keyboard_report(&self) {
        if self.config.kitty_keyboard {
            self.event_proxy.send_event(Event::PtyWrite(format!(
                "\x1b[?{}u",
                self.keyboard_protocol.current.bits()
            )));
        }
    }

    pub(super) fn keyboard_push(&mut self, mode: KeyboardModes) {
        if !self.config.kitty_keyboard {
            return;
        }
        self.advance_keyboard_epoch();
        if self.keyboard_protocol.saved.len() == KEYBOARD_MODE_STACK_MAX_DEPTH {
            self.keyboard_protocol.saved.remove(0);
        }
        self.keyboard_protocol
            .saved
            .push(self.keyboard_protocol.current);
        self.keyboard_protocol.current = mode;
        self.sync_keyboard_mode();
    }

    pub(super) fn keyboard_pop(&mut self, count: u16) {
        if !self.config.kitty_keyboard || count == 0 {
            return;
        }
        self.advance_keyboard_epoch();
        let saved = &mut self.keyboard_protocol.saved;
        let next = saved.len().saturating_sub(usize::from(count));
        let mode = if next == 0 {
            KeyboardModes::NO_MODE
        } else {
            saved[next]
        };
        saved.truncate(next);
        self.keyboard_protocol.current = mode;
        self.sync_keyboard_mode();
    }

    pub(super) fn keyboard_set(&mut self, mode: KeyboardModes, apply: KeyboardModesApplyBehavior) {
        if !self.config.kitty_keyboard {
            return;
        }
        self.advance_keyboard_epoch();
        self.keyboard_protocol.current = match apply {
            KeyboardModesApplyBehavior::Replace => mode,
            KeyboardModesApplyBehavior::Union => self.keyboard_protocol.current.union(mode),
            KeyboardModesApplyBehavior::Difference => {
                self.keyboard_protocol.current.difference(mode)
            }
        };
        self.sync_keyboard_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::VoidListener;

    struct Size;
    impl Dimensions for Size {
        fn total_lines(&self) -> usize {
            3
        }
        fn screen_lines(&self) -> usize {
            3
        }
        fn columns(&self) -> usize {
            80
        }
    }

    fn terminal() -> Term<VoidListener> {
        Term::new(
            Config {
                kitty_keyboard: true,
                ..Default::default()
            },
            &Size,
            VoidListener,
        )
    }

    #[test]
    fn bounded_stack_evicts_keyboard_states_without_changing_title_stack() {
        let mut terminal = terminal();
        terminal.title_stack.push(Some("preserved".into()));
        for number in 0..KEYBOARD_MODE_STACK_MAX_DEPTH + 10 {
            terminal.keyboard_push(KeyboardModes::from_bits_truncate((number % 32) as u8));
        }
        assert_eq!(
            terminal.keyboard_protocol.saved.len(),
            KEYBOARD_MODE_STACK_MAX_DEPTH
        );
        assert_eq!(terminal.title_stack, [Some("preserved".into())]);
        terminal.keyboard_pop(1);
        assert_eq!(terminal.keyboard_mode().bits(), 8);
        terminal.keyboard_pop(u16::MAX);
        assert_eq!(terminal.keyboard_mode(), KeyboardModes::NO_MODE);
    }

    #[test]
    fn direct_changes_and_nested_saved_states_are_independent() {
        let mut terminal = terminal();
        terminal.keyboard_set(
            KeyboardModes::from_bits_truncate(5),
            KeyboardModesApplyBehavior::Replace,
        );
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(7));
        terminal.keyboard_set(
            KeyboardModes::from_bits_truncate(8),
            KeyboardModesApplyBehavior::Union,
        );
        assert_eq!(terminal.keyboard_mode().bits(), 15);
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(31));
        terminal.keyboard_pop(1);
        assert_eq!(terminal.keyboard_mode().bits(), 15);
        terminal.swap_alt();
        assert_eq!(terminal.keyboard_mode().bits(), 0);
        terminal.keyboard_set(
            KeyboardModes::from_bits_truncate(2),
            KeyboardModesApplyBehavior::Replace,
        );
        terminal.swap_alt();
        assert_eq!(terminal.keyboard_mode().bits(), 15);
        terminal.keyboard_pop(1);
        assert_eq!(terminal.keyboard_mode().bits(), 0);
    }

    #[test]
    fn disabling_protocol_and_reset_clear_both_screen_states() {
        let mut terminal = terminal();
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(7));
        terminal.swap_alt();
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(31));
        terminal.modify_other_keys = ModifyOtherKeys::EnableAll;
        terminal.set_options(Config::default());
        assert_eq!(terminal.keyboard_protocol.current.bits(), 0);
        assert!(terminal.keyboard_protocol.saved.is_empty());
        assert_eq!(terminal.inactive_keyboard_protocol.current.bits(), 0);
        assert!(terminal.inactive_keyboard_protocol.saved.is_empty());
        assert_eq!(terminal.modify_other_keys, ModifyOtherKeys::Reset);
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(31));
        assert_eq!(terminal.keyboard_mode().bits(), 0);
        terminal.set_options(Config {
            kitty_keyboard: true,
            ..Default::default()
        });
        terminal.keyboard_push(KeyboardModes::from_bits_truncate(7));
        terminal.reset_state();
        assert_eq!(terminal.keyboard_mode().bits(), 0);
        assert!(terminal.keyboard_protocol.saved.is_empty());
        assert!(terminal.inactive_keyboard_protocol.saved.is_empty());
    }
}
