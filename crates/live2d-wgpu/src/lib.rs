pub(crate) const MASK_ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
pub(crate) const MASK_DRAW_LOOKUP_INDEX_THRESHOLD: usize = 8 * 1024;
pub(crate) const POST_PROCESS_CLEAR: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

mod api;
mod main_pass;
mod mask;
mod pipeline;
mod post_process;
mod preview;
#[cfg(feature = "probe")]
mod probe;
mod renderer;
mod resources;
#[cfg(test)]
mod tests;
mod upload;

pub use api::*;
pub use post_process::*;
pub use preview::*;
pub use renderer::WgpuLive2DRenderer;
