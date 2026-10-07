// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;
use windows::Win32::Foundation::RECT;

/// Metadata describes only pixels that completed rendering and presentation.
#[derive(Default)]
pub(super) struct RetainedFrame {
    snapshot: Option<SceneSnapshot>,
    appearance: Option<WindowBackgroundAppearance>,
    atlas_revision: u64,
}

impl RetainedFrame {
    #[cfg(test)]
    pub(super) fn is_valid(&self) -> bool {
        self.snapshot.is_some()
    }

    pub(super) fn invalidate(&mut self) {
        self.snapshot = None;
        self.appearance = None;
    }

    pub(super) fn damage(
        &self,
        snapshot: &SceneSnapshot,
        appearance: WindowBackgroundAppearance,
        atlas_revision: u64,
        partial_clear_supported: bool,
    ) -> SceneDamage {
        if self.appearance != Some(appearance) || self.atlas_revision != atlas_revision {
            return SceneDamage::Full;
        }
        let Some(previous) = &self.snapshot else {
            return SceneDamage::Full;
        };
        let damage = snapshot.damage_since(previous);
        if matches!(damage, SceneDamage::Partial(_)) && !partial_clear_supported {
            SceneDamage::Full
        } else {
            damage
        }
    }

    pub(super) fn scroll(
        &self,
        snapshot: &SceneSnapshot,
        appearance: WindowBackgroundAppearance,
        atlas_revision: u64,
        partial_clear_supported: bool,
    ) -> Option<SceneScrollPlan> {
        if !partial_clear_supported
            || self.appearance != Some(appearance)
            || self.atlas_revision != atlas_revision
        {
            return None;
        }
        snapshot.scroll_since(self.snapshot.as_ref()?)
    }

    pub(super) fn commit(
        &mut self,
        snapshot: SceneSnapshot,
        appearance: WindowBackgroundAppearance,
        atlas_revision: u64,
    ) {
        self.snapshot = Some(snapshot);
        self.appearance = Some(appearance);
        self.atlas_revision = atlas_revision;
    }
}

pub(super) fn dx_rect(rect: SceneDamageRect) -> RECT {
    RECT {
        left: rect.left as i32,
        top: rect.top as i32,
        right: rect.right as i32,
        bottom: rect.bottom as i32,
    }
}

/// D3D11.1 interfaces alone do not guarantee rectangular ClearView support.
pub(super) fn clear_view_context(devices: &DirectXRendererDevices) -> Option<ID3D11DeviceContext1> {
    let mut options = D3D11_FEATURE_DATA_D3D11_OPTIONS::default();
    unsafe {
        devices
            .device
            .CheckFeatureSupport(
                D3D11_FEATURE_D3D11_OPTIONS,
                &mut options as *mut _ as *mut _,
                std::mem::size_of_val(&options) as u32,
            )
            .ok()?;
    }
    options
        .ClearView
        .as_bool()
        .then(|| devices.device_context.cast().ok())
        .flatten()
}

/// A persistent target avoids depending on flip-model back-buffer contents.
pub(super) fn create_target(
    device: &ID3D11Device,
    swap_chain_target: &ID3D11Texture2D,
    swap_chain_view: Option<ID3D11RenderTargetView>,
) -> Result<(bool, ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
    let attempt = (|| -> Result<_> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { swap_chain_target.GetDesc(&mut desc) };
        desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
        desc.MiscFlags = 0;
        let mut texture = None;
        unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
        let texture = texture.context("creating retained render target")?;
        let mut view = None;
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut view))? };
        anyhow::ensure!(view.is_some(), "creating retained render target view");
        Ok((true, texture, view))
    })();
    match attempt {
        Ok(target) => Ok(target),
        Err(error) => {
            log::warn!(
                "Retained render-target allocation unavailable; using full-window rendering: {error:#}"
            );
            Ok((false, swap_chain_target.clone(), swap_chain_view))
        }
    }
}

impl DirectXRenderer {
    pub(super) fn copy_retained_to_swap_chain(&self) -> Result<()> {
        let context = &self
            .devices
            .as_ref()
            .context("devices missing")?
            .device_context;
        let resources = self.resources.as_ref().context("resources missing")?;
        if !resources.retained_enabled {
            return Ok(());
        }
        let source = resources
            .render_target
            .as_ref()
            .context("retained target missing")?;
        let destination = resources
            .swap_chain_target
            .as_ref()
            .context("swap-chain target missing")?;
        unsafe {
            // Unbind before copying. Every subsequent render explicitly restores
            // targets, viewport and rasterizer state on the shared context.
            context.OMSetRenderTargets(None, None);
            context.CopyResource(destination, source);
        }
        Ok(())
    }
}
