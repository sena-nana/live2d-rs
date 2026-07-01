use crate::{
    api::WgpuTextureSampling,
    renderer::{TextureTopology, WgpuLive2DRenderer},
    upload::GpuPosition,
};
#[cfg(feature = "probe")]
use live2d_probe::{counter, measure, ProbeAttr, ProbeSink, Stage};

pub(crate) struct TextureCache {
    pub(crate) topology: TextureTopology,
    pub(crate) sampling: WgpuTextureSampling,
    pub(crate) bind_groups: Vec<wgpu::BindGroup>,
}

pub(crate) struct GpuScene {
    pub(crate) position_buffer: wgpu::Buffer,
    pub(crate) uv_buffer: wgpu::Buffer,
    pub(crate) index_buffer: wgpu::Buffer,
    pub(crate) positions: Vec<GpuPosition>,
    pub(crate) vertex_count: u32,
    pub(crate) index_count: u32,
    pub(crate) textures: Vec<wgpu::BindGroup>,
}

pub(crate) struct MaskAtlas {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) slot_width: u32,
    pub(crate) slot_height: u32,
    pub(crate) columns: usize,
    pub(crate) slots: usize,
    pub(crate) signature: Option<u64>,
    pub(crate) view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaskAtlasUpdate {
    pub(crate) encoded: bool,
    pub(crate) draw_calls: usize,
    pub(crate) uniform_writes: usize,
}

pub(crate) struct OffscreenTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) texture: wgpu::Texture,
    pub(crate) view: wgpu::TextureView,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) composite_uniform_buffer: wgpu::Buffer,
    pub(crate) composite_uniform_bind_group: wgpu::BindGroup,
}

pub(crate) struct BlendCopyTarget {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) format: wgpu::TextureFormat,
    pub(crate) texture: wgpu::Texture,
    pub(crate) bind_group: wgpu::BindGroup,
}

impl WgpuLive2DRenderer {
    pub(crate) fn ensure_model_offscreen_targets(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        count: usize,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        self.model_offscreen_targets.retain(|index, target| {
            *index < count && target.width == width && target.height == height
        });
        for index in 0..count {
            if !self.model_offscreen_targets.contains_key(&index) {
                self.model_offscreen_targets.insert(
                    index,
                    create_offscreen_target(
                        device,
                        &self.texture_layout,
                        &self.offscreen_uniform_layout,
                        &self.sampler,
                        self.pipelines.target_format,
                        width,
                        height,
                    ),
                );
            }
        }
    }

    pub(crate) fn ensure_offscreen_target(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        let rebuild = self
            .offscreen_target
            .as_ref()
            .map(|target| target.width != width || target.height != height)
            .unwrap_or(true);
        if rebuild {
            self.offscreen_target = Some(create_offscreen_target(
                device,
                &self.texture_layout,
                &self.offscreen_uniform_layout,
                &self.sampler,
                self.pipelines.target_format,
                width,
                height,
            ));
        }
    }

    pub(crate) fn ensure_blend_copy_target(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) {
        let width = width.max(1);
        let height = height.max(1);
        let rebuild = self
            .blend_copy_target
            .as_ref()
            .map(|target| {
                target.width != width || target.height != height || target.format != format
            })
            .unwrap_or(true);
        if rebuild {
            self.blend_copy_target = Some(create_blend_copy_target(
                device,
                &self.texture_layout,
                &self.sampler,
                format,
                width,
                height,
            ));
        }
    }

    #[cfg(feature = "probe")]
    pub(crate) fn ensure_offscreen_target_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        probe: &P,
    ) where
        P: ProbeSink,
    {
        let width = width.max(1);
        let height = height.max(1);
        let rebuild = self
            .offscreen_target
            .as_ref()
            .map(|target| target.width != width || target.height != height)
            .unwrap_or(true);
        if rebuild {
            measure(
                probe,
                Stage::WgpuOffscreenResize,
                vec![
                    ProbeAttr::new("width", width),
                    ProbeAttr::new("height", height),
                ],
                || self.ensure_offscreen_target(device, width, height),
            );
            counter(
                probe,
                Stage::WgpuOffscreenResize,
                "resource_rebuilds",
                1,
                vec![ProbeAttr::new("resource", "offscreen_target")],
            );
        } else {
            self.ensure_offscreen_target(device, width, height);
        }
    }
}

pub(crate) fn create_empty_sampled_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Empty Blend Texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    create_sampled_texture_bind_group(device, layout, sampler, &view)
}

pub(crate) fn create_offscreen_target(
    device: &wgpu::Device,
    texture_layout: &wgpu::BindGroupLayout,
    uniform_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> OffscreenTarget {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Offscreen Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = create_sampled_texture_bind_group(device, texture_layout, sampler, &view);
    let composite_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Live2D Offscreen Composite Uniform"),
        size: std::mem::size_of::<[f32; 4]>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let composite_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Live2D Offscreen Composite Uniform Bind Group"),
        layout: uniform_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: composite_uniform_buffer.as_entire_binding(),
        }],
    });
    OffscreenTarget {
        width,
        height,
        texture,
        view,
        bind_group,
        composite_uniform_buffer,
        composite_uniform_bind_group,
    }
}

pub(crate) fn create_blend_copy_target(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> BlendCopyTarget {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Live2D Blend Copy Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = create_sampled_texture_bind_group(device, layout, sampler, &view);
    BlendCopyTarget {
        width,
        height,
        format,
        texture,
        bind_group,
    }
}

pub(crate) fn create_sampled_texture_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    create_sampled_texture_bind_group_with_linear_sampler(device, layout, sampler, sampler, view)
}

pub(crate) fn create_sampled_texture_bind_group_with_linear_sampler(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    linear_sampler: &wgpu::Sampler,
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Live2D Sampled Texture Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(linear_sampler),
            },
        ],
    })
}
