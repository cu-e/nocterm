//! In-memory providers for tests of the host and of the crates that implement providers.

use std::{cell::RefCell, rc::Rc};

use gpui::App;
use nocterm_plugin_api::{Capability, HostError, LocalId};
use nocterm_plugin_model::PluginId;

use crate::{Apply, Observe, Surface};

/// An observation provider that answers what it was told to and counts how often it was read.
pub struct ScriptedObserve<C: Capability> {
    answer: RefCell<Result<C::Value, HostError>>,
    reads: RefCell<usize>,
}

impl<C: Capability> ScriptedObserve<C>
where
    C::Value: Clone,
{
    pub fn new(answer: Result<C::Value, HostError>) -> Rc<Self> {
        Rc::new(Self {
            answer: RefCell::new(answer),
            reads: RefCell::new(0),
        })
    }

    /// What the provider answers from now on.
    pub fn answer_with(&self, answer: Result<C::Value, HostError>) {
        *self.answer.borrow_mut() = answer;
    }

    /// How many times the provider was asked.
    pub fn reads(&self) -> usize {
        *self.reads.borrow()
    }
}

impl<C: Capability> Observe<C> for ScriptedObserve<C>
where
    C::Value: Clone,
{
    fn sample(&self, _: &PluginId, _: &App) -> Result<C::Value, HostError> {
        *self.reads.borrow_mut() += 1;
        self.answer.borrow().clone()
    }
}

/// An effect provider that records every request and answers what it was told to.
pub struct RecordingApply<C: Capability> {
    requests: RefCell<Vec<(PluginId, C::Value)>>,
    answer: RefCell<Result<(), HostError>>,
}

impl<C: Capability> RecordingApply<C>
where
    C::Value: Clone,
{
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            requests: RefCell::new(Vec::new()),
            answer: RefCell::new(Ok(())),
        })
    }

    /// What the provider answers from now on.
    pub fn answer_with(&self, answer: Result<(), HostError>) {
        *self.answer.borrow_mut() = answer;
    }

    /// The requests received so far, oldest first.
    pub fn requests(&self) -> Vec<(PluginId, C::Value)> {
        self.requests.borrow().clone()
    }
}

impl<C: Capability> Apply<C> for RecordingApply<C>
where
    C::Value: Clone,
{
    fn apply(&self, plugin: &PluginId, request: C::Value, _: &mut App) -> Result<(), HostError> {
        self.requests.borrow_mut().push((plugin.clone(), request));
        *self.answer.borrow()
    }
}

/// A surface that answers the same for every contribution until told otherwise.
pub struct ScriptedSurface {
    answer: RefCell<Result<(), HostError>>,
    asked: RefCell<Vec<(PluginId, LocalId)>>,
}

impl ScriptedSurface {
    pub fn new(answer: Result<(), HostError>) -> Rc<Self> {
        Rc::new(Self {
            answer: RefCell::new(answer),
            asked: RefCell::new(Vec::new()),
        })
    }

    pub fn answer_with(&self, answer: Result<(), HostError>) {
        *self.answer.borrow_mut() = answer;
    }

    /// The contributions the host asked about, oldest first.
    pub fn asked(&self) -> Vec<(PluginId, LocalId)> {
        self.asked.borrow().clone()
    }
}

impl Surface for ScriptedSurface {
    fn permit(&self, plugin: &PluginId, local: &LocalId, _: &App) -> Result<(), HostError> {
        self.asked
            .borrow_mut()
            .push((plugin.clone(), local.clone()));
        *self.answer.borrow()
    }
}
