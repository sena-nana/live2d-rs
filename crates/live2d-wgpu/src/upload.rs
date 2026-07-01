use crate::*;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Live2dUniform {
    pub(crate) viewport: [f32; 4],
    pub(crate) view_transform: [f32; 4],
    pub(crate) canvas: [f32; 4],
    pub(crate) effect: [f32; 4],
    pub(crate) mask: [f32; 4],
    pub(crate) blend: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuPosition {
    pub(crate) position: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct GpuUv {
    pub(crate) uv: [f32; 2],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PositionUpload {
    pub(crate) vertex_range: std::ops::Range<u32>,
    pub(crate) byte_offset: u64,
}

pub(crate) struct GpuUploadPlan {
    pub(crate) uploads: Vec<PositionUpload>,
}

impl GpuUploadPlan {
    #[cfg(feature = "probe")]
    fn upload_bytes(&self) -> u64 {
        self.uploads
            .iter()
            .map(|upload| {
                (upload.vertex_range.end - upload.vertex_range.start) as u64
                    * std::mem::size_of::<GpuPosition>() as u64
            })
            .sum()
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct UniformUploadStats {
    pub(crate) writes: u64,
    pub(crate) bytes: u64,
}
pub(crate) fn live2d_canvas_uniform(canvas: &CanvasInfo) -> [f32; 4] {
    let pixels_per_unit = canvas.pixels_per_unit.max(0.0001);
    [
        (canvas.size[0] / pixels_per_unit).max(0.0001),
        (canvas.size[1] / pixels_per_unit).max(0.0001),
        canvas.origin[0] / pixels_per_unit,
        canvas.origin[1] / pixels_per_unit,
    ]
}

pub(crate) fn aligned_uniform_stride(device: &wgpu::Device) -> u64 {
    aligned_uniform_stride_for::<Live2dUniform>(device)
}

pub(crate) fn aligned_uniform_stride_for<T>(device: &wgpu::Device) -> u64 {
    align_to(
        std::mem::size_of::<T>() as u64,
        device.limits().min_uniform_buffer_offset_alignment.max(1) as u64,
    )
}

pub(crate) fn uniform_binding(
    buffer: &wgpu::Buffer,
    uniform_stride: u64,
) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: wgpu::BufferSize::new(uniform_stride),
    })
}
pub(crate) fn align_to(value: u64, alignment: u64) -> u64 {
    value.div_ceil(alignment) * alignment
}

pub(crate) fn uniform_slots(render_plan: &RenderPlan) -> usize {
    render_plan.draws.len() + mask_uniform_slots(render_plan)
}
pub(crate) fn mix_f32_slice(signature: &mut u64, values: &[f32]) {
    for value in values {
        mix_u64(signature, value.to_bits() as u64);
    }
}

pub(crate) fn mix_str(signature: &mut u64, value: &str) {
    mix_u64(signature, value.len() as u64);
    for byte in value.as_bytes() {
        mix_u64(signature, *byte as u64);
    }
}

pub(crate) fn mix_u64(signature: &mut u64, value: u64) {
    *signature ^= value
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(*signature << 6)
        .wrapping_add(*signature >> 2);
}
pub(crate) fn upload_main_uniforms(
    queue: &wgpu::Queue,
    uniform_buffer: &wgpu::Buffer,
    uniform_stride: u64,
    first_uniform_slot: usize,
    render_plan: &RenderPlan,
    canvas: &CanvasInfo,
    view: &WgpuLive2DView,
    mask_atlas: Option<&MaskAtlas>,
    bytes: &mut Vec<u8>,
) -> UniformUploadStats {
    fill_main_uniform_upload_bytes(render_plan, canvas, view, mask_atlas, uniform_stride, bytes);
    if bytes.is_empty() {
        return UniformUploadStats::default();
    }

    queue.write_buffer(
        uniform_buffer,
        uniform_stride * first_uniform_slot as u64,
        &bytes,
    );
    UniformUploadStats {
        writes: 1,
        bytes: bytes.len() as u64,
    }
}

pub(crate) fn fill_main_uniform_upload_bytes(
    render_plan: &RenderPlan,
    canvas: &CanvasInfo,
    view: &WgpuLive2DView,
    mask_atlas: Option<&MaskAtlas>,
    uniform_stride: u64,
    bytes: &mut Vec<u8>,
) {
    bytes.clear();
    if render_plan.draws.is_empty() {
        return;
    }

    let uniform_stride = uniform_stride as usize;
    let uniform_size = std::mem::size_of::<Live2dUniform>();
    debug_assert!(uniform_stride >= uniform_size);
    bytes.resize(uniform_stride * render_plan.draws.len(), 0);
    let canvas = live2d_canvas_uniform(canvas);
    let viewport = [
        view.width.max(1) as f32,
        view.height.max(1) as f32,
        canvas[0] / canvas[1],
        0.0,
    ];

    for (index, draw) in render_plan.draws.iter().enumerate() {
        let effect = if view.target_drawable_ids.is_empty()
            || view
                .target_drawable_ids
                .iter()
                .any(|target_id| target_id == draw.drawable_id.as_ref())
        {
            [
                view.effect[0],
                view.effect[1],
                view.effect[2],
                view.effect[3] * draw.opacity.clamp(0.0, 1.0),
            ]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let uniform = Live2dUniform {
            viewport,
            view_transform: view.transform,
            canvas,
            effect,
            mask: mask_uniform(draw, mask_atlas),
            blend: blend_uniform(draw.blend_mode),
        };
        let offset = index * uniform_stride;
        bytes[offset..offset + uniform_size].copy_from_slice(bytemuck::bytes_of(&uniform));
    }
}

pub(crate) fn scene_topology(snapshot: &ModelSnapshot) -> SceneTopology {
    let mut signature = 0xcbf2_9ce4_8422_2325;
    for drawable in renderable_drawables(snapshot) {
        mix_u64(&mut signature, 1);
        mix_str(&mut signature, drawable.id.as_ref());
        mix_u64(&mut signature, drawable.vertices.len() as u64);
        for vertex in &drawable.vertices {
            mix_f32_slice(&mut signature, &vertex.uv);
        }
        mix_u64(&mut signature, drawable.indices.len() as u64);
        for index in &drawable.indices {
            mix_u64(&mut signature, *index as u64);
        }
    }
    signature
}

pub(crate) fn texture_topology(snapshot: &ModelSnapshot) -> TextureTopology {
    snapshot
        .textures
        .iter()
        .map(|texture| (texture.width, texture.height, texture.rgba.len()))
        .collect()
}

pub(crate) fn gpu_scene_positions(
    snapshot: &ModelSnapshot,
    render_plan: &RenderPlan,
) -> Vec<GpuPosition> {
    let mut positions = Vec::with_capacity(render_plan.model.vertex_count as usize);
    for drawable in renderable_drawables(snapshot) {
        positions.extend(drawable.vertices.iter().map(|vertex| GpuPosition {
            position: vertex.position,
        }));
    }
    positions
}

pub(crate) fn gpu_position_upload_plan(
    positions: &mut Vec<GpuPosition>,
    snapshot: &ModelSnapshot,
    render_plan: &RenderPlan,
) -> GpuUploadPlan {
    if positions.len() != render_plan.model.vertex_count as usize {
        let expected = render_plan.model.vertex_count as usize;
        positions.clear();
        positions.reserve(expected);
        for drawable in renderable_drawables(snapshot) {
            positions.extend(drawable.vertices.iter().map(|vertex| GpuPosition {
                position: vertex.position,
            }));
        }
        let uploads = full_position_upload(positions);
        return GpuUploadPlan { uploads };
    }

    let mut uploads = Vec::new();
    let mut dirty_start = None;
    let mut index = 0;

    for drawable in renderable_drawables(snapshot) {
        for vertex in &drawable.vertices {
            let position = GpuPosition {
                position: vertex.position,
            };
            let Some(previous_position) = positions.get_mut(index) else {
                break;
            };
            let is_dirty = *previous_position != position;
            if is_dirty {
                *previous_position = position;
            }
            match (dirty_start, is_dirty) {
                (None, true) => dirty_start = Some(index),
                (Some(start), false) => {
                    uploads.push(PositionUpload {
                        vertex_range: start as u32..index as u32,
                        byte_offset: position_byte_offset(start),
                    });
                    dirty_start = None;
                }
                _ => {}
            }
            index += 1;
        }
    }

    if index != positions.len() {
        positions.truncate(index);
        let uploads = full_position_upload(positions);
        return GpuUploadPlan { uploads };
    }

    if let Some(start) = dirty_start {
        uploads.push(PositionUpload {
            vertex_range: start as u32..positions.len() as u32,
            byte_offset: position_byte_offset(start),
        });
    }

    GpuUploadPlan { uploads }
}

pub(crate) fn full_position_upload(positions: &[GpuPosition]) -> Vec<PositionUpload> {
    if positions.is_empty() {
        Vec::new()
    } else {
        vec![PositionUpload {
            vertex_range: 0..positions.len() as u32,
            byte_offset: 0,
        }]
    }
}

pub(crate) fn position_byte_offset(vertex_index: usize) -> u64 {
    vertex_index as u64 * std::mem::size_of::<GpuPosition>() as u64
}

pub(crate) fn apply_gpu_upload_plan(
    queue: &wgpu::Queue,
    gpu_scene: &mut GpuScene,
    upload_plan: GpuUploadPlan,
) {
    for upload in &upload_plan.uploads {
        let range = upload.vertex_range.start as usize..upload.vertex_range.end as usize;
        queue.write_buffer(
            &gpu_scene.position_buffer,
            upload.byte_offset,
            bytemuck::cast_slice(&gpu_scene.positions[range]),
        );
    }
}

pub(crate) fn gpu_scene_uvs(snapshot: &ModelSnapshot, render_plan: &RenderPlan) -> Vec<GpuUv> {
    let mut uvs = Vec::with_capacity(render_plan.model.vertex_count as usize);
    for drawable in renderable_drawables(snapshot) {
        uvs.extend(
            drawable
                .vertices
                .iter()
                .map(|vertex| GpuUv { uv: vertex.uv }),
        );
    }
    uvs
}

pub(crate) fn gpu_scene_indices(snapshot: &ModelSnapshot, render_plan: &RenderPlan) -> Vec<u16> {
    let mut indices = Vec::with_capacity(render_plan.model.index_count as usize);
    for drawable in renderable_drawables(snapshot) {
        indices.extend_from_slice(&drawable.indices);
    }
    indices
}

pub(crate) fn renderable_drawables(snapshot: &ModelSnapshot) -> impl Iterator<Item = &Drawable> {
    snapshot
        .drawables
        .iter()
        .filter(|drawable| !drawable.vertices.is_empty() && !drawable.indices.is_empty())
}

pub(crate) fn buffer_size<T>(len: usize) -> u64 {
    (len * std::mem::size_of::<T>()).max(1) as u64
}

pub(crate) fn padded_index_bytes(indices: &[u16]) -> Vec<u8> {
    let bytes = bytemuck::cast_slice(indices);
    let aligned_len = bytes
        .len()
        .next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT as usize);
    let mut padded = Vec::with_capacity(aligned_len);
    padded.extend_from_slice(bytes);
    padded.resize(aligned_len, 0);
    padded
}

impl WgpuLive2DRenderer {
    pub(crate) fn prepare_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) -> RenderPlan {
        let render_plan = self.render_world.build(snapshot);
        let topology = scene_topology(snapshot);
        let texture_topology = texture_topology(snapshot);
        let active_scene_changed =
            self.active_scene_key.as_deref() != Some(snapshot.model_key.as_str());
        let texture_dirty = self
            .texture_caches
            .get(&snapshot.model_key)
            .map(|cache| cache.topology != texture_topology)
            .unwrap_or(true);
        let textures = self.prepare_textures(device, queue, snapshot);
        if active_scene_changed || texture_dirty {
            self.mask_atlas_dirty = true;
        }
        if self.scene_topologies.get(&snapshot.model_key) == Some(&topology)
            && self.gpu_scenes.contains_key(&snapshot.model_key)
        {
            self.active_scene_key = Some(snapshot.model_key.clone());
            self.upload_scene_positions(queue, snapshot, &render_plan);
            if let Some(gpu_scene) = self.active_gpu_scene_mut() {
                gpu_scene.textures = textures;
            }
            return render_plan;
        }
        self.active_scene_key = Some(snapshot.model_key.clone());
        self.scene_topologies
            .insert(snapshot.model_key.clone(), topology);
        self.mask_atlas_dirty = true;
        let positions = gpu_scene_positions(snapshot, &render_plan);
        let uvs = gpu_scene_uvs(snapshot, &render_plan);
        let indices = gpu_scene_indices(snapshot, &render_plan);
        let position_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model Positions"),
            size: buffer_size::<GpuPosition>(positions.len()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&position_buffer, 0, bytemuck::cast_slice(&positions));
        let uv_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model UVs"),
            size: buffer_size::<GpuUv>(uvs.len()),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uv_buffer, 0, bytemuck::cast_slice(&uvs));
        let index_bytes = padded_index_bytes(&indices);
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Model Indices"),
            size: index_bytes.len().max(1) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index_buffer, 0, &index_bytes);
        self.gpu_scenes.insert(
            snapshot.model_key.clone(),
            GpuScene {
                position_buffer,
                uv_buffer,
                index_buffer,
                positions,
                vertex_count: render_plan.model.vertex_count,
                index_count: render_plan.model.index_count,
                textures,
            },
        );
        render_plan
    }

    #[cfg(feature = "probe")]
    pub(crate) fn prepare_scene_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        probe: &P,
    ) -> RenderPlan
    where
        P: ProbeSink,
    {
        let render_plan = self.render_world.build_with_probe(snapshot, probe);
        let topology = scene_topology(snapshot);
        let texture_topology = texture_topology(snapshot);
        let active_scene_changed =
            self.active_scene_key.as_deref() != Some(snapshot.model_key.as_str());
        let texture_dirty = self
            .texture_caches
            .get(&snapshot.model_key)
            .map(|cache| cache.topology != texture_topology)
            .unwrap_or(true);
        let textures = self.prepare_textures_with_probe(device, queue, snapshot, probe);
        if active_scene_changed || texture_dirty {
            self.mask_atlas_dirty = true;
        }
        if self.scene_topologies.get(&snapshot.model_key) == Some(&topology)
            && self.gpu_scenes.contains_key(&snapshot.model_key)
        {
            self.active_scene_key = Some(snapshot.model_key.clone());
            counter(
                probe,
                Stage::WgpuSceneTopologyHit,
                "cache_hits",
                1,
                vec![ProbeAttr::new("cache", "scene_topology")],
            );
            self.upload_scene_positions_with_probe(queue, snapshot, &render_plan, probe);
            if let Some(gpu_scene) = self.active_gpu_scene_mut() {
                gpu_scene.textures = textures;
            }
            return render_plan;
        }
        counter(
            probe,
            Stage::WgpuSceneTopologyMiss,
            "cache_misses",
            1,
            vec![ProbeAttr::new("cache", "scene_topology")],
        );
        self.active_scene_key = Some(snapshot.model_key.clone());
        self.scene_topologies
            .insert(snapshot.model_key.clone(), topology);
        self.mask_atlas_dirty = true;
        let positions = gpu_scene_positions(snapshot, &render_plan);
        let uvs = gpu_scene_uvs(snapshot, &render_plan);
        let indices = gpu_scene_indices(snapshot, &render_plan);
        measure(
            probe,
            Stage::WgpuBufferRebuild,
            vec![
                ProbeAttr::new("vertices", positions.len()),
                ProbeAttr::new("indices", indices.len()),
            ],
            || {
                let position_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Live2D Model Positions"),
                    size: buffer_size::<GpuPosition>(positions.len()),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&position_buffer, 0, bytemuck::cast_slice(&positions));
                let uv_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Live2D Model UVs"),
                    size: buffer_size::<GpuUv>(uvs.len()),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&uv_buffer, 0, bytemuck::cast_slice(&uvs));
                let index_bytes = padded_index_bytes(&indices);
                let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Live2D Model Indices"),
                    size: index_bytes.len().max(1) as u64,
                    usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                queue.write_buffer(&index_buffer, 0, &index_bytes);
                counter(
                    probe,
                    Stage::WgpuBufferRebuild,
                    "buffer_writes",
                    3,
                    Vec::new(),
                );
                counter(
                    probe,
                    Stage::WgpuBufferRebuild,
                    "bytes",
                    (positions.len() * std::mem::size_of::<GpuPosition>()
                        + uvs.len() * std::mem::size_of::<GpuUv>()
                        + index_bytes.len()) as u64,
                    Vec::new(),
                );
                counter(
                    probe,
                    Stage::WgpuBufferRebuild,
                    "resource_rebuilds",
                    3,
                    Vec::new(),
                );
                self.gpu_scenes.insert(
                    snapshot.model_key.clone(),
                    GpuScene {
                        position_buffer,
                        uv_buffer,
                        index_buffer,
                        positions,
                        vertex_count: render_plan.model.vertex_count,
                        index_count: render_plan.model.index_count,
                        textures,
                    },
                );
            },
        );
        render_plan
    }

    pub(crate) fn prepare_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
    ) -> Vec<wgpu::BindGroup> {
        let topology = texture_topology(snapshot);
        if let Some(cache) = self.texture_caches.get(&snapshot.model_key) {
            if cache.topology == topology {
                return cache.bind_groups.clone();
            }
        }

        let bind_groups = snapshot
            .textures
            .iter()
            .map(|texture| self.create_texture_bind_group(device, queue, texture))
            .collect::<Vec<_>>();
        self.texture_caches.insert(
            snapshot.model_key.clone(),
            TextureCache {
                topology,
                bind_groups: bind_groups.clone(),
            },
        );
        bind_groups
    }

    #[cfg(feature = "probe")]
    fn prepare_textures_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        probe: &P,
    ) -> Vec<wgpu::BindGroup>
    where
        P: ProbeSink,
    {
        let topology = texture_topology(snapshot);
        if let Some(cache) = self.texture_caches.get(&snapshot.model_key) {
            if cache.topology == topology {
                counter(
                    probe,
                    Stage::WgpuTextureCacheHit,
                    "cache_hits",
                    1,
                    vec![ProbeAttr::new("cache", "textures")],
                );
                return cache.bind_groups.clone();
            }
        }

        counter(
            probe,
            Stage::WgpuTextureCacheMiss,
            "cache_misses",
            1,
            vec![ProbeAttr::new("cache", "textures")],
        );
        let bind_groups = snapshot
            .textures
            .iter()
            .map(|texture| {
                measure(
                    probe,
                    Stage::WgpuTextureUpload,
                    vec![
                        ProbeAttr::new("width", texture.width),
                        ProbeAttr::new("height", texture.height),
                    ],
                    || self.create_texture_bind_group(device, queue, texture),
                )
            })
            .collect::<Vec<_>>();
        counter(
            probe,
            Stage::WgpuTextureUpload,
            "bytes",
            snapshot
                .textures
                .iter()
                .map(|texture| texture.rgba.len() as u64)
                .sum(),
            Vec::new(),
        );
        counter(
            probe,
            Stage::WgpuTextureUpload,
            "resource_rebuilds",
            bind_groups.len() as u64,
            vec![ProbeAttr::new("resource", "texture_bind_group")],
        );
        self.texture_caches.insert(
            snapshot.model_key.clone(),
            TextureCache {
                topology,
                bind_groups: bind_groups.clone(),
            },
        );
        bind_groups
    }

    pub(crate) fn ensure_uniform_capacity(&mut self, device: &wgpu::Device, required_slots: usize) {
        let required_slots = required_slots.max(1);
        if self.uniform_capacity >= required_slots {
            return;
        }

        let new_capacity = required_slots.next_power_of_two();
        self.uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Live2D Uniform"),
            size: self.uniform_stride * new_capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Uniform Bind Group"),
            layout: &self.uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_binding(&self.uniform_buffer, self.uniform_stride),
            }],
        });
        self.uniform_capacity = new_capacity;
    }

    #[cfg(feature = "probe")]
    pub(crate) fn ensure_uniform_capacity_with_probe<P>(
        &mut self,
        device: &wgpu::Device,
        required_slots: usize,
        probe: &P,
    ) where
        P: ProbeSink,
    {
        let required_slots = required_slots.max(1);
        if self.uniform_capacity >= required_slots {
            return;
        }
        measure(
            probe,
            Stage::WgpuUniformCapacityGrow,
            vec![
                ProbeAttr::new("old_capacity", self.uniform_capacity),
                ProbeAttr::new("required_slots", required_slots),
            ],
            || self.ensure_uniform_capacity(device, required_slots),
        );
        counter(
            probe,
            Stage::WgpuUniformCapacityGrow,
            "resource_rebuilds",
            2,
            vec![ProbeAttr::new("resource", "uniform_buffer_and_bind_group")],
        );
        counter(
            probe,
            Stage::WgpuUniformCapacityGrow,
            "bytes",
            self.uniform_stride * self.uniform_capacity as u64,
            Vec::new(),
        );
    }

    pub(crate) fn upload_scene_positions(
        &mut self,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        render_plan: &RenderPlan,
    ) {
        let mask_dirty = {
            let Some(gpu_scene) = self.active_gpu_scene_mut() else {
                return;
            };
            if gpu_scene.vertex_count != render_plan.model.vertex_count {
                return;
            }
            let upload_plan =
                gpu_position_upload_plan(&mut gpu_scene.positions, snapshot, render_plan);
            let mask_dirty = position_uploads_touch_masks(&upload_plan.uploads, render_plan);
            apply_gpu_upload_plan(queue, gpu_scene, upload_plan);
            mask_dirty
        };
        self.mask_atlas_dirty |= mask_dirty;
    }

    #[cfg(feature = "probe")]
    fn upload_scene_positions_with_probe<P>(
        &mut self,
        queue: &wgpu::Queue,
        snapshot: &ModelSnapshot,
        render_plan: &RenderPlan,
        probe: &P,
    ) where
        P: ProbeSink,
    {
        let Some(gpu_scene) = self.active_gpu_scene() else {
            return;
        };
        if gpu_scene.vertex_count != render_plan.model.vertex_count {
            return;
        }
        let mut uploads = 0;
        let mut bytes = 0;
        let mut mask_dirty = false;
        measure(probe, Stage::WgpuPositionUpload, Vec::new(), || {
            let Some(gpu_scene) = self.active_gpu_scene_mut() else {
                return;
            };
            let upload_plan =
                gpu_position_upload_plan(&mut gpu_scene.positions, snapshot, render_plan);
            uploads = upload_plan.uploads.len();
            bytes = upload_plan.upload_bytes();
            mask_dirty = position_uploads_touch_masks(&upload_plan.uploads, render_plan);
            apply_gpu_upload_plan(queue, gpu_scene, upload_plan);
        });
        self.mask_atlas_dirty |= mask_dirty;
        counter(
            probe,
            Stage::WgpuPositionUpload,
            "buffer_writes",
            uploads as u64,
            Vec::new(),
        );
        counter(probe, Stage::WgpuPositionUpload, "bytes", bytes, Vec::new());
    }

    fn create_texture_bind_group(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &TextureAsset,
    ) -> wgpu::BindGroup {
        let size = wgpu::Extent3d {
            width: texture.width.max(1),
            height: texture.height.max(1),
            depth_or_array_layers: 1,
        };
        let gpu_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Live2D Texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &gpu_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texture.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * texture.width.max(1)),
                rows_per_image: Some(texture.height.max(1)),
            },
            size,
        );
        let view = gpu_texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Live2D Texture Bind Group"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::*;
    use crate::*;
    use live2d_core::{
        AlphaBlendMode, BlendMode, CanvasInfo, ClippingInfo, ColorBlendMode, DrawableId, MaskRef,
        MaterialKey, TextureAsset, Vertex,
    };
    use live2d_render::RenderPlanner;

    #[test]
    fn pads_odd_index_upload_bytes_without_changing_draw_count() {
        let indices = [0_u16, 1, 2];
        let bytes = padded_index_bytes(&indices);
        let raw_bytes: &[u8] = bytemuck::cast_slice(&indices);

        assert_eq!(bytes.len() % wgpu::COPY_BUFFER_ALIGNMENT as usize, 0);
        assert_eq!(
            &bytes[..indices.len() * std::mem::size_of::<u16>()],
            raw_bytes
        );
        assert_eq!(indices.len(), 3);
    }

    #[test]
    fn align_to_rounds_uniform_stride_to_required_boundary() {
        assert_eq!(align_to(80, 256), 256);
        assert_eq!(align_to(512, 256), 512);
    }

    #[test]
    fn uniform_slots_include_mask_writer_and_main_draws() {
        let render_plan = RenderPlanner::new().build(&masked_snapshot());

        assert_eq!(mask_uniform_slots(&render_plan), 1);
        assert_eq!(uniform_slots(&render_plan), 3);
    }

    #[test]
    fn main_uniform_upload_bytes_pack_aligned_draw_slots() {
        let render_plan = RenderPlanner::new().build(&masked_snapshot());
        let stride = align_to(std::mem::size_of::<Live2dUniform>() as u64, 256);
        let view = WgpuLive2DView {
            transform: [0.1, 0.2, 1.5, 0.0],
            width: 320,
            height: 240,
            effect: [0.4, 0.5, 0.6, 0.7],
            target_drawable_ids: vec!["masked".to_owned()],
        };

        let canvas = CanvasInfo {
            size: [4.0, 2.0],
            origin: [0.0, 0.0],
            pixels_per_unit: 1.0,
        };
        let mut bytes = Vec::new();
        fill_main_uniform_upload_bytes(&render_plan, &canvas, &view, None, stride, &mut bytes);
        let first =
            bytemuck::from_bytes::<Live2dUniform>(&bytes[..std::mem::size_of::<Live2dUniform>()]);
        let second_offset = stride as usize;
        let second = bytemuck::from_bytes::<Live2dUniform>(
            &bytes[second_offset..second_offset + std::mem::size_of::<Live2dUniform>()],
        );

        assert_eq!(bytes.len(), stride as usize * render_plan.draws.len());
        assert_eq!(first.effect, [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(second.effect, [0.4, 0.5, 0.6, 0.7]);
        assert_eq!(second.viewport, [320.0, 240.0, 2.0, 0.0]);
        assert_eq!(second.view_transform, view.transform);
    }

    #[test]
    fn scene_topology_allows_dynamic_vertex_upload_without_rebuild() {
        let mut next = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        next.drawables[0].vertices[0].position = [2.0, 3.0];
        let base_plan = RenderPlanner::new().build(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1));
        let next_plan = RenderPlanner::new().build(&next);

        assert_eq!(
            scene_topology(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1)),
            scene_topology(&next)
        );
        assert_ne!(
            gpu_scene_positions(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1), &base_plan),
            gpu_scene_positions(&next, &next_plan)
        );
        assert_eq!(
            gpu_scene_uvs(&snapshot_with_drawable("mesh", 0, 2, 3, 0, 1), &base_plan),
            gpu_scene_uvs(&next, &next_plan)
        );
    }

    #[test]
    fn scene_topology_changes_for_static_gpu_buffer_data() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        let mut changed_uv = base.clone();
        changed_uv.drawables[0].vertices[0].uv = [0.5, 0.25];
        let mut changed_indices = base.clone();
        changed_indices.drawables[0].indices[0] = 1;

        assert_ne!(
            scene_topology(&base),
            scene_topology(&snapshot_with_drawable("mesh", 0, 3, 3, 0, 1))
        );
        assert_ne!(
            scene_topology(&base),
            scene_topology(&snapshot_with_drawable("mesh", 0, 2, 4, 0, 1))
        );
        assert_ne!(scene_topology(&base), scene_topology(&changed_uv));
        assert_ne!(scene_topology(&base), scene_topology(&changed_indices));
    }

    #[test]
    fn texture_topology_is_independent_from_drawable_buffer_shape() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        let changed_drawable = snapshot_with_drawable("mesh", 0, 4, 6, 0, 1);

        assert_eq!(texture_topology(&base), texture_topology(&changed_drawable));
    }

    #[test]
    fn texture_topology_changes_for_texture_resource_shape() {
        let base = snapshot_with_drawable("mesh", 0, 2, 3, 0, 1);
        let mut resized = base.clone();
        resized.textures[0].width = 2;
        resized.textures[0].rgba.resize(2 * 1 * 4, 255);
        let mut changed_bytes = base.clone();
        changed_bytes.textures[0].rgba.push(0);

        assert_ne!(texture_topology(&base), texture_topology(&resized));
        assert_ne!(texture_topology(&base), texture_topology(&changed_bytes));
    }

    #[test]
    fn position_upload_plan_merges_contiguous_dirty_vertices() {
        let base = snapshot_with_drawables(&[("a", 0, 2, 3), ("b", 1, 3, 3)]);
        let mut next = base.clone();
        next.drawables[0].vertices[1].position = [10.0, 11.0];
        next.drawables[1].vertices[0].position = [20.0, 21.0];
        let plan = RenderPlanner::new().build(&base);
        let mut positions = gpu_scene_positions(&base, &plan);
        let upload_plan = gpu_position_upload_plan(&mut positions, &next, &plan);

        assert_eq!(
            upload_plan.uploads,
            vec![PositionUpload {
                vertex_range: 1..3,
                byte_offset: std::mem::size_of::<GpuPosition>() as u64,
            }]
        );
        assert_eq!(positions, gpu_scene_positions(&next, &plan));
    }

    #[test]
    fn position_upload_plan_falls_back_to_full_range_when_lengths_differ() {
        let base = snapshot_with_drawables(&[("a", 0, 2, 3)]);
        let next = snapshot_with_drawables(&[("a", 0, 3, 3)]);
        let base_plan = RenderPlanner::new().build(&base);
        let next_plan = RenderPlanner::new().build(&next);
        let mut positions = gpu_scene_positions(&base, &base_plan);
        let upload_plan = gpu_position_upload_plan(&mut positions, &next, &next_plan);

        assert_eq!(
            upload_plan.uploads,
            vec![PositionUpload {
                vertex_range: 0..3,
                byte_offset: 0,
            }]
        );
        assert_eq!(positions, gpu_scene_positions(&next, &next_plan));
    }
}
