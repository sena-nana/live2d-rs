use live2d_core::{ArtMeshInfo, ModelSnapshot};
#[cfg(feature = "live2d-cubism")]
use live2d_core::{BlendMode, CanvasInfo, Drawable, DrawableId, TextureAsset, Vertex};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(not(feature = "live2d-cubism"))]
const RUNTIME_UNAVAILABLE: &str = "live2d_runtime_unavailable";

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFiles {
    pub model_json_path: PathBuf,
    pub model_root: PathBuf,
    pub moc_path: PathBuf,
    pub texture_paths: Vec<PathBuf>,
    pub missing_files: Vec<String>,
}

pub trait AssetResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone)]
pub struct FsAssetResolver {
    root: PathBuf,
}

impl FsAssetResolver {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

impl AssetResolver for FsAssetResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        fs::read(self.resolve(path)).map_err(|_| "asset_unreadable".to_string())
    }
}

#[derive(Debug, Clone)]
pub struct Live2DInstance {
    snapshot: ModelSnapshot,
    elapsed_seconds: f32,
}

impl Live2DInstance {
    pub fn load(resolver: &FsAssetResolver, model_json_path: &str) -> Result<Self, String> {
        Self::load_file(resolver.resolve(model_json_path))
    }

    pub fn load_file(model_json_path: impl AsRef<Path>) -> Result<Self, String> {
        let snapshot = load_snapshot(model_json_path)?;
        Ok(Self {
            snapshot,
            elapsed_seconds: 0.0,
        })
    }

    pub fn update(&mut self, dt: f32) {
        self.elapsed_seconds += dt.max(0.0);
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed_seconds
    }

    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snapshot
    }
}

pub fn resolve_model_files(model_json_path: impl AsRef<Path>) -> Result<ModelFiles, String> {
    let model_json_path = model_json_path.as_ref();
    if model_json_path.as_os_str().is_empty() {
        return Err("invalid_live2d_model_path".into());
    }
    if !model_json_path.exists() {
        return Err("live2d_model_not_found".into());
    }

    let raw = fs::read_to_string(model_json_path).map_err(|_| "live2d_model_unreadable")?;
    let json: Value = serde_json::from_str(&raw).map_err(|_| "invalid_model3_json")?;
    let model_root = model_json_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let file_refs = json.get("FileReferences").unwrap_or(&Value::Null);
    let moc = file_refs
        .get("Moc")
        .and_then(Value::as_str)
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| "live2d_moc_not_declared".to_string())?;

    let mut missing_files = Vec::new();
    let moc_path = model_root.join(moc);
    if !moc_path.exists() {
        missing_files.push(normalize_relative_path(moc));
    }

    let mut texture_paths = Vec::new();
    if let Some(textures) = file_refs.get("Textures").and_then(Value::as_array) {
        for texture in textures.iter().filter_map(Value::as_str) {
            let path = model_root.join(texture);
            if !path.exists() {
                missing_files.push(normalize_relative_path(texture));
            }
            texture_paths.push(path);
        }
    }

    missing_files.sort();
    missing_files.dedup();

    Ok(ModelFiles {
        model_json_path: model_json_path.to_path_buf(),
        model_root,
        moc_path,
        texture_paths,
        missing_files,
    })
}

pub fn inspect_art_meshes(model_json_path: impl AsRef<Path>) -> Result<Vec<ArtMeshInfo>, String> {
    runtime::inspect_art_meshes(model_json_path.as_ref())
}

pub fn load_snapshot(model_json_path: impl AsRef<Path>) -> Result<ModelSnapshot, String> {
    runtime::load_snapshot(model_json_path.as_ref())
}

fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(not(feature = "live2d-cubism"))]
mod runtime {
    use super::*;

    pub fn inspect_art_meshes(model_json_path: &Path) -> Result<Vec<ArtMeshInfo>, String> {
        let files = resolve_model_files(model_json_path)?;
        if !files.missing_files.is_empty() {
            return Err("live2d_model_assets_missing".into());
        }
        Err(RUNTIME_UNAVAILABLE.into())
    }

    pub fn load_snapshot(model_json_path: &Path) -> Result<ModelSnapshot, String> {
        let files = resolve_model_files(model_json_path)?;
        if !files.missing_files.is_empty() {
            return Err("live2d_model_assets_missing".into());
        }
        Err(RUNTIME_UNAVAILABLE.into())
    }
}

#[cfg(feature = "live2d-cubism")]
mod runtime {
    use super::*;
    use std::ffi::CStr;

    pub fn inspect_art_meshes(model_json_path: &Path) -> Result<Vec<ArtMeshInfo>, String> {
        Ok(load_snapshot(model_json_path)?.art_meshes)
    }

    pub fn load_snapshot(model_json_path: &Path) -> Result<ModelSnapshot, String> {
        let files = resolve_model_files(model_json_path)?;
        if !files.missing_files.is_empty() {
            return Err("live2d_model_assets_missing".into());
        }
        let textures = load_textures(&files.texture_paths)?;
        let mut moc_bytes = fs::read(&files.moc_path).map_err(|_| "live2d_moc_unreadable")?;
        let moc = unsafe {
            live2d_sys::csmReviveMocInPlace(moc_bytes.as_mut_ptr().cast(), moc_bytes.len() as _)
        };
        if moc.is_null() {
            return Err("live2d_moc_invalid".into());
        }
        let model_size = unsafe { live2d_sys::csmGetSizeofModel(moc) } as usize;
        if model_size == 0 {
            return Err("live2d_model_allocation_failed".into());
        }
        let mut model_bytes = vec![0_u8; model_size];
        let model = unsafe {
            live2d_sys::csmInitializeModelInPlace(
                moc,
                model_bytes.as_mut_ptr().cast(),
                model_bytes.len() as _,
            )
        };
        if model.is_null() {
            return Err("live2d_model_initialization_failed".into());
        }
        unsafe { live2d_sys::csmUpdateModel(model) };
        let (canvas, drawables, art_meshes) = unsafe { snapshot_model(model)? };

        Ok(ModelSnapshot {
            model_key: files.model_json_path.to_string_lossy().into_owned(),
            canvas,
            art_meshes,
            drawables,
            textures,
        })
    }

    unsafe fn snapshot_model(
        model: *mut live2d_sys::CsmModel,
    ) -> Result<(CanvasInfo, Vec<Drawable>, Vec<ArtMeshInfo>), String> {
        let mut canvas_size = live2d_sys::CsmVector2 { x: 2.0, y: 2.0 };
        let mut canvas_origin = live2d_sys::CsmVector2 { x: 0.0, y: 0.0 };
        let mut pixels_per_unit = 1.0;
        live2d_sys::csmReadCanvasInfo(
            model,
            &mut canvas_size,
            &mut canvas_origin,
            &mut pixels_per_unit,
        );
        let count = live2d_sys::csmGetDrawableCount(model).max(0) as usize;
        let ids = live2d_sys::csmGetDrawableIds(model);
        let render_orders = live2d_sys::csmGetRenderOrders(model);
        let texture_indices = live2d_sys::csmGetDrawableTextureIndices(model);
        let vertex_counts = live2d_sys::csmGetDrawableVertexCounts(model);
        let vertex_positions = live2d_sys::csmGetDrawableVertexPositions(model);
        let vertex_uvs = live2d_sys::csmGetDrawableVertexUvs(model);
        let index_counts = live2d_sys::csmGetDrawableIndexCounts(model);
        let indices = live2d_sys::csmGetDrawableIndices(model);
        if ids.is_null()
            || render_orders.is_null()
            || texture_indices.is_null()
            || vertex_counts.is_null()
            || vertex_positions.is_null()
            || vertex_uvs.is_null()
            || index_counts.is_null()
            || indices.is_null()
        {
            return Err("live2d_model_snapshot_failed".into());
        }

        let mut drawables = Vec::new();
        let mut art_meshes = Vec::new();
        for index in 0..count {
            let id_ptr = *ids.add(index);
            if id_ptr.is_null() {
                continue;
            }
            let id = CStr::from_ptr(id_ptr).to_string_lossy().into_owned();
            let vertex_count = *vertex_counts.add(index) as usize;
            let index_count = *index_counts.add(index) as usize;
            let pos_ptr = *vertex_positions.add(index);
            let uv_ptr = *vertex_uvs.add(index);
            let index_ptr = *indices.add(index);
            if pos_ptr.is_null() || uv_ptr.is_null() || index_ptr.is_null() {
                continue;
            }
            let positions = std::slice::from_raw_parts(pos_ptr, vertex_count);
            let uvs = std::slice::from_raw_parts(uv_ptr, vertex_count);
            let vertices = positions
                .iter()
                .zip(uvs.iter())
                .map(|(position, uv)| Vertex {
                    position: [position.x, position.y],
                    uv: [uv.x, uv.y],
                })
                .collect::<Vec<_>>();
            let drawable_indices = std::slice::from_raw_parts(index_ptr, index_count).to_vec();
            let drawable_id = DrawableId(id);
            art_meshes.push(ArtMeshInfo {
                id: drawable_id.clone(),
                label: drawable_id.0.clone(),
                original_name: drawable_id.0.clone(),
                index,
                mask_type: "unknown".into(),
            });
            drawables.push(Drawable {
                id: drawable_id,
                render_order: *render_orders.add(index),
                texture_index: (*texture_indices.add(index)).max(0) as usize,
                vertices,
                indices: drawable_indices,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            });
        }
        drawables.sort_by_key(|drawable| drawable.render_order);
        Ok((
            CanvasInfo {
                size: [canvas_size.x, canvas_size.y],
                origin: [canvas_origin.x, canvas_origin.y],
                pixels_per_unit,
            },
            drawables,
            art_meshes,
        ))
    }

    fn load_textures(paths: &[PathBuf]) -> Result<Vec<TextureAsset>, String> {
        paths
            .iter()
            .map(|path| {
                let image = image::open(path)
                    .map_err(|_| "live2d_texture_unreadable")?
                    .into_rgba8();
                let (width, height) = image.dimensions();
                Ok(TextureAsset {
                    width,
                    height,
                    rgba: image.into_raw(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolves_model_files_and_reports_missing_assets() {
        let root = unique_temp_dir("live2d-runtime-files");
        fs::create_dir_all(root.join("textures")).unwrap();
        fs::write(root.join("texture_ok.png"), "").unwrap();
        let model = root.join("sample.model3.json");
        fs::write(
            &model,
            r#"{"FileReferences":{"Moc":"sample.moc3","Textures":["texture_ok.png","textures/missing.png"]}}"#,
        )
        .unwrap();

        let files = resolve_model_files(&model).unwrap();

        assert_eq!(files.moc_path, root.join("sample.moc3"));
        assert_eq!(files.texture_paths.len(), 2);
        assert_eq!(
            files.missing_files,
            vec!["sample.moc3", "textures/missing.png"]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn reports_unavailable_runtime_without_fake_art_meshes() {
        let root = unique_temp_dir("live2d-runtime-unavailable");
        fs::create_dir_all(&root).unwrap();
        let moc = root.join("sample.moc3");
        let model = root.join("sample.model3.json");
        fs::write(&model, r#"{"FileReferences":{"Moc":"sample.moc3"}}"#).unwrap();
        fs::write(&moc, b"noise\0ArtMeshZ\0").unwrap();

        let result = inspect_art_meshes(model);

        assert_eq!(result.unwrap_err(), "live2d_runtime_unavailable");

        fs::remove_dir_all(root).unwrap();
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}"))
    }
}
