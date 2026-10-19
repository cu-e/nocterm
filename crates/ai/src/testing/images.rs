//! Byte-keyed supplied image evidence; never decodes or infers successful evidence.
use crate::images::{
    EncodedPreview, ImageCodecError, ImageFacts, ImageInspector, ImageThumbnailer, InspectedImage,
    PreviewBounds,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
#[derive(Clone, Debug)]
pub enum ImageRequest {
    Inspect(Arc<[u8]>),
    Thumbnail(Arc<[u8]>, PreviewBounds),
}
#[derive(Default)]
struct Script {
    inspections: HashMap<Vec<u8>, Result<ImageFacts, ImageCodecError>>,
    previews: HashMap<Vec<u8>, Result<EncodedPreview, ImageCodecError>>,
    requests: Vec<ImageRequest>,
    managed_requests: Vec<(usize, usize)>,
    replaced: HashSet<Vec<u8>>,
}
#[derive(Clone, Default)]
pub struct ScriptedImageCodec {
    script: Arc<Mutex<Script>>,
}
impl ScriptedImageCodec {
    pub fn inspection(&self, bytes: &[u8], result: Result<ImageFacts, ImageCodecError>) {
        self.script
            .lock()
            .expect("image script")
            .inspections
            .insert(bytes.to_vec(), result);
    }
    pub fn preview(&self, bytes: &[u8], result: Result<EncodedPreview, ImageCodecError>) {
        self.script
            .lock()
            .expect("image script")
            .previews
            .insert(bytes.to_vec(), result);
    }
    pub fn replace_source(&self, bytes: &[u8]) {
        self.script
            .lock()
            .expect("image script")
            .replaced
            .insert(bytes.to_vec());
    }
    pub fn requests(&self) -> Vec<ImageRequest> {
        self.script.lock().expect("image script").requests.clone()
    }
    /// Observation only; these scalars cannot resurrect or authorize an image.
    pub fn managed_requests(&self) -> Vec<(usize, usize)> {
        self.script
            .lock()
            .expect("image script")
            .managed_requests
            .clone()
    }
    pub fn clear_managed_requests(&self) {
        self.script
            .lock()
            .expect("image script")
            .managed_requests
            .clear();
    }
}
impl ImageInspector for ScriptedImageCodec {
    fn inspect_managed(
        &self,
        bytes: &crate::images::managed::ManagedImageBytes,
        _ledger: &crate::model::json::deferred::DecodeLedger,
    ) -> Result<ImageFacts, ImageCodecError> {
        let mut script = self.script.lock().expect("image script");
        script
            .managed_requests
            .push((bytes.bytes().as_ptr() as usize, bytes.bytes().len()));
        if script.replaced.contains(bytes.bytes()) {
            return Err(ImageCodecError::InvalidEvidence);
        }
        script
            .inspections
            .get(bytes.bytes())
            .cloned()
            .unwrap_or(Err(ImageCodecError::InvalidEvidence))
    }
    fn inspect(&self, bytes: Arc<[u8]>) -> Result<InspectedImage, ImageCodecError> {
        let (result, replaced) = {
            let mut script = self.script.lock().expect("image script");
            script.requests.push(ImageRequest::Inspect(bytes.clone()));
            (
                script
                    .inspections
                    .get(bytes.as_ref())
                    .cloned()
                    .unwrap_or(Err(ImageCodecError::InvalidEvidence)),
                script.replaced.contains(bytes.as_ref()),
            )
        };
        let bytes = if replaced {
            Arc::from(bytes.as_ref())
        } else {
            bytes
        };
        InspectedImage::new(bytes, result?)
    }
}
impl ImageThumbnailer for ScriptedImageCodec {
    fn thumbnail(
        &self,
        bytes: Arc<[u8]>,
        bounds: PreviewBounds,
    ) -> Result<EncodedPreview, ImageCodecError> {
        let mut script = self.script.lock().expect("image script");
        script
            .requests
            .push(ImageRequest::Thumbnail(bytes.clone(), bounds));
        script
            .previews
            .get(bytes.as_ref())
            .cloned()
            .unwrap_or(Err(ImageCodecError::InvalidEvidence))
    }
}
/// Pure supplied header fixture. This is not physically decoded image evidence.
pub fn png_header_fixture(width: u32, height: u32, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length.max(33)];
    bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
    bytes[12..16].copy_from_slice(b"IHDR");
    bytes[16..20].copy_from_slice(&width.to_be_bytes());
    bytes[20..24].copy_from_slice(&height.to_be_bytes());
    bytes
}
