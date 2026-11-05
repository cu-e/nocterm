//! Images and embedded fallbacks are shared presentation, never raw fact acquisition.
use gpui_kit::{Image, ImageFormat};
use nocterm_connections_model::{
    observation::{CountryCode, FlagBytes},
    profiles::Profile,
};
use nocterm_connections_store::FlagState;
use std::{cell::RefCell, collections::HashMap, sync::Arc};
#[derive(Default)]
pub(crate) struct FactsPresentation {
    images: RefCell<HashMap<CountryCode, FlagImage>>,
    icons: RefCell<HashMap<(&'static str, u32), Arc<Image>>>,
}
impl FactsPresentation {
    pub(crate) fn flag(&self, code: CountryCode, state: Option<FlagState>) -> Option<Arc<Image>> {
        let mut images = self.images.borrow_mut();
        let source = match state {
            Some(FlagState::Ready(bytes)) => Some(bytes),
            Some(FlagState::Unavailable) => None,
            // A new attempt must not reuse its previous result or fallback.
            None | Some(FlagState::Requested | FlagState::Loading) => {
                images.remove(&code);
                return None;
            }
        };
        if let Some(cached) = images.get(&code)
            && cached.same_source(source.as_ref())
        {
            return cached.image.clone();
        }
        let bytes = source.as_ref().map_or_else(
            || crate::geo::builtin_flag(code.as_str()),
            |bytes| Some(bytes.bytes().to_vec()),
        );
        let byte_cost = bytes.as_ref().map_or(0, Vec::len)
            + source.as_ref().map_or(0, |bytes| bytes.bytes().len());
        // Installation-scoped, at most 64 entries and 4 MiB of retained SVG bytes.
        // Clearing the small cache on saturation also evicts obsolete country choices.
        images.remove(&code);
        if images.len() >= 64
            || images.values().map(|image| image.byte_cost).sum::<usize>() + byte_cost
                > 4 * 1024 * 1024
        {
            images.clear();
        }
        let image = bytes.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
        images.insert(
            code,
            FlagImage {
                source,
                image: image.clone(),
                byte_cost,
            },
        );
        image
    }
    pub(crate) fn icon(
        &self,
        profile: &Profile,
        detected: Option<&'static crate::os::Os>,
    ) -> Option<Arc<Image>> {
        let os = profile
            .icon
            .as_deref()
            .and_then(crate::os::find)
            .or(detected)?;
        let color = profile
            .icon_color
            .as_deref()
            .filter(|color| crate::os::is_valid_color(color))
            .unwrap_or(os.color);
        let rgb = crate::os::rgb(color)?;
        let mut icons = self.icons.borrow_mut();
        let key = (os.id, rgb);
        if let Some(image) = icons.get(&key) {
            return Some(image.clone());
        }
        // OS catalog identity and validated RGB are the complete rendering inputs.
        if icons.len() >= 256 {
            icons.clear();
        }
        let image = Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            crate::os::svg(os, color),
        ));
        icons.insert(key, image.clone());
        Some(image)
    }
}
struct FlagImage {
    // Retaining the source makes pointer equality safe even after raw-cache eviction.
    source: Option<FlagBytes>,
    image: Option<Arc<Image>>,
    byte_cost: usize,
}
impl FlagImage {
    fn same_source(&self, source: Option<&FlagBytes>) -> bool {
        match (self.source.as_ref(), source) {
            (Some(old), Some(new)) => std::ptr::eq(old.bytes(), new.bytes()),
            (None, None) => true,
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests;
