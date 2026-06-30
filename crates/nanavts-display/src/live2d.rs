use crate::model::ArtMeshItem;
use std::path::Path;

pub type Live2dScene = live2d::core::ModelSnapshot;

#[cfg(feature = "wgpu")]
pub type Live2dGpuRenderer = live2d::wgpu::WgpuLive2DRenderer;

#[cfg(feature = "wgpu")]
pub type Live2dView = live2d::wgpu::WgpuLive2DView;

pub fn inspect_art_meshes(model_json_path: impl AsRef<Path>) -> Result<Vec<ArtMeshItem>, String> {
    live2d::runtime::inspect_art_meshes(model_json_path)
        .map(|meshes| meshes.into_iter().map(ArtMeshItem::from).collect())
}

pub fn load_scene(model_json_path: impl AsRef<Path>) -> Result<Live2dScene, String> {
    live2d::runtime::load_snapshot(model_json_path)
}
