// Nocterm modifications, licensed under the upstream Apache-2.0 license.
//! Linux-only persistent image. Presentation still copies every acquired frame.
use super::*;
use gpui::{ReusableSceneSnapshot, SceneComparisonMemo, SceneDamage, SceneDamageRect};

mod allocation;
#[cfg(test)]
mod tests;

const MAX_RETAINED_BYTES: u64 = 64 * 1024 * 1024;

fn surface_usage(opt_in: Option<&std::ffi::OsStr>, copy_supported: bool) -> wgpu::TextureUsages {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT;
    if opt_in == Some(std::ffi::OsStr::new("1")) && copy_supported {
        usage |= wgpu::TextureUsages::COPY_DST;
    }
    usage
}

pub(super) fn requested_surface_usage(
    caps: &wgpu::SurfaceCapabilities,
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
) -> wgpu::TextureUsages {
    surface_usage(
        std::env::var_os("NOCTERM_EXPERIMENTAL_LINUX_RETAINED_RENDERER").as_deref(),
        surface_copy_supported(caps, adapter, format),
    )
}

pub(super) fn surface_copy_supported(
    caps: &wgpu::SurfaceCapabilities,
    adapter: &wgpu::Adapter,
    format: wgpu::TextureFormat,
) -> bool {
    supported_format(format)
        && caps.usages.contains(wgpu::TextureUsages::COPY_DST)
        && adapter
            .get_texture_format_features(format)
            .allowed_usages
            .contains(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC)
}

fn supported_format(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm
            | wgpu::TextureFormat::Rgba8Unorm
            | wgpu::TextureFormat::Bgra8UnormSrgb
            | wgpu::TextureFormat::Rgba8UnormSrgb
    )
}

fn bounded_size(size: Size<DevicePixels>, limit: u32) -> bool {
    size.width.0 > 0
        && size.height.0 > 0
        && size.width.0 as u32 <= limit
        && size.height.0 as u32 <= limit
        && (size.width.0 as u64)
            .checked_mul(size.height.0 as u64)
            .and_then(|pixels| pixels.checked_mul(4))
            .is_some_and(|bytes| bytes <= MAX_RETAINED_BYTES)
}

struct RetainedImage {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    clear: allocation::OverwriteClear,
}

#[derive(Default)]
pub(super) struct RetainedFrame {
    image: Option<RetainedImage>,
    snapshot: Option<ReusableSceneSnapshot>,
    spare: Option<ReusableSceneSnapshot>,
    comparison: SceneComparisonMemo,
    atlas_revision: u64,
    premultiplied_alpha: bool,
    clear_color: Option<wgpu::Color>,
    is_bgr: bool,
    disabled: bool,
    #[cfg(test)]
    last_damage: Option<SceneDamage>,
    #[cfg(test)]
    mutate_atlas_before_commit: bool,
}

impl RetainedFrame {
    pub(super) fn invalidate(&mut self) {
        self.snapshot = None;
        self.spare = None;
        self.comparison = SceneComparisonMemo::default();
        self.clear_color = None;
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn disable(&mut self) {
        self.invalidate();
        self.image = None;
        self.disabled = true;
    }

    fn eligible(&self, size: Size<DevicePixels>, format: wgpu::TextureFormat, limit: u32) -> bool {
        !self.disabled && supported_format(format) && bounded_size(size, limit)
    }

    fn ensure(
        &mut self,
        device: &wgpu::Device,
        size: Size<DevicePixels>,
        format: wgpu::TextureFormat,
        limit: u32,
    ) -> Result<bool> {
        if !self.eligible(size, format, limit) {
            self.image = None;
            self.invalidate();
            return Ok(false);
        }
        if self.image.as_ref().is_some_and(|image| {
            image.texture.width() == size.width.0 as u32
                && image.texture.height() == size.height.0 as u32
                && image.texture.format() == format
        }) {
            return Ok(true);
        }
        // Release the previous owned target before allocation. In-flight commands
        // retain their normal WGPU resource references until GPU completion.
        self.image = None;
        self.invalidate();
        self.image = Some(allocation::create_image(device, size, format)?);
        Ok(true)
    }

    fn damage(
        &mut self,
        next: &ReusableSceneSnapshot,
        revision: u64,
        premultiplied: bool,
        clear: wgpu::Color,
        is_bgr: bool,
    ) -> SceneDamage {
        if self.atlas_revision != revision
            || self.premultiplied_alpha != premultiplied
            || self.clear_color != Some(clear)
            || self.is_bgr != is_bgr
        {
            return SceneDamage::Full;
        }
        match self.snapshot.as_ref() {
            Some(old) => next.damage_since(old, &mut self.comparison),
            None => SceneDamage::Full,
        }
    }
}

impl WgpuRendererCore {
    pub(super) fn render_retained_surface(
        &mut self,
        scene: &Scene,
        destination: &wgpu::Texture,
        size: Size<DevicePixels>,
        premultiplied: bool,
        clear: wgpu::Color,
    ) -> Result<wgpu::SubmissionIndex> {
        if size.width.0 <= 0
            || size.height.0 <= 0
            || destination.width() != size.width.0 as u32
            || destination.height() != size.height.0 as u32
            || destination.format() != self.target_format
        {
            self.retained.invalidate();
            anyhow::bail!("retained destination does not match frame size/format");
        }
        if !destination.usage().contains(wgpu::TextureUsages::COPY_DST) {
            self.retained.invalidate();
            return self.render_frame(
                scene,
                &destination.create_view(&Default::default()),
                size,
                premultiplied,
                clear,
            );
        }
        let (available, submission) =
            self.render_retained_to(scene, size, premultiplied, clear, Some(destination))?;
        if !available {
            return self.render_frame(
                scene,
                &destination.create_view(&Default::default()),
                size,
                premultiplied,
                clear,
            );
        }
        submission.context("retained presentation was not submitted")
    }

    pub(super) fn encode_present_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Texture,
    ) -> Result<()> {
        let image = self
            .retained
            .image
            .as_ref()
            .context("retained copy target missing")?;
        encoder.copy_texture_to_texture(
            image.texture.as_image_copy(),
            destination.as_image_copy(),
            wgpu::Extent3d {
                width: image.texture.width(),
                height: image.texture.height(),
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    /// Returns false when retention is unavailable; caller uses original full rendering.
    #[cfg(test)]
    pub(super) fn render_retained(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
        premultiplied: bool,
        clear: wgpu::Color,
    ) -> Result<bool> {
        Ok(self
            .render_retained_to(scene, size, premultiplied, clear, None)?
            .0)
    }

    fn render_retained_to(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
        premultiplied: bool,
        clear: wgpu::Color,
        presentation: Option<&wgpu::Texture>,
    ) -> Result<(bool, Option<wgpu::SubmissionIndex>)> {
        if !self
            .retained
            .eligible(size, self.target_format, self.max_texture_size)
        {
            self.retained.image = None;
            self.retained.invalidate();
            return Ok((false, None));
        }
        // Flush even unchanged scenes. Additive writes can change neighboring sampled texels.
        let revision = self.atlas.prepare_revision();
        let viewport = [size.width.0 as u32, size.height.0 as u32];
        if self
            .retained
            .snapshot
            .as_ref()
            .is_some_and(|old| old.viewport() != viewport)
            || self.retained.image.as_ref().is_some_and(|image| {
                image.texture.width() != viewport[0]
                    || image.texture.height() != viewport[1]
                    || image.texture.format() != self.target_format
            })
        {
            self.retained.invalidate();
        }
        // A local prepared buffer survives image allocation's invalidation.
        // The spare slot is empty until a successful frame commits below.
        let snapshot = ReusableSceneSnapshot::capture(scene, viewport, self.retained.spare.take());
        if !snapshot.supports_partial_updates() {
            self.retained.invalidate();
            #[cfg(test)]
            {
                self.retained.last_damage = None;
            }
            return Ok((false, None));
        }
        let available = match self.retained.ensure(
            &self.resources.device,
            size,
            self.target_format,
            self.max_texture_size,
        ) {
            Ok(available) => available,
            Err(error) => {
                log::warn!("Retained image unavailable; using full rendering: {error:#}");
                self.retained.disable();
                false
            }
        };
        if !available {
            return Ok((false, None));
        }
        let damage = self
            .retained
            .damage(&snapshot, revision, premultiplied, clear, self.is_bgr);
        // An exact whole-viewport repair uses the original attachment clear.
        // There is no area heuristic: all smaller conservative rectangles stay partial.
        let damage = match damage {
            SceneDamage::Partial(rect)
                if rect.left == 0
                    && rect.top == 0
                    && rect.right == size.width.0 as u32
                    && rect.bottom == size.height.0 as u32 =>
            {
                SceneDamage::Full
            }
            damage => damage,
        };
        #[cfg(test)]
        {
            self.retained.last_damage = Some(damage);
        }
        let mut submission = None;
        if damage != SceneDamage::Unchanged {
            let image = self
                .retained
                .image
                .as_ref()
                .context("retained target missing")?;
            let view = image.view.clone();
            let color = [
                clear.r as f32,
                clear.g as f32,
                clear.b as f32,
                clear.a as f32,
            ];
            if matches!(damage, SceneDamage::Partial(_)) {
                self.resources.queue.write_buffer(
                    &image.clear.uniform,
                    0,
                    bytemuck::cast_slice(&color),
                );
            }
            let partial = match damage {
                SceneDamage::Partial(rect) => Some(rect),
                _ => None,
            };
            submission = Some(
                match self.render_frame_prepared(
                    scene,
                    &view,
                    size,
                    premultiplied,
                    clear,
                    partial,
                    presentation,
                ) {
                    Ok(submission) => submission,
                    Err(error) => {
                        self.retained.invalidate();
                        return Err(error);
                    }
                },
            );
        } else if let Some(destination) = presentation {
            let mut encoder =
                self.resources
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("unchanged_retained_present"),
                    });
            self.encode_present_copy(&mut encoder, destination)?;
            submission = Some(self.resources.queue.submit([encoder.finish()]));
        }
        #[cfg(test)]
        if std::mem::take(&mut self.retained.mutate_atlas_before_commit) {
            self.atlas.clear();
        }
        if self.atlas.content_revision() == revision {
            self.retained.spare = self.retained.snapshot.replace(snapshot);
            self.retained.atlas_revision = revision;
            self.retained.premultiplied_alpha = premultiplied;
            self.retained.clear_color = Some(clear);
            self.retained.is_bgr = self.is_bgr;
        } else {
            self.retained.invalidate();
        }
        Ok((true, submission))
    }

    pub(super) fn clear_damage<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        rect: SceneDamageRect,
        _color: wgpu::Color,
    ) {
        let image = self
            .retained
            .image
            .as_ref()
            .expect("partial frame requires retained image");
        pass.set_scissor_rect(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
        pass.set_pipeline(&image.clear.pipeline);
        pass.set_bind_group(0, &image.clear.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}
