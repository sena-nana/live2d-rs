use crate::{catalog, model};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySession {
    #[serde(default)]
    pub active_channel_index: usize,
    #[serde(default)]
    pub channels: Vec<EffectChannel>,
    #[serde(default)]
    pub effect_parts: Vec<EffectPart>,
    #[serde(default)]
    pub art_mesh_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub live2d_model: Option<Live2dModelResource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectChannel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub render_mode: String,
    #[serde(default)]
    pub part_render_modes: BTreeMap<String, String>,
    #[serde(default)]
    pub parts: Vec<String>,
    #[serde(default)]
    pub art_mesh_ids: Vec<String>,
    #[serde(default)]
    pub mask_art_mesh_ids: Vec<String>,
    #[serde(default)]
    pub values: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EffectPart {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub defaults_on: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Live2dModelResource {
    #[serde(default)]
    pub name: String,
    pub model_json_path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DisplayResponse {
    pub ok: bool,
    pub channel_count: usize,
    pub renderer: &'static str,
    pub model_loaded: bool,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn active_channel(session: &DisplaySession) -> Option<&EffectChannel> {
    if session.channels.is_empty() {
        return None;
    }
    let index = session
        .active_channel_index
        .min(session.channels.len().saturating_sub(1));
    session.channels.get(index)
}

pub fn apply_art_mesh_alias(
    session: Option<&DisplaySession>,
    mesh: &model::ArtMeshItem,
) -> model::ArtMeshItem {
    let Some(alias) = session.and_then(|session| session.art_mesh_aliases.get(&mesh.id)) else {
        return mesh.clone();
    };
    if alias.trim().is_empty() {
        return mesh.clone();
    }
    let mut item = mesh.clone();
    item.label = alias.clone();
    item
}

pub fn merged_values(session: &DisplaySession) -> serde_json::Map<String, Value> {
    let mut values = serde_json::Map::new();
    let Some(channel) = active_channel(session) else {
        return values;
    };

    for part_id in &channel.parts {
        if let Some(part) = session.effect_parts.iter().find(|part| &part.id == part_id) {
            for (key, value) in &part.defaults_on {
                values.insert(key.clone(), value.clone());
            }
        }
    }

    for (key, value) in &channel.values {
        values.insert(key.clone(), value.clone());
    }

    values
}

pub fn preview_color(session: Option<&DisplaySession>) -> [f64; 4] {
    let Some(session) = session else {
        return [0.07, 0.09, 0.12, 1.0];
    };
    let values = merged_values(session);
    let color = read_color(&values).unwrap_or([1.0, 0.74, 0.18, 0.86]);
    let strength = read_float(
        &values,
        &["strength", "intensity", "effectStrength", "mix", "amount"],
    )
    .unwrap_or(0.62)
    .clamp(0.0, 1.0);
    let brightness =
        read_float(&values, &["brightness", "lightness", "glow", "haloGlow"]).unwrap_or(1.08);

    let base = [0.08, 0.10, 0.14];
    [
        ((base[0] * (1.0 - strength)) + color[0] * strength * brightness).clamp(0.0, 1.0),
        ((base[1] * (1.0 - strength)) + color[1] * strength * brightness).clamp(0.0, 1.0),
        ((base[2] * (1.0 - strength)) + color[2] * strength * brightness).clamp(0.0, 1.0),
        1.0,
    ]
}

pub fn validate_session(session: &DisplaySession) -> Result<Vec<String>, String> {
    validate_effect_parts(session)?;

    if let Some(model) = &session.live2d_model {
        if model.model_json_path.trim().is_empty() {
            return Err("invalid_live2d_model_path".into());
        }
        validate_art_meshes(session, &model.model_json_path)?;
    } else if session
        .channels
        .iter()
        .any(|channel| !channel.art_mesh_ids.is_empty() || !channel.mask_art_mesh_ids.is_empty())
    {
        return Err("live2d_model_required_for_art_meshes".into());
    }
    Ok(Vec::new())
}

fn validate_effect_parts(session: &DisplaySession) -> Result<(), String> {
    let known_catalog: HashSet<&str> = catalog::effect_parts()
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    let sent_parts: HashSet<&str> = session
        .effect_parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();

    for part in &session.effect_parts {
        if !known_catalog.contains(part.id.as_str()) {
            return Err(format!("unknown_effect_part:{}", part.id));
        }
    }

    for channel in &session.channels {
        for part_id in &channel.parts {
            if !known_catalog.contains(part_id.as_str()) {
                return Err(format!("unknown_effect_part:{}", part_id));
            }
            if !sent_parts.contains(part_id.as_str()) {
                return Err(format!("missing_effect_part_payload:{}", part_id));
            }
        }
    }
    Ok(())
}

fn validate_art_meshes(session: &DisplaySession, model_json_path: &str) -> Result<(), String> {
    let available = model::available_art_mesh_ids(model_json_path)?;
    for channel in &session.channels {
        for id in channel
            .art_mesh_ids
            .iter()
            .chain(channel.mask_art_mesh_ids.iter())
        {
            if !available.contains(id) {
                return Err(format!("unknown_art_mesh:{}", id));
            }
        }
    }
    Ok(())
}

fn read_float(values: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<f64> {
    for key in keys {
        let value = read_case_insensitive(values, key)?;
        match value {
            Value::Number(number) => return number.as_f64(),
            Value::Bool(value) => return Some(if *value { 1.0 } else { 0.0 }),
            Value::String(value) => {
                if let Ok(parsed) = value.parse::<f64>() {
                    return Some(parsed);
                }
            }
            _ => {}
        }
    }
    None
}

fn read_color(values: &serde_json::Map<String, Value>) -> Option<[f64; 4]> {
    for key in ["color", "tint", "tintColor", "mainColor", "effectColor"] {
        let Some(value) = read_case_insensitive(values, key) else {
            continue;
        };
        match value {
            Value::String(raw) => return parse_hex_color(raw),
            Value::Object(map) => {
                let channel = |name: &str, fallback: f64| {
                    map.get(name)
                        .and_then(Value::as_f64)
                        .unwrap_or(fallback)
                        .clamp(0.0, 1.0)
                };
                return Some([
                    channel("r", 1.0),
                    channel("g", 1.0),
                    channel("b", 1.0),
                    channel("a", 1.0),
                ]);
            }
            _ => {}
        }
    }
    None
}

fn read_case_insensitive<'a>(
    values: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Option<&'a Value> {
    values.get(key).or_else(|| {
        let lower = key.to_ascii_lowercase();
        values
            .iter()
            .find(|(candidate, _)| candidate.to_ascii_lowercase() == lower)
            .map(|(_, value)| value)
    })
}

fn parse_hex_color(raw: &str) -> Option<[f64; 4]> {
    let hex = raw.trim().trim_start_matches('#');
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&hex[range], 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    match hex.len() {
        6 => Some([parse(0..2)?, parse(2..4)?, parse(4..6)?, 1.0]),
        8 => Some([parse(0..2)?, parse(2..4)?, parse(4..6)?, parse(6..8)?]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_part_defaults_before_channel_values() {
        let session = DisplaySession {
            active_channel_index: 0,
            channels: vec![EffectChannel {
                parts: vec!["recolor".into()],
                values: serde_json::Map::from_iter([("strength".into(), json!(0.25))]),
                ..EffectChannel::default()
            }],
            effect_parts: vec![EffectPart {
                id: "recolor".into(),
                defaults_on: serde_json::Map::from_iter([
                    ("strength".into(), json!(0.9)),
                    ("color".into(), json!("#ff0000")),
                ]),
                ..EffectPart::default()
            }],
            ..DisplaySession::default()
        };

        let values = merged_values(&session);
        assert_eq!(values.get("strength"), Some(&json!(0.25)));
        assert_eq!(values.get("color"), Some(&json!("#ff0000")));
    }
}
