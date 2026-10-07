// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;

impl WgpuRendererCore {
    pub(super) fn render_frame(
        &mut self,
        scene: &Scene,
        target_view: &wgpu::TextureView,
        size: Size<DevicePixels>,
        premultiplied_alpha: bool,
        clear_color: wgpu::Color,
    ) -> Result<wgpu::SubmissionIndex> {
        anyhow::ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "invalid render target size: {size:?}"
        );
        anyhow::ensure!(
            size.width.0 as u32 <= self.max_texture_size
                && size.height.0 as u32 <= self.max_texture_size,
            "render target size {size:?} exceeds maximum texture dimension {}",
            self.max_texture_size
        );

        self.atlas.before_frame();
        self.render_frame_prepared(
            scene,
            target_view,
            size,
            premultiplied_alpha,
            clear_color,
            None,
            None,
        )
    }

    pub(super) fn render_frame_prepared(
        &mut self,
        scene: &Scene,
        target_view: &wgpu::TextureView,
        size: Size<DevicePixels>,
        premultiplied_alpha: bool,
        clear_color: wgpu::Color,
        partial: Option<gpui::SceneDamageRect>,
        presentation: Option<&wgpu::Texture>,
    ) -> Result<wgpu::SubmissionIndex> {
        self.ensure_intermediate_textures(size);

        let gamma_params = GammaParams {
            gamma_ratios: self.rendering_params.gamma_ratios,
            grayscale_enhanced_contrast: self.rendering_params.grayscale_enhanced_contrast,
            subpixel_enhanced_contrast: self.rendering_params.subpixel_enhanced_contrast,
            is_bgr: self.is_bgr as u32,
            _pad: 0,
        };
        let globals = GlobalParams {
            viewport_size: [size.width.0 as f32, size.height.0 as f32],
            premultiplied_alpha: premultiplied_alpha as u32,
            pad: 0,
        };
        let path_globals = GlobalParams {
            premultiplied_alpha: 0,
            ..globals
        };
        self.resources.queue.write_buffer(
            &self.resources.globals_buffer,
            0,
            bytemuck::bytes_of(&globals),
        );
        self.resources.queue.write_buffer(
            &self.resources.globals_buffer,
            self.path_globals_offset,
            bytemuck::bytes_of(&path_globals),
        );
        self.resources.queue.write_buffer(
            &self.resources.globals_buffer,
            self.gamma_offset,
            bytemuck::bytes_of(&gamma_params),
        );

        self.record_frame(scene, target_view, clear_color, partial, presentation)
            .inspect_err(|_| {
                // Queue writes are staged before encoding; flush them even if the frame fails.
                self.resources.queue.submit(std::iter::empty());
            })
    }

    pub(super) fn record_frame(
        &mut self,
        scene: &Scene,
        frame_view: &wgpu::TextureView,
        clear_color: wgpu::Color,
        partial: Option<gpui::SceneDamageRect>,
        presentation: Option<&wgpu::Texture>,
    ) -> Result<wgpu::SubmissionIndex> {
        let mut instance_offset = 0;
        let instance_bindings = self
            .write_instances(scene, &mut instance_offset)
            .with_context(|| {
                format!(
                    "scene too large: {} paths, {} shadows, {} quads, {} underlines, {} monochrome sprites, {} subpixel sprites, {} polychrome sprites",
                    scene.paths.len(),
                    scene.shadows.len(),
                    scene.quads.len(),
                    scene.underlines.len(),
                    scene.monochrome_sprites.len(),
                    scene.subpixel_sprites.len(),
                    scene.polychrome_sprites.len(),
                )
            })?;
        self.prepare_texture_bind_groups(scene);

        let mut encoder =
            self.resources()
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("main_encoder"),
                });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if partial.is_some() {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(clear_color)
                        },
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            #[cfg(target_os = "linux")]
            if let Some(rect) = partial {
                self.clear_damage(&mut pass, rect, clear_color);
            }
            for batch in scene.batches() {
                if partial.is_some_and(|rect| !rect.intersects_batch(scene, &batch)) {
                    continue;
                }
                match batch {
                    PrimitiveBatch::Quads(range) => self.draw_instances(
                        &instance_bindings.quads,
                        &self.resources().pipelines.quads,
                        instance_range(range),
                        &mut pass,
                    ),
                    PrimitiveBatch::Shadows(range) => self.draw_instances(
                        &instance_bindings.shadows,
                        &self.resources().pipelines.shadows,
                        instance_range(range),
                        &mut pass,
                    ),
                    PrimitiveBatch::Paths(range) => {
                        let paths = &scene.paths[range];
                        if paths.is_empty() {
                            continue;
                        }

                        drop(pass);
                        let rasterized = self.draw_paths_to_intermediate(
                            &mut encoder,
                            paths,
                            &mut instance_offset,
                        )?;

                        pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("main_pass_continued"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: frame_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Load,
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            ..Default::default()
                        });

                        if let Some(rect) = partial {
                            pass.set_scissor_rect(
                                rect.left,
                                rect.top,
                                rect.right - rect.left,
                                rect.bottom - rect.top,
                            );
                        }
                        if rasterized {
                            self.draw_paths_from_intermediate(
                                paths,
                                &mut instance_offset,
                                &mut pass,
                            )?;
                        }
                    }
                    PrimitiveBatch::Underlines(range) => self.draw_instances(
                        &instance_bindings.underlines,
                        &self.resources().pipelines.underlines,
                        instance_range(range),
                        &mut pass,
                    ),
                    PrimitiveBatch::MonochromeSprites { texture_id, range } => {
                        self.draw_sprites(
                            &instance_bindings.monochrome_sprites,
                            texture_id,
                            &self.resources().pipelines.mono_sprites,
                            instance_range(range),
                            &mut pass,
                        )?;
                    }
                    PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                        let resources = self.resources();
                        self.draw_sprites(
                            &instance_bindings.subpixel_sprites,
                            texture_id,
                            resources
                                .pipelines
                                .subpixel_sprites
                                .as_ref()
                                .unwrap_or(&resources.pipelines.mono_sprites),
                            instance_range(range),
                            &mut pass,
                        )?;
                    }
                    PrimitiveBatch::PolychromeSprites { texture_id, range } => {
                        self.draw_sprites(
                            &instance_bindings.polychrome_sprites,
                            texture_id,
                            &self.resources().pipelines.poly_sprites,
                            instance_range(range),
                            &mut pass,
                        )?;
                    }
                    // Surfaces are macOS-only for video playback and are not
                    // implemented by the WGPU renderer.
                    PrimitiveBatch::Surfaces(_surfaces) => {}
                }
            }
        }

        #[cfg(target_os = "linux")]
        if let Some(destination) = presentation {
            self.encode_present_copy(&mut encoder, destination)?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = presentation;
        let submission = self
            .resources()
            .queue
            .submit(std::iter::once(encoder.finish()));
        Ok(submission)
    }

    pub(super) fn create_texture_bind_group(
        &self,
        label: &str,
        texture_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let resources = self.resources();
        resources
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &resources.bind_group_layouts.texture,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&resources.atlas_sampler),
                    },
                ],
            })
    }

    pub(super) fn prepare_texture_bind_groups(&mut self, scene: &Scene) {
        let mut texture_ids = SmallVec::<[AtlasTextureId; 8]>::new();
        for batch in scene.batches() {
            let texture_id = match batch {
                PrimitiveBatch::MonochromeSprites { texture_id, .. }
                | PrimitiveBatch::SubpixelSprites { texture_id, .. }
                | PrimitiveBatch::PolychromeSprites { texture_id, .. } => texture_id,
                _ => continue,
            };
            if !texture_ids.contains(&texture_id) {
                texture_ids.push(texture_id);
            }
        }

        self.resources_mut()
            .atlas_texture_bind_groups
            .retain(|texture_id, _| texture_ids.contains(texture_id));

        for texture_id in texture_ids {
            let Some(texture_info) = self.atlas.get_texture_info(texture_id) else {
                self.resources_mut()
                    .atlas_texture_bind_groups
                    .remove(&texture_id);
                continue;
            };
            let is_current = self
                .resources()
                .atlas_texture_bind_groups
                .get(&texture_id)
                .is_some_and(|cached| cached.texture_generation == texture_info.generation);
            if is_current {
                continue;
            }

            let bind_group =
                self.create_texture_bind_group("atlas_texture_bind_group", &texture_info.view);
            self.resources_mut().atlas_texture_bind_groups.insert(
                texture_id,
                CachedTextureBindGroup {
                    texture_generation: texture_info.generation,
                    bind_group,
                },
            );
        }
    }

    pub(super) fn draw_instances(
        &self,
        instances: &InstanceBinding,
        pipeline: &wgpu::RenderPipeline,
        range: Range<u32>,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.resources().globals_bind_group, &[]);
        pass.set_bind_group(1, &instances.bind_group, &[]);
        pass.draw(
            0..4,
            instances.first_instance + range.start..instances.first_instance + range.end,
        );
    }

    pub(super) fn draw_sprites(
        &self,
        sprite_instances: &InstanceBinding,
        texture_id: AtlasTextureId,
        pipeline: &wgpu::RenderPipeline,
        range: Range<u32>,
        pass: &mut wgpu::RenderPass<'_>,
    ) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        let resources = self.resources();
        // The atlas has released this texture; the batch belongs to a stale
        // paint that will be replaced once its view re-renders.
        let Some(texture) = resources.atlas_texture_bind_groups.get(&texture_id) else {
            return Ok(());
        };
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &resources.globals_bind_group, &[]);
        pass.set_bind_group(1, &sprite_instances.bind_group, &[]);
        pass.set_bind_group(2, &texture.bind_group, &[]);
        pass.draw(
            0..4,
            sprite_instances.first_instance + range.start
                ..sprite_instances.first_instance + range.end,
        );
        Ok(())
    }

    pub(super) unsafe fn instance_bytes<T>(instances: &[T]) -> &[u8] {
        unsafe {
            std::slice::from_raw_parts(
                instances.as_ptr() as *const u8,
                std::mem::size_of_val(instances),
            )
        }
    }
}
