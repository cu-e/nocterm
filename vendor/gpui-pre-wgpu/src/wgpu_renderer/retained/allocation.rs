// Nocterm modifications, licensed under the upstream Apache-2.0 license.
use super::*;

pub(super) struct OverwriteClear {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) uniform: wgpu::Buffer,
    pub(super) binding: wgpu::BindGroup,
}

/// Native WGPU scopes catch allocation/validation errors rather than accepting
/// the invalid placeholder handle returned by an infallible create method.
pub(super) fn create_image(
    device: &wgpu::Device,
    size: Size<DevicePixels>,
    format: wgpu::TextureFormat,
) -> Result<RetainedImage> {
    let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("linux_retained_image"),
        size: wgpu::Extent3d {
            width: size.width.0 as u32,
            height: size.height.0 as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let clear = create_clear(device, format);
    let validation_error = gpui::block_on(validation.pop());
    let internal_error = gpui::block_on(internal.pop());
    let oom_error = gpui::block_on(oom.pop());
    device
        .poll(wgpu::PollType::Poll)
        .context("checking retained allocation")?;
    if let Some(error) = validation_error.or(internal_error).or(oom_error) {
        anyhow::bail!("creating retained resources: {error}");
    }
    Ok(RetainedImage {
        texture,
        view,
        clear,
    })
}

fn create_clear(device: &wgpu::Device, format: wgpu::TextureFormat) -> OverwriteClear {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("retained_clear_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(16),
            },
            count: None,
        }],
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("retained_clear_color"),
        size: 16,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("retained_clear_binding"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("retained_clear_pipeline_layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("retained_overwrite_clear"),
        source: wgpu::ShaderSource::Wgsl(include_str!("clear.wgsl").into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("retained_overwrite_clear"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_clear"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_clear"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    OverwriteClear {
        pipeline,
        uniform,
        binding,
    }
}
