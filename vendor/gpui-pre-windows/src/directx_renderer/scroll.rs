// Nocterm modifications, licensed under the upstream Apache-2.0 license.
//! Execute verified copies through a scratch texture, then repair disjoint strips.
use super::*;

impl DirectXRenderer {
    fn ensure_scroll_scratch(&mut self) -> Result<()> {
        let resources = self.resources.as_mut().context("resources missing")?;
        anyhow::ensure!(
            !resources.scroll_scratch_failed,
            "scroll scratch previously unavailable"
        );
        if resources.scroll_scratch.is_some() {
            return Ok(());
        }
        let devices = self.devices.as_ref().context("devices missing")?;
        let target = resources.render_target.as_ref().context("target missing")?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { target.GetDesc(&mut desc) };
        desc.BindFlags = 0;
        desc.MiscFlags = 0;
        desc.CPUAccessFlags = 0;
        desc.Usage = D3D11_USAGE_DEFAULT;
        let mut scratch = None;
        unsafe {
            devices
                .device
                .CreateTexture2D(&desc, None, Some(&mut scratch))?
        };
        resources.scroll_scratch = Some(scratch.context("scroll scratch allocation missing")?);
        Ok(())
    }

    /// No target pixel changes until scratch allocation and buffer upload succeed.
    pub(super) fn render_scroll(
        &mut self,
        scene: &Scene,
        appearance: WindowBackgroundAppearance,
        plan: &SceneScrollPlan,
        ordinary_damage: SceneDamage,
    ) -> Result<()> {
        if let Err(error) = self.ensure_scroll_scratch() {
            if let Some(resources) = &mut self.resources {
                resources.scroll_scratch_failed = true;
            }
            log::debug!("Scroll-copy allocation unavailable; replaying damage: {error:#}");
            return self.render_damage(scene, appearance, ordinary_damage);
        }
        let selection = self.prepare_damage_buffers(scene, &plan.redraw)?;
        self.copy_scroll(plan)?;
        let clear = match appearance {
            WindowBackgroundAppearance::Opaque => [1.0f32; 4],
            _ => [0.0f32; 4],
        };
        for (strip, rect) in plan.redraw.iter().enumerate() {
            self.pre_draw(&clear, Some(*rect))?;
            self.draw_batches(
                scene,
                Some(*rect),
                selection.as_ref().map(|selected| (selected, strip)),
            )?;
        }
        Ok(())
    }

    fn copy_scroll(&self, plan: &SceneScrollPlan) -> Result<()> {
        let resources = self.resources.as_ref().context("resources missing")?;
        let target = resources.render_target.as_ref().context("target missing")?;
        let scratch = resources
            .scroll_scratch
            .as_ref()
            .context("scratch missing")?;
        let context = &self
            .devices
            .as_ref()
            .context("devices missing")?
            .device_context;
        unsafe {
            context.OMSetRenderTargets(None, None);
            // Preserve all source pixels before any overlapping destination writes.
            let source = copy_box(plan.region);
            context.CopySubresourceRegion(
                scratch,
                0,
                plan.region.left,
                plan.region.top,
                0,
                target,
                0,
                Some(&source),
            );
            for destination in &plan.copied {
                let source = SceneDamageRect {
                    top: (destination.top as i32 - plan.dy) as u32,
                    bottom: (destination.bottom as i32 - plan.dy) as u32,
                    ..*destination
                };
                context.CopySubresourceRegion(
                    target,
                    0,
                    destination.left,
                    destination.top,
                    0,
                    scratch,
                    0,
                    Some(&copy_box(source)),
                );
            }
        }
        Ok(())
    }
}

fn copy_box(rect: SceneDamageRect) -> D3D11_BOX {
    D3D11_BOX {
        left: rect.left,
        top: rect.top,
        front: 0,
        right: rect.right,
        bottom: rect.bottom,
        back: 1,
    }
}
