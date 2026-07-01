#[derive(Debug, Clone)]
pub struct WgpuLive2DView {
    pub transform: [f32; 4],
    pub width: u32,
    pub height: u32,
    pub effect: [f32; 4],
    pub target_drawable_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WgpuTextureSampling {
    Nearest,
    #[default]
    Linear,
    Cubic,
}

impl WgpuTextureSampling {
    pub(crate) fn shader_mode(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Linear => 1,
            Self::Cubic => 2,
        }
    }

    pub(crate) fn bind_group_sampling(self) -> Self {
        match self {
            Self::Nearest => Self::Nearest,
            Self::Linear | Self::Cubic => Self::Linear,
        }
    }
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
