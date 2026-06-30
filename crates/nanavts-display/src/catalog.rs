use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

const COLORFUL_EFFECTS_JSON: &str = include_str!("../assets/colorful-effects.json");
static EFFECT_PARTS: OnceLock<Vec<EffectPartSpec>> = OnceLock::new();

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectPartSpec {
    pub id: String,
    pub label: String,
    pub path: String,
    #[serde(default)]
    pub params: Vec<EffectParamSpec>,
    #[serde(default)]
    pub defaults_on: serde_json::Map<String, Value>,
    #[serde(default)]
    pub defaults_off: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectParamSpec {
    pub kind: String,
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub shader_prop: String,
    #[serde(default)]
    pub default_value: Value,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub options: Option<Vec<String>>,
}

pub fn effect_parts() -> &'static [EffectPartSpec] {
    EFFECT_PARTS.get_or_init(|| {
        serde_json::from_str(COLORFUL_EFFECTS_JSON).expect("valid Colorful effect catalog")
    })
}

pub fn effect_schema() -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "schema": {
            "moduleId": "effects.runtime",
            "title": "效果运行时",
            "sections": [{
                "id": "channels",
                "title": "通道",
                "fields": [
                    { "key": "optimizerEnabled", "kind": "toggle", "label": "启用优化器", "path": "optimizerEnabled", "defaultValue": false },
                    { "key": "recycleInvisibleMeshResources", "kind": "toggle", "label": "回收不可见网格资源", "path": "recycleInvisibleMeshResources", "defaultValue": false }
                ]
            }],
            "actions": [],
            "meta": { "parts": effect_parts() }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_colorful_effects() {
        let parts = effect_parts();
        assert!(parts.len() >= 10);
        assert!(parts.iter().any(|part| part.id == "recolor"));
        assert!(parts
            .iter()
            .all(|part| !part.params.is_empty() || part.id == "flow"));
    }
}
