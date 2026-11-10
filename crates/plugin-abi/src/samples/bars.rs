//! Samples of the bar of a plugin's terminal: the call that draws it, an event of it, and the
//! effects that show, move and take it away.

use alloc::{string::String, vec, vec::Vec};

use nocterm_plugin_api::{
    ContributionContext, Edge, Effect, EffectRequest, Effects, Event, Kind, Zone,
    capability::terminals::Bar,
};

use super::{Sample, empty_snapshot, local, reply, sample};
use crate::{Answer, Call, WireSnapshot, encode_call, encode_reply};

fn bar_snapshot() -> WireSnapshot {
    WireSnapshot {
        context: ContributionContext {
            kind: Kind::Toolbar,
            zone: Zone::Center,
        },
        ..empty_snapshot()
    }
}

pub(super) fn samples() -> Vec<Sample> {
    let effects = Edge::ALL
        .into_iter()
        .map(Some)
        .chain([None])
        .map(|edge| EffectRequest {
            key: None,
            effect: Effect::TerminalsBar(Bar {
                terminal: String::from("srv"),
                edge,
            }),
        })
        .collect();
    vec![
        sample(
            "call-render-terminal-bar",
            encode_call(&Call::RenderTerminalBar {
                local: local("panel"),
                terminal: String::from("srv"),
                snapshot: bar_snapshot(),
            }),
        ),
        sample(
            "call-event-terminal-bar",
            encode_call(&Call::TerminalBarEvent {
                local: local("panel"),
                terminal: String::from("srv"),
                event: Event::Activated {
                    key: String::from("restart"),
                },
                snapshot: bar_snapshot(),
            }),
        ),
        sample(
            "reply-effects-terminal-bar",
            encode_reply(&reply(Answer::Effects(Effects(effects)), &[])),
        ),
    ]
}
