//! Guest-side dispatch of one ABI call, which `export!` and `testing::Host` share.

use alloc::vec::Vec;

use nocterm_plugin_abi::{AbiError, Answer, Call, Reply, decode_call, encode_reply};
use nocterm_plugin_api::Snapshot;

use crate::{Cx, Plugin};

/// Decodes a call, runs the plugin and encodes the reply.
pub fn dispatch<P: Plugin>(plugin: &mut P, input: &[u8]) -> Result<Vec<u8>, AbiError> {
    let reply = match decode_call(input)? {
        Call::Render { local, snapshot } => {
            let snapshot = Snapshot::from(snapshot);
            let mut cx = Cx::new(&local, &snapshot);
            let view = plugin.render(&mut cx);
            Reply {
                answer: Answer::View(view),
                log: cx.into_log(),
            }
        }
        Call::Event {
            local,
            event,
            snapshot,
        } => {
            let snapshot = Snapshot::from(snapshot);
            let mut cx = Cx::new(&local, &snapshot);
            let effects = plugin.on_event(&mut cx, &event);
            Reply {
                answer: Answer::Effects(effects),
                log: cx.into_log(),
            }
        }
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
