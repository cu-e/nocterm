//! A subscription owns its original capture before first poll and through cache I/O.
use crate::{CloudChat, CloudChatError};
use gpui::{Context, Task};
use nocterm_cloud_model::projection::ports::CapturedConversationSource;
use std::sync::Arc;

pub(crate) struct ReplaySubscription(Arc<dyn CapturedConversationSource>);
impl ReplaySubscription {
    pub(crate) fn new(source: Arc<dyn CapturedConversationSource>) -> Self {
        Self(source)
    }
    pub(crate) fn start(self, cx: &mut Context<CloudChat>) -> Task<()> {
        cx.spawn(async move |chat, cx| {
            loop {
                if let Err(error) = self.0.check_current() {
                    let _ = chat.update(cx, |chat, cx| chat.fail(error.into(), cx));
                    return;
                }
                let prepared = chat.update(cx, |chat, cx| chat.prepare_replay(cx));
                let (cursor, allocation) = match prepared {
                    Ok(Ok(value)) => value,
                    Ok(Err(error)) => {
                        let _ = chat.update(cx, |chat, cx| chat.fail(error, cx));
                        return;
                    }
                    Err(_) => return,
                };
                let result = self.0.next(cursor, allocation).await;
                let result = self.0.check_current().and(result);
                let published = chat.update(cx, |chat, cx| match result {
                    Ok(batch) => chat.accept_page(batch, cx),
                    Err(error) => Err(CloudChatError::from(error)),
                });
                match published {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        let _ = chat.update(cx, |chat, cx| chat.fail(error, cx));
                        return;
                    }
                    Err(_) => return,
                }
            }
        })
    }
}
