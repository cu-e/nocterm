//! Animated indicators, pre-rendered to animated WebP alpha masks and
//! embedded in the binary. Each is tinted to a caller's colour once and
//! cached by the app's asset cache. The image element plays the frames only
//! when it has an id — the frame position is element state — and holds the
//! first frame when the platform asks to reduce motion.

use std::{io::Cursor, sync::Arc};

use gpui_kit::{
    App, Asset, ElementId, Hsla, ImageCacheError, ImageSource, InteractiveElement as _,
    IntoElement, RenderImage, Styled as _, px,
};
use image::{AnimationDecoder as _, Rgba, codecs::webp::WebPDecoder};

/// The thinking indicator's mask: white frames whose alpha is the drawing.
const THINKING: &[u8] = include_bytes!("../assets/animations/thinking.webp");

/// The looping indicator shown beside a working agent's status, drawn in
/// `color`; `id` keeps its frame position across renders.
pub fn thinking_indicator(id: impl Into<ElementId>, color: Hsla) -> impl IntoElement {
    let tint = Tint::from(color);
    let source = ImageSource::Custom(Arc::new(move |window, cx: &mut App| {
        window.use_asset::<ThinkingAnimation>(&tint, cx)
    }));
    gpui_kit::img(source).id(id).flex_shrink_0().size(px(16.))
}

/// A colour as straight 8-bit BGRA, the image cache's pixel layout.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct Tint([u8; 4]);

impl From<Hsla> for Tint {
    fn from(color: Hsla) -> Self {
        let rgba = color.to_rgb();
        let byte = |channel: f32| (channel.clamp(0., 1.) * 255.).round() as u8;
        Self([byte(rgba.b), byte(rgba.g), byte(rgba.r), byte(rgba.a)])
    }
}

/// The thinking animation tinted to one colour.
enum ThinkingAnimation {}

impl Asset for ThinkingAnimation {
    type Source = Tint;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    #[expect(
        clippy::manual_async_fn,
        reason = "an `async fn` future would borrow `App`, which is neither `Send` nor `'static`"
    )]
    fn load(
        tint: Self::Source,
        _: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        async move { tinted(THINKING, tint).map(Arc::new) }
    }
}

/// Decodes the animated mask `bytes` with every pixel painted `tint`,
/// keeping the mask's coverage as alpha.
fn tinted(bytes: &[u8], tint: Tint) -> Result<RenderImage, ImageCacheError> {
    let mut decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(image_error)?;
    decoder
        .set_background_color(Rgba([0, 0, 0, 0]))
        .map_err(image_error)?;
    let [b, g, r, a] = tint.0;
    let mut frames = Vec::new();
    for frame in decoder.into_frames() {
        let mut frame = frame.map_err(image_error)?;
        for pixel in frame.buffer_mut().as_chunks_mut::<4>().0 {
            let coverage = u16::from(pixel[3]) * u16::from(a) / 255;
            *pixel = [b, g, r, coverage as u8];
        }
        frames.push(frame);
    }
    Ok(RenderImage::new(frames))
}

fn image_error(error: image::ImageError) -> ImageCacheError {
    ImageCacheError::Image(Arc::new(error))
}

#[cfg(test)]
mod tests {
    use super::{THINKING, Tint, tinted};

    #[test]
    fn thinking_indicator_animates_in_the_requested_colour() {
        let black = tinted(THINKING, Tint::from(gpui_kit::black())).unwrap();
        assert!(black.frame_count() > 1);
        let pixels = black.as_bytes(0).unwrap().as_chunks::<4>().0.iter();
        assert!(pixels.clone().any(|pixel| pixel[3] > 0));
        assert!(pixels.clone().any(|pixel| pixel[3] == 0));
        assert!(pixels.clone().all(|pixel| pixel[..3] == [0, 0, 0]));

        let white = tinted(THINKING, Tint::from(gpui_kit::white())).unwrap();
        let pixels = white.as_bytes(0).unwrap().as_chunks::<4>().0.iter();
        assert!(pixels.clone().all(|pixel| pixel[..3] == [255, 255, 255]));
    }
}
