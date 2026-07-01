#[derive(Debug, Clone)]
pub struct WgpuLive2DView {
    pub transform: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub effect: [f32; 4],
    pub target_drawable_ids: Vec<String>,
}

pub struct WgpuLive2DTarget<'view> {
    pub texture: &'view wgpu::Texture,
    pub view: &'view wgpu::TextureView,
    pub resolve_target: Option<&'view wgpu::TextureView>,
    pub load_op: wgpu::LoadOp<wgpu::Color>,
    pub store_op: wgpu::StoreOp,
}

impl<'view> WgpuLive2DTarget<'view> {
    pub fn load(texture: &'view wgpu::Texture, view: &'view wgpu::TextureView) -> Self {
        Self {
            texture,
            view,
            resolve_target: None,
            load_op: wgpu::LoadOp::Load,
            store_op: wgpu::StoreOp::Store,
        }
    }

    pub fn clear(
        texture: &'view wgpu::Texture,
        view: &'view wgpu::TextureView,
        color: wgpu::Color,
    ) -> Self {
        Self {
            texture,
            view,
            resolve_target: None,
            load_op: wgpu::LoadOp::Clear(color),
            store_op: wgpu::StoreOp::Store,
        }
    }
}
