//! Explicit codec dependencies supplied by application composition.
use nocterm_ai::images::{ImageInspector, ImageThumbnailer};
use std::sync::Arc;
#[derive(Clone)]
pub struct ImageDeps {
    inspector: Arc<dyn ImageInspector>,
    thumbnailer: Arc<dyn ImageThumbnailer>,
}
impl ImageDeps {
    pub fn new(inspector: Arc<dyn ImageInspector>, thumbnailer: Arc<dyn ImageThumbnailer>) -> Self {
        Self {
            inspector,
            thumbnailer,
        }
    }
    pub(crate) fn inspector(&self) -> Arc<dyn ImageInspector> {
        self.inspector.clone()
    }
    pub(crate) fn thumbnailer(&self) -> Arc<dyn ImageThumbnailer> {
        self.thumbnailer.clone()
    }
}
