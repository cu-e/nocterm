// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;

impl CosmicTextSystem {
    /// Returns the selected face's weight and style, which may differ from the request.
    pub fn font_weight_and_style(
        &self,
        font_id: FontId,
    ) -> Result<(gpui::FontWeight, gpui::FontStyle)> {
        let state = self.0.read();
        let font = state
            .loaded_fonts
            .get(font_id.0)
            .context("invalid font ID")?;
        let face = state
            .font_system
            .db()
            .face(font.font.id())
            .context("font face not found")?;
        let style = match face.style {
            cosmic_text::Style::Normal => gpui::FontStyle::Normal,
            cosmic_text::Style::Italic => gpui::FontStyle::Italic,
            cosmic_text::Style::Oblique => gpui::FontStyle::Oblique,
        };
        Ok((gpui::FontWeight(face.weight.0 as f32), style))
    }

    /// Builds reports for unresolved source indices after an outer text system
    /// has applied its own fallback.
    pub fn missing_glyphs(
        &self,
        text: &str,
        font_runs: &[FontRun],
        missing_text_indices: impl IntoIterator<Item = usize>,
    ) -> Vec<MissingGlyph> {
        self.0
            .read()
            .missing_glyphs(text, font_runs, missing_text_indices)
    }

    pub fn new(system_font_fallback: &str) -> Self {
        let font_system = FontSystem::new();

        Self(RwLock::new(CosmicTextSystemState {
            font_system,
            scratch: ShapeBuffer::default(),
            swash_scale_context: ScaleContext::new(),
            pending_glyph_images: HashMap::default(),
            loaded_fonts: Vec::new(),
            loaded_font_ids_by_key: HashMap::default(),
            font_ids_by_family_cache: HashMap::default(),
            system_font_fallback: system_font_fallback.to_string(),
            missing_glyph_sink: None,
        }))
    }

    pub fn new_without_system_fonts(system_font_fallback: &str) -> Self {
        let font_system = FontSystem::new_with_locale_and_db(
            "en-US".to_string(),
            cosmic_text::fontdb::Database::new(),
        );

        Self(RwLock::new(CosmicTextSystemState {
            font_system,
            scratch: ShapeBuffer::default(),
            swash_scale_context: ScaleContext::new(),
            pending_glyph_images: HashMap::default(),
            loaded_fonts: Vec::new(),
            loaded_font_ids_by_key: HashMap::default(),
            font_ids_by_family_cache: HashMap::default(),
            system_font_fallback: system_font_fallback.to_string(),
            missing_glyph_sink: None,
        }))
    }
}
