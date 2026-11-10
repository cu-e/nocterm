//! Samples of the files of a plugin's terminal: a request of every kind, the answers to them,
//! and a tab and a request with bytes.

use alloc::{string::String, vec, vec::Vec};

use nocterm_plugin_api::{
    Effect, EffectRequest, Effects, Event,
    capability::{
        files::{FileAnswer, FileEntry, FileFailure, FileOp, FileReply},
        net::{Method, Request},
        terminals::Open,
    },
};

use super::{Sample, empty_snapshot, local, reply, sample};
use crate::{Answer, Call, encode_call, encode_reply};

pub(super) fn samples() -> Vec<Sample> {
    let mut samples = requests();
    samples.push(sample(
        "reply-effects-files",
        encode_reply(&reply(Answer::Effects(answers()), &[])),
    ));
    samples
}

/// One `file-request` event of every kind.
fn requests() -> Vec<Sample> {
    const NAMES: [&str; 7] = [
        "call-event-file-list",
        "call-event-file-stat",
        "call-event-file-read",
        "call-event-file-write",
        "call-event-file-create-dir",
        "call-event-file-remove",
        "call-event-file-rename",
    ];
    let path = || String::from("/plugins/a.yml");
    let ops = [
        FileOp::List { path: path() },
        FileOp::Stat { path: path() },
        FileOp::Read { path: path() },
        FileOp::Write {
            path: path(),
            data: String::from("TWFu"),
            create: true,
        },
        FileOp::CreateDir { path: path() },
        FileOp::Remove {
            path: path(),
            directory: false,
        },
        FileOp::Rename {
            from: path(),
            to: String::from("/b.yml"),
        },
    ];
    ops.into_iter()
        .zip(NAMES)
        .enumerate()
        .map(|(index, (op, name))| {
            let event = Event::FileRequest {
                terminal: String::from("srv"),
                request: u32::try_from(index).unwrap_or(0),
                op,
            };
            sample(
                name,
                encode_call(&Call::Event {
                    local: local("panel"),
                    event,
                    snapshot: empty_snapshot(),
                }),
            )
        })
        .collect()
}

/// An answer of every kind, a request with bytes and a tab that serves files.
fn answers() -> Effects {
    let entry = FileEntry {
        name: String::from("a.yml"),
        directory: false,
        size: Some(12),
        modified: Some(1_700_000_000),
    };
    let answers = [
        FileAnswer::Listed(vec![entry.clone()]),
        FileAnswer::Found(Some(entry)),
        FileAnswer::Content(String::from("TWFu")),
        FileAnswer::Done,
        FileAnswer::Failed(FileFailure::NotFound),
        FileAnswer::Failed(FileFailure::Other(String::from("busy"))),
    ];
    let mut effects: Vec<EffectRequest> = answers
        .into_iter()
        .enumerate()
        .map(|(index, answer)| EffectRequest {
            key: None,
            effect: Effect::TerminalsFiles(FileReply {
                terminal: String::from("srv"),
                request: u32::try_from(index).unwrap_or(0),
                answer,
            }),
        })
        .collect();
    effects.push(EffectRequest {
        key: Some(String::from("read")),
        effect: Effect::NetRequest(Request {
            method: Method::Post,
            path: String::from("/write"),
            body: Some(String::from("TWFu")),
            bytes: true,
        }),
    });
    effects.push(EffectRequest {
        key: None,
        effect: Effect::TerminalsOpen(Open {
            local: local("panel"),
            terminal: String::from("srv"),
            title: String::from("Build"),
            files: true,
        }),
    });
    Effects(effects)
}
