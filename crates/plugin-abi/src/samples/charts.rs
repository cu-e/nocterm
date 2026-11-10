//! A sample of a view with charts: one series against a top value and a span, and two series
//! scaled to their largest value.

use alloc::{vec, vec::Vec};

use nocterm_plugin_api::{NodeKind, Tone, View};

use super::{Sample, node, reply, sample};
use crate::{Answer, encode_reply};

pub(super) fn samples() -> Vec<Sample> {
    let view = View {
        nodes: vec![
            node(None, None, NodeKind::Column, Tone::Default),
            node(
                Some(0),
                None,
                NodeKind::Chart {
                    series: vec![vec![12, 30, 36]],
                    max: Some(100),
                    span: Some(60),
                },
                Tone::Default,
            ),
            node(
                Some(0),
                None,
                NodeKind::Chart {
                    series: vec![vec![4, 9, 7], vec![2, 3, 5]],
                    max: None,
                    span: None,
                },
                Tone::Caution,
            ),
        ],
        refresh: None,
    };
    vec![sample(
        "reply-view-charts",
        encode_reply(&reply(Answer::View(view), &[])),
    )]
}
