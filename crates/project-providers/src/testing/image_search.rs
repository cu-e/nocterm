//! Shared scripted registry search, retaining the captured execution authority.
use crate::{ImageSearch, ImageSearchError, ImageSearchFuture, ImageSearchQuery, RegistryImage};
use futures::{FutureExt as _, future::BoxFuture};
use nocterm_containers::Engine;
use nocterm_session::{HostExec, local_fs::CancelFlag};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

type SearchStep = (
    Option<BoxFuture<'static, ()>>,
    Result<Vec<RegistryImage>, ImageSearchError>,
);
#[derive(Clone)]
pub struct ImageSearchCall {
    pub exec: Arc<dyn HostExec>,
    pub engine: Engine,
    pub query: ImageSearchQuery,
    pub cancel: CancelFlag,
}
#[derive(Default)]
struct SearchScript {
    results: VecDeque<SearchStep>,
    calls: Vec<ImageSearchCall>,
}
#[derive(Clone, Default)]
pub struct FakeImageSearch {
    state: Arc<Mutex<SearchScript>>,
}
impl FakeImageSearch {
    pub fn respond(
        &self,
        result: Result<Vec<RegistryImage>, ImageSearchError>,
    ) -> Result<(), ImageSearchError> {
        self.enqueue(None, result)
    }
    pub fn respond_after(
        &self,
        gate: BoxFuture<'static, ()>,
        result: Result<Vec<RegistryImage>, ImageSearchError>,
    ) -> Result<(), ImageSearchError> {
        self.enqueue(Some(gate), result)
    }
    fn enqueue(
        &self,
        gate: Option<BoxFuture<'static, ()>>,
        result: Result<Vec<RegistryImage>, ImageSearchError>,
    ) -> Result<(), ImageSearchError> {
        self.state
            .lock()
            .map_err(|_| ImageSearchError::Canceled)?
            .results
            .push_back((gate, result));
        Ok(())
    }
    pub fn calls(&self) -> Result<Vec<ImageSearchCall>, ImageSearchError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| ImageSearchError::Canceled)?
            .calls
            .clone())
    }
}
impl ImageSearch for FakeImageSearch {
    fn search(
        &self,
        exec: Arc<dyn HostExec>,
        engine: Engine,
        query: ImageSearchQuery,
        cancel: CancelFlag,
    ) -> ImageSearchFuture<Vec<RegistryImage>> {
        let state = self.state.clone();
        async move {
            let (gate, result) = {
                let mut state = state.lock().map_err(|_| ImageSearchError::Canceled)?;
                state.calls.push(ImageSearchCall {
                    exec,
                    engine,
                    query,
                    cancel,
                });
                state
                    .results
                    .pop_front()
                    .unwrap_or((None, Err(ImageSearchError::Unavailable)))
            };
            if let Some(gate) = gate {
                gate.await;
            }
            result
        }
        .boxed()
    }
}
