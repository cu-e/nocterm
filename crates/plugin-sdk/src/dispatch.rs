//! Guest-side dispatch of one ABI call, which `export!` and `testing::Host` share.

use alloc::vec::Vec;

use nocterm_plugin_abi::{AbiError, Answer, Reply, decode_call, encode_reply};

use crate::{Cx, Plugin};

/// Decodes a call, runs the plugin and encodes the reply.
pub fn dispatch<P: Plugin>(plugin: &mut P, input: &[u8]) -> Result<Vec<u8>, AbiError> {
    let (local, event, snapshot) = decode_call(input)?.into_parts();
    let mut cx = Cx::new(&local, &snapshot);
    let answer = match event {
        None => Answer::View(plugin.render(&mut cx)),
        Some(event) => Answer::Effects(plugin.on_event(&mut cx, &event)),
    };
    let reply = Reply {
        answer,
        log: cx.into_log(),
    };
    encode_reply(&reply)
}

/// What `export!` expands against. Not an API: it changes with the macro.
#[doc(hidden)]
pub mod __private {
    pub use alloc::vec::Vec;

    pub use nocterm_plugin_abi::{ABI_VERSION, MAX_INPUT_BYTES, pack};

    pub use super::dispatch;
}
