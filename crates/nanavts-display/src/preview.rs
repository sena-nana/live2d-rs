use crate::{
    catalog,
    session::{self, DisplaySession},
};
use live2d::wgpu::WgpuPreviewUniform;
use serde_json::{Map, Value};
use std::collections::HashSet;

pub type PreviewUniform = WgpuPreviewUniform;

pub fn preview_uniform_from_session(
    session: Option<&DisplaySession>,
    time_seconds: f32,
    width: u32,
    height: u32,
) -> PreviewUniform {
    let data = active_part_data(session);
    let values = &data.values;
    let mut uniform = PreviewUniform::neutral(time_seconds, width, height);

    uniform.tint_a = read_color(values, "tint", [1.0, 1.0, 1.0, 1.0]);
    uniform.tint_b = read_color(values, "tint2", [1.0, 1.0, 1.0, 1.0]);
    uniform.grad_lo = read_color(values, "grad_lo", [0.0, 0.0, 0.0, 1.0]);
    uniform.grad_hi = read_color(values, "grad_hi", [1.0, 1.0, 1.0, 1.0]);
    uniform.ptcl_color = read_color(values, "ptcl_color", [1.0, 1.0, 1.0, 1.0]);
    uniform.damage_fray_color = read_color(values, "damage_fray_color", [0.92, 0.88, 0.80, 1.0]);

    uniform.params0 = [
        read_f32(values, "strength", 0.0).clamp(0.0, 1.0),
        read_f32(values, "brightness", 1.0).clamp(0.0, 2.0),
        enabled(&data, "flow"),
        read_f32(values, "flow_speed", 1.0).clamp(0.0, 5.0),
    ];
    uniform.params1 = [
        read_f32(values, "flow_scale", 1.0).clamp(0.0, 5.0),
        read_f32(values, "grad_amount", 0.0).clamp(0.0, 1.0),
        read_f32(values, "hue", 0.0).clamp(-180.0, 180.0),
        read_f32(values, "hue_cycle", 0.0).clamp(-180.0, 180.0),
    ];
    uniform.params2 = [
        read_f32(values, "posterize", 0.0).clamp(0.0, 16.0),
        read_f32(values, "pulse_amount", 0.0).clamp(0.0, 1.0),
        read_f32(values, "pulse_speed", 2.0).clamp(0.0, 8.0),
        read_f32(values, "saturation", 1.0).clamp(0.0, 2.0),
    ];
    uniform.params3 = [
        read_f32(values, "contrast", 1.0).clamp(0.0, 2.0),
        read_f32(values, "opacity", 1.0).clamp(0.0, 1.0),
        read_f32(values, "scale", 1.0).clamp(0.1, 3.0),
        read_f32(values, "sphere_progress", 0.0).clamp(0.0, 1.0),
    ];
    uniform.params4 = [
        read_f32(values, "sphere_angle_z", 0.0).clamp(-180.0, 180.0),
        read_f32(values, "sphere_speed_z", 0.0).clamp(-360.0, 360.0),
        read_f32(values, "sphere_shade", 0.35).clamp(0.0, 1.0),
        read_f32(values, "dissolve_progress", 0.0).clamp(0.0, 1.0),
    ];
    uniform.params5 = [
        read_f32(values, "dissolve_block_size", 0.12).clamp(0.02, 0.5),
        read_f32(values, "dissolve_glow", 1.25).max(0.0),
        enabled(&data, "particle"),
        read_f32(values, "ptcl_shape", 2.0).clamp(0.0, 3.0),
    ];
    uniform.params6 = [
        read_f32(values, "ptcl_density", 1.0).clamp(0.0, 5.0),
        read_f32(values, "ptcl_size", 0.2).clamp(0.0, 1.0),
        read_f32(values, "ptcl_speed", 1.0).clamp(0.0, 5.0),
        read_f32(values, "damage_amount", 0.0).clamp(0.0, 1.0),
    ];
    uniform.params7 = [
        read_f32(values, "damage_count", 18.0).clamp(1.0, 24.0),
        read_f32(values, "damage_spread", 0.15).clamp(0.0, 0.5),
        read_f32(values, "damage_size", 0.65).clamp(0.0, 1.0),
        read_f32(values, "damage_corex", 0.0).clamp(-0.25, 0.25),
    ];
    uniform.params8 = [
        read_f32(values, "damage_corey", 0.0).clamp(-0.25, 0.25),
        read_f32(values, "damage_falloff", 0.5).clamp(0.0, 1.0),
        read_f32(values, "damage_elong", 0.6).clamp(0.1, 1.5),
        read_f32(values, "damage_angle", 0.0).rem_euclid(360.0),
    ];
    uniform.params9 = [
        read_f32(values, "damage_ragged", 0.4).clamp(0.0, 1.0),
        read_f32(values, "damage_fray", 0.4).clamp(0.0, 1.0),
        read_f32(values, "damage_seed", 0.0),
        enabled(&data, "damage"),
    ];
    uniform
}

struct PreviewData {
    values: Map<String, Value>,
    enabled_parts: HashSet<String>,
}

fn active_part_data(session: Option<&DisplaySession>) -> PreviewData {
    let mut values = Map::new();
    let mut enabled_parts = HashSet::new();
    let Some(session) = session else {
        return PreviewData {
            values,
            enabled_parts,
        };
    };
    let Some(channel) = session::active_channel(session) else {
        return PreviewData {
            values,
            enabled_parts,
        };
    };
    let preview_mode = normalize_render_mode(&channel.render_mode);

    let mut accepted_keys = HashSet::new();
    for part_id in &channel.parts {
        if !matches_preview_mode(channel, part_id, preview_mode) {
            continue;
        }
        enabled_parts.insert(part_id.clone());
        if let Some(part) = session.effect_parts.iter().find(|part| &part.id == part_id) {
            for (key, value) in &part.defaults_on {
                accepted_keys.insert(key.clone());
                values.insert(key.clone(), value.clone());
            }
        }
    }
    for catalog_part in catalog::effect_parts() {
        if !channel.parts.iter().any(|part| part == &catalog_part.id) {
            continue;
        }
        if !matches_preview_mode(channel, &catalog_part.id, preview_mode) {
            continue;
        }
        for param in &catalog_part.params {
            accepted_keys.insert(param.key.clone());
            values
                .entry(param.key.clone())
                .or_insert_with(|| param.default_value.clone());
        }
    }
    for (key, value) in &channel.values {
        if accepted_keys.contains(key) {
            values.insert(key.clone(), value.clone());
        }
    }
    PreviewData {
        values,
        enabled_parts,
    }
}

fn matches_preview_mode(
    channel: &session::EffectChannel,
    part_id: &str,
    preview_mode: Option<&'static str>,
) -> bool {
    preview_mode
        .map(|mode| resolve_part_render_mode(channel, part_id) == mode)
        .unwrap_or(true)
}

fn resolve_part_render_mode(channel: &session::EffectChannel, part_id: &str) -> &'static str {
    if let Some(mode) = channel
        .part_render_modes
        .get(part_id)
        .and_then(|value| normalize_render_mode(value))
    {
        return mode;
    }

    catalog::effect_parts()
        .iter()
        .find(|part| part.id == part_id)
        .and_then(|part| normalize_render_mode(&part.path))
        .unwrap_or("Inline")
}

fn normalize_render_mode(value: &str) -> Option<&'static str> {
    if value.eq_ignore_ascii_case("surface") {
        Some("Surface")
    } else if value.eq_ignore_ascii_case("inline") {
        Some("Inline")
    } else {
        None
    }
}

fn enabled(data: &PreviewData, part_id: &str) -> f32 {
    if data.enabled_parts.contains(part_id) {
        1.0
    } else {
        0.0
    }
}

fn read_f32(values: &Map<String, Value>, key: &str, fallback: f32) -> f32 {
    match values.get(key) {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(fallback as f64) as f32,
        Some(Value::Bool(value)) => {
            if *value {
                1.0
            } else {
                0.0
            }
        }
        Some(Value::String(value)) => value.parse::<f32>().unwrap_or(fallback),
        _ => fallback,
    }
}

fn read_color(values: &Map<String, Value>, key: &str, fallback: [f32; 4]) -> [f32; 4] {
    match values.get(key) {
        Some(Value::String(raw)) => parse_hex_color(raw).unwrap_or(fallback),
        Some(Value::Object(map)) => [
            read_object_f32(map, "r", fallback[0]).clamp(0.0, 1.0),
            read_object_f32(map, "g", fallback[1]).clamp(0.0, 1.0),
            read_object_f32(map, "b", fallback[2]).clamp(0.0, 1.0),
            read_object_f32(map, "a", fallback[3]).clamp(0.0, 1.0),
        ],
        _ => fallback,
    }
}

fn read_object_f32(map: &Map<String, Value>, key: &str, fallback: f32) -> f32 {
    map.get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(fallback)
}

fn parse_hex_color(raw: &str) -> Option<[f32; 4]> {
    let hex = raw.trim().trim_start_matches('#');
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    let a = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).ok()? as f32 / 255.0
    } else {
        1.0
    };
    Some([r, g, b, a])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{DisplaySession, EffectChannel, EffectPart};
    use serde_json::json;

    #[test]
    fn maps_all_colorful_parts_to_non_neutral_uniform_fields() {
        let parts = crate::catalog::effect_parts()
            .iter()
            .map(|part| EffectPart {
                id: part.id.clone(),
                label: part.label.clone(),
                defaults_on: part.defaults_on.clone(),
            })
            .collect::<Vec<_>>();
        let session = DisplaySession {
            channels: vec![EffectChannel {
                parts: parts.iter().map(|part| part.id.clone()).collect(),
                values: serde_json::from_value(json!({
                    "tint": "#ff0000",
                    "tint2": "#00ff00",
                    "hue": 45,
                    "saturation": 1.4,
                    "contrast": 1.2,
                    "pulse_amount": 0.5,
                    "opacity": 0.7,
                    "scale": 1.3,
                    "posterize": 5,
                    "grad_amount": 0.8,
                    "dissolve_progress": 0.4,
                    "sphere_progress": 0.9,
                    "ptcl_density": 2.0,
                    "damage_amount": 0.6
                }))
                .unwrap(),
                ..Default::default()
            }],
            effect_parts: parts,
            ..Default::default()
        };

        let uniform = preview_uniform_from_session(Some(&session), 1.5, 800, 600);

        assert_eq!(uniform.tint_a[0], 1.0);
        assert_eq!(uniform.params0[2], 1.0);
        assert_eq!(uniform.params1[2], 45.0);
        assert_eq!(uniform.params2[3], 1.4);
        assert_eq!(uniform.params3[1], 0.7);
        assert_eq!(uniform.params4[3], 0.4);
        assert_eq!(uniform.params5[2], 1.0);
        assert_eq!(uniform.params6[3], 0.6);
        assert_eq!(uniform.params9[3], 1.0);
    }

    #[test]
    fn ignores_stale_values_for_disabled_parts() {
        let session = DisplaySession {
            channels: vec![EffectChannel {
                parts: vec!["recolor".into()],
                values: serde_json::from_value(json!({
                    "strength": 0.5,
                    "damage_amount": 1.0,
                    "ptcl_density": 5.0
                }))
                .unwrap(),
                ..Default::default()
            }],
            effect_parts: vec![EffectPart {
                id: "recolor".into(),
                label: "换色".into(),
                defaults_on: serde_json::from_value(json!({ "strength": 1.0 })).unwrap(),
            }],
            ..Default::default()
        };

        let uniform = preview_uniform_from_session(Some(&session), 0.0, 1, 1);

        assert_eq!(uniform.params0[0], 0.5);
        assert_eq!(uniform.params5[2], 0.0);
        assert_eq!(uniform.params6[3], 0.0);
        assert_eq!(uniform.params9[3], 0.0);
    }

    #[test]
    fn filters_parts_by_channel_preview_render_mode() {
        let session = DisplaySession {
            channels: vec![EffectChannel {
                render_mode: "Surface".into(),
                parts: vec!["recolor".into(), "hue".into()],
                part_render_modes: std::iter::once(("hue".into(), "Surface".into())).collect(),
                values: serde_json::from_value(json!({
                    "strength": 0.5,
                    "hue": 45
                }))
                .unwrap(),
                ..Default::default()
            }],
            effect_parts: vec![
                EffectPart {
                    id: "recolor".into(),
                    label: "换色".into(),
                    defaults_on: serde_json::from_value(json!({ "strength": 1.0 })).unwrap(),
                },
                EffectPart {
                    id: "hue".into(),
                    label: "色相".into(),
                    defaults_on: serde_json::Map::new(),
                },
            ],
            ..Default::default()
        };

        let uniform = preview_uniform_from_session(Some(&session), 0.0, 1, 1);

        assert_eq!(uniform.params0[0], 0.0);
        assert_eq!(uniform.params1[2], 45.0);
    }
}
