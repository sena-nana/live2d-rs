use crate::live2d;
use ::live2d::core::ArtMeshInfo;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInspectRequest {
    pub model_json_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtMeshItem {
    pub id: String,
    pub label: String,
    pub original_name: String,
    pub index: usize,
    pub mask_type: String,
}

impl From<ArtMeshInfo> for ArtMeshItem {
    fn from(value: ArtMeshInfo) -> Self {
        Self {
            id: value.id.0,
            label: value.label,
            original_name: value.original_name,
            index: value.index,
            mask_type: value.mask_type,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInspectResponse {
    pub ok: bool,
    pub available_art_meshes: Vec<ArtMeshItem>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn inspect_model(model_json_path: impl AsRef<Path>) -> ModelInspectResponse {
    match inspect_model_inner(model_json_path.as_ref()) {
        Ok(meshes) => ModelInspectResponse {
            ok: true,
            available_art_meshes: meshes,
            warnings: Vec::new(),
            error: None,
        },
        Err(error) => ModelInspectResponse {
            ok: false,
            available_art_meshes: Vec::new(),
            warnings: vec![error.clone()],
            error: Some(error),
        },
    }
}

pub fn inspect_model_inner(model_json_path: &Path) -> Result<Vec<ArtMeshItem>, String> {
    live2d::inspect_art_meshes(model_json_path)
}

pub fn available_art_mesh_ids(model_json_path: &str) -> Result<BTreeSet<String>, String> {
    Ok(inspect_model_inner(Path::new(model_json_path))?
        .into_iter()
        .map(|mesh| mesh.id)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn reports_unavailable_runtime_without_fake_art_meshes() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nanavts-display-model-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let moc = root.join("sample.moc3");
        let model = root.join("sample.model3.json");
        std::fs::write(&model, r#"{"FileReferences":{"Moc":"sample.moc3"}}"#).unwrap();
        std::fs::write(&moc, b"noise\0ArtMeshZ\0").unwrap();

        let result = inspect_model(model);

        assert!(!result.ok);
        assert_eq!(result.available_art_meshes, Vec::<ArtMeshItem>::new());
        assert_eq!(result.error.as_deref(), Some("live2d_runtime_unavailable"));
        assert_eq!(result.warnings, vec!["live2d_runtime_unavailable"]);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_missing_model() {
        let result = inspect_model("Z:/missing/model.model3.json");

        assert!(!result.ok);
        assert_eq!(result.error.as_deref(), Some("live2d_model_not_found"));
    }
}
