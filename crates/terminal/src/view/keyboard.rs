//! Platform event ownership and composed input. Encoding stays in nocterm-vt.
use nocterm_vt::KeyboardState;

const MAX_PRESSED_KEYS: usize = 128;

#[derive(Default)]
pub(super) struct KeyboardInput {
    context: Option<(KeyboardState, bool, u64, u64)>,
    pressed: Vec<String>,
    pending_text: Option<String>,
}

impl KeyboardInput {
    pub(super) fn clear(&mut self) {
        self.pressed.clear();
        self.pending_text = None;
        self.context = None;
    }

    pub(super) fn sync(
        &mut self,
        state: KeyboardState,
        alt_screen: bool,
        epoch: u64,
        protocol_epoch: u64,
    ) {
        let context = (state, alt_screen, epoch, protocol_epoch);
        if self.context != Some(context) {
            self.clear();
            self.context = Some(context);
        }
    }

    pub(super) fn forget(&mut self, key: &str) -> bool {
        if self.pending_text.as_deref() == Some(key) {
            self.pending_text = None;
        }
        let Some(index) = self.pressed.iter().position(|owned| owned == key) else {
            return false;
        };
        self.pressed.swap_remove(index);
        true
    }

    pub(super) fn own(&mut self, key: String) {
        if self.pressed.len() < MAX_PRESSED_KEYS && !self.pressed.contains(&key) {
            self.pressed.push(key);
        }
    }
    pub(super) fn clear_pending(&mut self) {
        self.pending_text = None;
    }
    pub(super) fn defer(&mut self, key: Option<String>) {
        self.pending_text = key;
    }
    pub(super) fn take_pending(&mut self) -> Option<String> {
        self.pending_text.take()
    }
}
