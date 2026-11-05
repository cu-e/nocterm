//! Recording session-announcement port, with controlled metadata preparation.
use super::lock;
use crate::authorized_keys::{ConnectedHost, ConnectedHosts};
use futures::future::BoxFuture;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct State {
    prepared: usize,
    published: Vec<ConnectedHost>,
    waiting: VecDeque<BoxFuture<'static, ()>>,
}
#[derive(Clone, Default)]
pub struct ScriptedConnectedHosts {
    state: Arc<Mutex<State>>,
}
impl ScriptedConnectedHosts {
    pub fn suspend_prepare(&self, until: BoxFuture<'static, ()>) {
        lock(&self.state).waiting.push_back(until);
    }
    pub fn prepared(&self) -> usize {
        lock(&self.state).prepared
    }
    pub fn published(&self) -> Vec<ConnectedHost> {
        lock(&self.state).published.clone()
    }
}
impl ConnectedHosts for ScriptedConnectedHosts {
    fn prepare(&self, host: ConnectedHost) -> ConnectedHost {
        let wait = {
            let mut state = lock(&self.state);
            state.prepared += 1;
            state.waiting.pop_front()
        };
        if let Some(wait) = wait {
            futures::executor::block_on(wait);
        }
        host
    }
    fn connected(&self, host: ConnectedHost) {
        let mut state = lock(&self.state);
        if state.published.len() < 256 {
            state.published.push(host);
        }
    }
}
