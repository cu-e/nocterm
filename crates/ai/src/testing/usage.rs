//! Shared scripted account usage source with explicitly completed reads.
use crate::usage::{AccountUsageSource, RateLimit, UsageAgent, UsageReadError};
use futures::{channel::oneshot, future::BoxFuture};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

type Answer = Result<Option<Vec<RateLimit>>, UsageReadError>;
#[derive(Default)]
struct Script {
    requests: Vec<UsageAgent>,
    answers: VecDeque<BoxFuture<'static, Answer>>,
}
#[derive(Clone, Default)]
pub struct ScriptedUsageSource(Arc<Mutex<Script>>);
impl ScriptedUsageSource {
    pub fn push(&self, result: Answer) {
        self.0
            .lock()
            .expect("usage script")
            .answers
            .push_back(Box::pin(async move { result }));
    }
    pub fn pending(&self) -> oneshot::Sender<Answer> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .lock()
            .expect("usage script")
            .answers
            .push_back(Box::pin(async move { receiver.await.unwrap_or(Ok(None)) }));
        sender
    }
    pub fn requests(&self) -> Vec<UsageAgent> {
        self.0.lock().expect("usage script").requests.clone()
    }
}
impl AccountUsageSource for ScriptedUsageSource {
    fn read(&self, agent: UsageAgent) -> BoxFuture<'static, Answer> {
        let script = self.0.clone();
        Box::pin(async move {
            let answer = {
                let mut script = script.lock().expect("usage script");
                script.requests.push(agent);
                script.answers.pop_front()
            };
            match answer {
                Some(answer) => answer.await,
                None => Ok(None),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{FutureExt, executor::block_on};
    use std::{error::Error, path::PathBuf};
    fn agent() -> UsageAgent {
        UsageAgent {
            id: "account".into(),
            command: "codex".into(),
            args: vec![],
        }
    }
    #[test]
    fn held_usage_failures_preserve_read_seek_category_native_kind_and_allocation() {
        for error in [
            UsageReadError::read(
                "/report",
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ),
            UsageReadError::seek(
                "/report",
                std::io::Error::from(std::io::ErrorKind::InvalidInput),
            ),
        ] {
            let expected = error.clone();
            let source = ScriptedUsageSource::default();
            let answer = source.pending();
            let mut future = source.read(agent());
            assert!(future.as_mut().now_or_never().is_none());
            assert_eq!(source.requests(), vec![agent()]);
            answer.send(Err(error)).unwrap();
            let observed = block_on(future).unwrap_err();
            match (&expected, &observed) {
                (UsageReadError::Read { path: a, .. }, UsageReadError::Read { path: b, .. })
                | (UsageReadError::Seek { path: a, .. }, UsageReadError::Seek { path: b, .. }) => {
                    assert_eq!(a, b);
                    assert_eq!(a, &PathBuf::from("/report"));
                }
                _ => panic!("usage operation category must be preserved"),
            }
            let native = expected
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            let shared = observed
                .source()
                .unwrap()
                .source()
                .unwrap()
                .downcast_ref::<std::io::Error>()
                .unwrap();
            assert!(std::ptr::eq(native, shared));
            assert_eq!(native.kind(), shared.kind());
            assert_eq!(expected.to_string(), observed.to_string());
        }
    }
    #[test]
    fn usage_source_preserves_immediate_native_error_before_its_deeper_cause() {
        #[derive(Debug)]
        struct NativeContext;
        impl std::fmt::Display for NativeContext {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("usage context")
            }
        }
        impl Error for NativeContext {
            fn source(&self) -> Option<&(dyn Error + 'static)> {
                Some(&std::fmt::Error)
            }
        }
        let error = UsageReadError::read("/report", std::io::Error::other(NativeContext));
        let clone = error.clone();
        let native = error
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        let shared = clone
            .source()
            .unwrap()
            .source()
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap();
        assert!(std::ptr::eq(native, shared));
        assert_eq!(native.kind(), std::io::ErrorKind::Other);
        assert!(native.source().unwrap().is::<std::fmt::Error>());
    }
}
