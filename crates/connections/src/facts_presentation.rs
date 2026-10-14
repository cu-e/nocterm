//! Images and embedded fallbacks are shared presentation, never raw fact acquisition.
use gpui_kit::{Image, ImageFormat};
use nocterm_connections_model::{observation::CountryCode, profiles::Profile};
use nocterm_connections_store::FlagState;
use std::{cell::RefCell, collections::HashMap, sync::Arc};
#[derive(Default)]
pub(crate) struct FactsPresentation {
    images: RefCell<HashMap<CountryCode, Option<Arc<Image>>>>,
}
impl FactsPresentation {
    pub(crate) fn flag(&self, code: CountryCode, state: Option<FlagState>) -> Option<Arc<Image>> {
        let state = state?;
        if matches!(state, FlagState::Requested | FlagState::Loading) {
            return None;
        }
        if let Some(image) = self.images.borrow().get(&code) {
            return image.clone();
        }
        let bytes = match state {
            FlagState::Ready(bytes) => Some(bytes.bytes().to_vec()),
            FlagState::Unavailable => crate::geo::builtin_flag(code.as_str()),
            FlagState::Requested | FlagState::Loading => None,
        };
        let image = bytes.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Svg, bytes)));
        self.images.borrow_mut().insert(code, image.clone());
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
        Some(Arc::new(Image::from_bytes(
            ImageFormat::Svg,
            crate::os::svg(os, color),
        )))
    }
}
#[cfg(test)]
mod tests;
