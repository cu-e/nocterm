//! Bounded unpublished coalescing; published chunks still await the host's ACK.
use nocterm_cloud_model::{
    engine::{EngineError, EngineEvent, EngineEventSink, EngineText, TextChannel},
    identity::StepId,
    payload::MAX_PROGRESS_BYTES,
};
use rig_core::streaming::{Item, StreamEvent};
use std::sync::Arc;

pub(crate) struct StreamFragment {
    bytes: usize,
    text: Option<(TextChannel, String)>,
}
impl StreamFragment {
    pub(crate) fn text(channel: TextChannel, text: String) -> Self {
        Self {
            bytes: text.len(),
            text: Some((channel, text)),
        }
    }
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }
}
impl From<Item<StreamEvent>> for StreamFragment {
    fn from(value: Item<StreamEvent>) -> Self {
        match value {
            Item::Event(StreamEvent::Text { text, .. }) => Self::text(TextChannel::Assistant, text),
            Item::Event(StreamEvent::Reasoning { text, .. }) => {
                Self::text(TextChannel::Thought, text)
            }
            Item::Event(StreamEvent::Arguments { json, .. }) => Self {
                bytes: json.len(),
                text: None,
            },
            _ => Self {
                bytes: 0,
                text: None,
            },
        }
    }
}

pub(crate) struct ProgressBuffer {
    step: StepId,
    sink: Arc<dyn EngineEventSink>,
    pending: Option<(TextChannel, String)>,
}
impl ProgressBuffer {
    pub(crate) fn new(step: StepId, sink: Arc<dyn EngineEventSink>) -> Self {
        Self {
            step,
            sink,
            pending: None,
        }
    }
    pub(crate) async fn push(&mut self, fragment: StreamFragment) -> Result<(), EngineError> {
        let Some((channel, text)) = fragment.text else {
            return Ok(());
        };
        if self
            .pending
            .as_ref()
            .is_some_and(|(original, _)| *original != channel)
        {
            self.flush().await?;
        }
        let mut rest = text.as_str();
        while !rest.is_empty() {
            let held = self.pending.as_ref().map_or(0, |(_, text)| text.len());
            let mut end = rest.len().min(MAX_PROGRESS_BYTES - held);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            if end == 0 {
                self.flush().await?;
                continue;
            }
            self.pending
                .get_or_insert_with(|| (channel, String::with_capacity(MAX_PROGRESS_BYTES)))
                .1
                .push_str(&rest[..end]);
            rest = &rest[end..];
            if self
                .pending
                .as_ref()
                .is_some_and(|(_, text)| text.len() == MAX_PROGRESS_BYTES)
            {
                self.flush().await?;
            }
        }
        Ok(())
    }
    pub(crate) async fn flush(&mut self) -> Result<(), EngineError> {
        if let Some((channel, text)) = self.pending.take() {
            self.sink
                .publish(
                    self.step,
                    EngineEvent::Text(EngineText::new(channel, text)?),
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_cloud_model::testing::ScriptedEngineEvents;

    #[tokio::test]
    async fn unpublished_fragments_coalesce_without_acknowledging_unsaved_events() {
        let sink = Arc::new(ScriptedEngineEvents::default());
        let mut buffer = ProgressBuffer::new(StepId::from_bytes([1; 16]), sink.clone());
        for _ in 0..200 {
            buffer
                .push(StreamFragment::text(TextChannel::Assistant, "я".into()))
                .await
                .unwrap();
        }
        assert!(sink.events().is_empty());
        let (entered, release) = sink.pause_next();
        let flush = buffer.flush();
        tokio::pin!(flush);
        tokio::select! { result=&mut flush=>panic!("host ACK must wait: {result:?}"), _=entered.wait()=>{} }
        release.release();
        flush.await.unwrap();
        let events = sink.events();
        let EngineEvent::Text(text) = &events[0] else {
            panic!("text");
        };
        assert_eq!(events.len(), 1);
        assert_eq!(text.text(), "я".repeat(200));
    }

    #[tokio::test]
    async fn coalescing_keeps_channels_ordered_and_unicode_chunks_bounded() {
        let sink = Arc::new(ScriptedEngineEvents::default());
        let mut buffer = ProgressBuffer::new(StepId::from_bytes([1; 16]), sink.clone());
        let text = "я".repeat(20_000);
        buffer
            .push(StreamFragment::text(TextChannel::Assistant, text.clone()))
            .await
            .unwrap();
        buffer
            .push(StreamFragment::text(TextChannel::Thought, "thought".into()))
            .await
            .unwrap();
        buffer.flush().await.unwrap();
        let events = sink.events();
        let fragments = events
            .iter()
            .filter_map(|event| match event {
                EngineEvent::Text(text) => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(
            fragments
                .iter()
                .all(|text| text.text().len() <= MAX_PROGRESS_BYTES)
        );
        assert_eq!(
            fragments
                .iter()
                .filter(|text| text.channel() == TextChannel::Assistant)
                .map(|text| text.text())
                .collect::<String>(),
            text
        );
        assert_eq!(fragments.last().unwrap().channel(), TextChannel::Thought);
    }
}
