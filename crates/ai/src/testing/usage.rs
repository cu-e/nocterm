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
