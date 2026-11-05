//! Explicit codec dependencies supplied by application composition.
use nocterm_ai::images::{ImageInspector, ImageThumbnailer};
use std::sync::Arc;
#[derive(Clone)]
pub struct ImageDeps {
    inspector: Arc<dyn ImageInspector>,
    thumbnailer: Arc<dyn ImageThumbnailer>,
    available: async_channel::Receiver<()>,
    returned: async_channel::Sender<()>,
}
/// Retained by the actual background work, including after a panel disappears.
pub(crate) struct ImageWorkPermit(async_channel::Sender<()>);
impl Drop for ImageWorkPermit {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}
impl ImageDeps {
    pub fn new(inspector: Arc<dyn ImageInspector>, thumbnailer: Arc<dyn ImageThumbnailer>) -> Self {
        let (returned, available) = async_channel::bounded(4);
        for _ in 0..4 {
            returned.try_send(()).expect("new image work slots");
        }
        Self {
            inspector,
            thumbnailer,
            available,
            returned,
        }
    }
    pub(crate) fn inspector(&self) -> Arc<dyn ImageInspector> {
        self.inspector.clone()
    }
    pub(crate) fn thumbnailer(&self) -> Arc<dyn ImageThumbnailer> {
        self.thumbnailer.clone()
    }
    pub(crate) fn admit_work(&self) -> Option<ImageWorkPermit> {
        self.available.try_recv().ok()?;
        Some(ImageWorkPermit(self.returned.clone()))
    }
    pub(crate) fn work(&self) -> futures::future::BoxFuture<'static, ImageWorkPermit> {
        let (available, returned) = (self.available.clone(), self.returned.clone());
        Box::pin(async move {
            available.recv().await.expect("retained image work slots");
            ImageWorkPermit(returned)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installations_share_a_finite_physical_image_work_budget() {
        let codec = Arc::new(nocterm_ai::testing::ScriptedImageCodec::default());
        let deps = ImageDeps::new(codec.clone(), codec);
        let other = deps.clone();
        let mut permits = (0..4)
            .map(|_| deps.admit_work().unwrap())
            .collect::<Vec<_>>();
        assert!(other.admit_work().is_none());
        permits.pop();
        assert!(other.admit_work().is_some());
    }
    #[test]
    fn queued_decoding_wakes_when_physical_work_finishes_and_cancelled_waiters_release() {
        let codec = Arc::new(nocterm_ai::testing::ScriptedImageCodec::default());
        let deps = ImageDeps::new(codec.clone(), codec);
        let mut permits = (0..4)
            .map(|_| deps.admit_work().unwrap())
            .collect::<Vec<_>>();
        futures::executor::block_on(async {
            let mut cancelled = deps.work();
            assert!(futures::poll!(&mut cancelled).is_pending());
            drop(cancelled);
            let mut waiting = deps.work();
            assert!(futures::poll!(&mut waiting).is_pending());
            permits.pop();
            let released = waiting.await;
            assert!(deps.admit_work().is_none());
            drop(released);
            assert!(deps.admit_work().is_some());
        });
    }
}
