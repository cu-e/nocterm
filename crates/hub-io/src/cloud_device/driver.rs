//! Runs device core flows: hands every effect a flow asks for to the [`EffectRunner`] and
//! passes each answer and frame back, in arrival order, until the flow ends.
use super::effects::{EffectRunner, Event};
use futures::stream::{BoxStream, SelectAll, StreamExt as _};
use nocterm_cloud_device::CloudDeviceError;
use nocterm_cloud_device::flow::{Effect, EffectId, Flow, Frame, Step};
use std::sync::Arc;
use std::time::Duration;

/// The wall clock the flows are given.
pub(crate) fn now_ms() -> i64 {
    nocterm_hub_client::now_ms()
}

/// Whether `event` is the last one of its effect: an answer, a page end or a gap.
fn terminal(event: &Event) -> bool {
    match event {
        Event::Input(_) => true,
        Event::Frame(frame) => matches!(frame, Frame::PageEnd { .. } | Frame::Gap { .. }),
    }
}

/// Answers `event` to `flow`.
pub(crate) fn answer<F: Flow>(flow: &mut F, id: EffectId, event: Event) {
    match event {
        Event::Input(input) => flow.resolve(id, input),
        Event::Frame(frame) => flow.frame(id, frame),
    }
}

/// The effects in flight of several flows, each tagged with the flow it belongs to.
pub(crate) struct Effects<T> {
    runner: Arc<dyn EffectRunner>,
    streams: SelectAll<BoxStream<'static, (T, EffectId, Event)>>,
}
impl<T: Copy + Send + 'static> Effects<T> {
    pub(crate) fn new(runner: Arc<dyn EffectRunner>) -> Self {
        Self {
            runner,
            streams: SelectAll::new(),
        }
    }
    /// Starts `effect`; its events come back from [`Self::next`] tagged with `tag` and `id`.
    pub(crate) fn start(&mut self, tag: T, id: EffectId, effect: Effect) {
        let events = self
            .runner
            .run(effect)
            .scan(false, |ended, event| {
                if *ended {
                    return futures::future::ready(None);
                }
                *ended = terminal(&event);
                futures::future::ready(Some(event))
            })
            .map(move |event| (tag, id, event));
        self.streams.push(events.boxed());
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
    /// The next event of any effect; pending forever when none is in flight.
    pub(crate) async fn next(&mut self) -> (T, EffectId, Event) {
        loop {
            match self.streams.next().await {
                Some(event) => return event,
                None => futures::future::pending::<()>().await,
            }
        }
    }
}

/// Sleeps until `until_ms`, or forever when it is `None`.
pub(crate) async fn sleep_until(until_ms: Option<i64>) {
    match until_ms {
        Some(until) => {
            let wait = until.saturating_sub(now_ms()).max(0);
            tokio::time::sleep(Duration::from_millis(u64::try_from(wait).unwrap_or(0))).await;
        }
        None => futures::future::pending::<()>().await,
    }
}

/// What the output handler of [`run`] decides.
pub(crate) enum Control {
    Continue,
    Stop,
}

/// Runs `flow` until it is done, fails or `output` stops it.
pub(crate) async fn run<F: Flow>(
    flow: &mut F,
    runner: Arc<dyn EffectRunner>,
    mut output: impl FnMut(&mut F, F::Output) -> Control,
) -> Result<(), CloudDeviceError> {
    let mut effects = Effects::<()>::new(runner);
    loop {
        match flow.next(now_ms()) {
            Step::Effect(id, effect) => effects.start((), id, effect),
            Step::Output(value) => {
                if let Control::Stop = output(flow, value) {
                    return Ok(());
                }
            }
            Step::Idle { until_ms } => {
                if effects.is_empty() && until_ms.is_none() {
                    return Err(CloudDeviceError::Protocol);
                }
                tokio::select! {
                    ((), id, event) = effects.next() => answer(flow, id, event),
                    () = sleep_until(until_ms) => {}
                }
            }
            Step::Done => return Ok(()),
            Step::Failed(error) => return Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;
    use nocterm_cloud_device::context::AccountKeys;
    use nocterm_cloud_device::discover::{Discover, Discovery};
    use nocterm_cloud_device::flow::{EffectFailure, Input};
    use std::sync::Mutex;

    /// Answers every listing with no collections and refuses everything else.
    #[derive(Default)]
    struct Scripted(Mutex<Vec<String>>);
    impl EffectRunner for Scripted {
        fn run(&self, effect: Effect) -> BoxStream<'static, Event> {
            let answer = match &effect {
                Effect::ListCollections { kind } => {
                    self.0.lock().unwrap().push(kind.clone());
                    Event::Input(Input::Collections(Vec::new()))
                }
                _ => Event::Input(Input::Failed(EffectFailure::Refused)),
            };
            stream::once(async move { answer }).boxed()
        }
    }

    fn keys() -> Arc<AccountKeys> {
        Arc::new(AccountKeys::new(
            "account",
            nocterm_keys::AccountKey::generate().unwrap(),
            nocterm_keys::AccountKeyPair::generate().unwrap(),
        ))
    }

    #[tokio::test]
    async fn a_flow_runs_to_its_output_through_the_effects_it_asks_for() {
        let runner = Arc::new(Scripted::default());
        let mut discover = Discover::new(keys(), "hub", Some("hub.test".into()));
        let mut found = None;
        run(&mut discover, runner.clone(), |_, discovery| {
            found = Some(discovery);
            Control::Stop
        })
        .await
        .unwrap();
        assert!(matches!(found, Some(Discovery::NotSetUp)));
        assert_eq!(runner.0.lock().unwrap().as_slice(), ["cloud-hub-trust"]);
    }

    /// A stream that goes on after its page end.
    struct Chatty;
    impl EffectRunner for Chatty {
        fn run(&self, _: Effect) -> BoxStream<'static, Event> {
            stream::iter([
                Event::Frame(Frame::PageEnd {
                    through_cursor: 0,
                    has_more: false,
                }),
                Event::Input(Input::Failed(EffectFailure::Ended)),
            ])
            .boxed()
        }
    }

    #[tokio::test]
    async fn nothing_of_an_effect_reaches_its_flow_after_its_last_event() {
        let mut effects = Effects::new(Arc::new(Chatty));
        effects.start(7u32, 1, Effect::ListInstances);
        let (tag, id, event) = effects.next().await;
        assert_eq!((tag, id), (7, 1));
        assert!(matches!(event, Event::Frame(Frame::PageEnd { .. })));
        let more = tokio::time::timeout(Duration::from_millis(50), effects.next()).await;
        assert!(more.is_err(), "the answer after the page end is dropped");
    }
}
