use crate::ParameterInfo;
use serde_json::Value;
use std::{collections::HashMap, fs, path::Path};

const AIR_RESISTANCE: f32 = 5.0;
const MAXIMUM_WEIGHT: f32 = 100.0;
const MOVEMENT_THRESHOLD: f32 = 0.001;
const MAX_DELTA_TIME: f32 = 5.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Live2DPhysics {
    gravity: Vec2,
    wind: Vec2,
    fps: f32,
    settings: Vec<PhysicsSetting>,
    current_outputs: Vec<Vec<f32>>,
    previous_outputs: Vec<Vec<f32>>,
    parameter_cache: Vec<f32>,
    parameter_input_cache: Vec<f32>,
    current_remain_time: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicsStats {
    pub settings: usize,
    pub inputs: usize,
    pub outputs: usize,
    pub particles: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysicsEvaluationStats {
    pub output_writes: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct PhysicsSetting {
    normalization_position: Normalization,
    normalization_angle: Normalization,
    inputs: Vec<PhysicsInput>,
    outputs: Vec<PhysicsOutput>,
    particles: Vec<PhysicsParticle>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Normalization {
    minimum: f32,
    maximum: f32,
    default: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct PhysicsInput {
    source_id: String,
    source_index: Option<usize>,
    weight: f32,
    source_type: PhysicsSource,
    reflect: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct PhysicsOutput {
    destination_id: String,
    destination_index: Option<usize>,
    vertex_index: usize,
    scale: f32,
    weight: f32,
    source_type: PhysicsSource,
    reflect: bool,
    value_below_minimum: f32,
    value_exceeded_maximum: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PhysicsSource {
    X,
    Y,
    Angle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PhysicsParticle {
    initial_position: Vec2,
    mobility: f32,
    delay: f32,
    acceleration: f32,
    radius: f32,
    position: Vec2,
    last_position: Vec2,
    last_gravity: Vec2,
    force: Vec2,
    velocity: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Vec2 {
    x: f32,
    y: f32,
}

impl Live2DPhysics {
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|_| "physics_file_unreadable".to_string())?;
        Self::from_json_str(&raw)
    }

    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        let root: Value =
            serde_json::from_str(raw).map_err(|_| "invalid_physics3_json".to_string())?;
        let meta = root
            .get("Meta")
            .ok_or_else(|| "physics_meta_missing".to_string())?;
        let forces = meta.get("EffectiveForces").unwrap_or(&Value::Null);
        let gravity = read_vec2(forces.get("Gravity")).unwrap_or(Vec2 { x: 0.0, y: -1.0 });
        let wind = read_vec2(forces.get("Wind")).unwrap_or_default();
        let fps = read_f32(meta.get("Fps")).unwrap_or(0.0).max(0.0);
        let settings = root
            .get("PhysicsSettings")
            .and_then(Value::as_array)
            .ok_or_else(|| "physics_settings_missing".to_string())?
            .iter()
            .map(parse_setting)
            .collect::<Result<Vec<_>, _>>()?;
        let current_outputs = settings
            .iter()
            .map(|setting| vec![0.0; setting.outputs.len()])
            .collect::<Vec<_>>();
        let previous_outputs = current_outputs.clone();
        let mut physics = Self {
            gravity,
            wind,
            fps,
            settings,
            current_outputs,
            previous_outputs,
            parameter_cache: Vec::new(),
            parameter_input_cache: Vec::new(),
            current_remain_time: 0.0,
        };
        physics.initialize();
        Ok(physics)
    }

    pub fn stats(&self) -> PhysicsStats {
        PhysicsStats {
            settings: self.settings.len(),
            inputs: self
                .settings
                .iter()
                .map(|setting| setting.inputs.len())
                .sum(),
            outputs: self
                .settings
                .iter()
                .map(|setting| setting.outputs.len())
                .sum(),
            particles: self
                .settings
                .iter()
                .map(|setting| setting.particles.len())
                .sum(),
        }
    }

    pub fn reset(&mut self) {
        self.current_remain_time = 0.0;
        self.parameter_cache.clear();
        self.parameter_input_cache.clear();
        for outputs in &mut self.current_outputs {
            outputs.fill(0.0);
        }
        for outputs in &mut self.previous_outputs {
            outputs.fill(0.0);
        }
        self.initialize();
    }

    pub fn evaluate(
        &mut self,
        parameters: &mut [ParameterInfo],
        delta_time_seconds: f32,
    ) -> PhysicsEvaluationStats {
        let delta_time_seconds = if delta_time_seconds.is_finite() {
            delta_time_seconds.max(0.0)
        } else {
            0.0
        };
        if delta_time_seconds <= 0.0 || self.settings.is_empty() {
            return self.empty_evaluation_stats();
        }
        self.resolve_parameter_indices(parameters);
        self.ensure_caches(parameters);
        self.current_remain_time += delta_time_seconds;
        if self.current_remain_time > MAX_DELTA_TIME {
            self.current_remain_time = 0.0;
        }
        let physics_delta_time = if self.fps > 0.0 {
            1.0 / self.fps
        } else {
            delta_time_seconds
        };
        let mut stats = self.empty_evaluation_stats();
        while self.current_remain_time >= physics_delta_time {
            for setting_index in 0..self.settings.len() {
                self.previous_outputs[setting_index]
                    .copy_from_slice(&self.current_outputs[setting_index]);
            }
            let input_weight = physics_delta_time / self.current_remain_time;
            for (index, parameter) in parameters.iter().enumerate() {
                let value = self.parameter_input_cache[index] * (1.0 - input_weight)
                    + parameter.value * input_weight;
                self.parameter_cache[index] = value;
                self.parameter_input_cache[index] = value;
            }
            for setting_index in 0..self.settings.len() {
                stats.output_writes += self.evaluate_setting(setting_index, parameters);
            }
            self.current_remain_time -= physics_delta_time;
        }
        let alpha = if physics_delta_time > 0.0 {
            self.current_remain_time / physics_delta_time
        } else {
            0.0
        };
        stats.output_writes += self.interpolate(parameters, alpha);
        stats
    }

    pub fn evaluate_to_writes(
        &mut self,
        parameters: &[ParameterInfo],
        delta_time_seconds: f32,
    ) -> (Vec<(String, f32)>, PhysicsEvaluationStats) {
        let mut next = parameters.to_vec();
        let stats = self.evaluate(&mut next, delta_time_seconds);
        (changed_parameter_writes(parameters, &next), stats)
    }

    fn initialize(&mut self) {
        for setting in &mut self.settings {
            if setting.particles.is_empty() {
                continue;
            }
            setting.particles[0].initial_position = Vec2::default();
            setting.particles[0].position = Vec2::default();
            setting.particles[0].last_position = Vec2::default();
            setting.particles[0].last_gravity = Vec2 { x: 0.0, y: 1.0 };
            setting.particles[0].velocity = Vec2::default();
            setting.particles[0].force = Vec2::default();
            for index in 1..setting.particles.len() {
                let initial = setting.particles[index - 1].initial_position
                    + Vec2 {
                        x: 0.0,
                        y: setting.particles[index].radius,
                    };
                setting.particles[index].initial_position = initial;
                setting.particles[index].position = initial;
                setting.particles[index].last_position = initial;
                setting.particles[index].last_gravity = Vec2 { x: 0.0, y: 1.0 };
                setting.particles[index].velocity = Vec2::default();
                setting.particles[index].force = Vec2::default();
            }
        }
    }

    fn empty_evaluation_stats(&self) -> PhysicsEvaluationStats {
        PhysicsEvaluationStats { output_writes: 0 }
    }

    fn ensure_caches(&mut self, parameters: &[ParameterInfo]) {
        if self.parameter_cache.len() != parameters.len() {
            self.parameter_cache = parameters.iter().map(|parameter| parameter.value).collect();
        }
        if self.parameter_input_cache.len() != parameters.len() {
            self.parameter_input_cache =
                parameters.iter().map(|parameter| parameter.value).collect();
        }
    }

    fn resolve_parameter_indices(&mut self, parameters: &[ParameterInfo]) {
        let indices = parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| (parameter.id.0.as_str(), index))
            .collect::<HashMap<_, _>>();
        for setting in &mut self.settings {
            for input in &mut setting.inputs {
                input.source_index = indices.get(input.source_id.as_str()).copied();
            }
            for output in &mut setting.outputs {
                output.destination_index = indices.get(output.destination_id.as_str()).copied();
            }
        }
    }

    fn evaluate_setting(&mut self, setting_index: usize, parameters: &[ParameterInfo]) -> usize {
        let setting = &mut self.settings[setting_index];
        if setting.particles.is_empty() {
            return 0;
        }
        let mut total_translation = Vec2::default();
        let mut total_angle = 0.0;
        for input in &setting.inputs {
            let Some(index) = input.source_index else {
                continue;
            };
            let parameter = &parameters[index];
            let value = self.parameter_cache[index];
            apply_input(
                input,
                value,
                parameter.minimum,
                parameter.maximum,
                parameter.default,
                setting.normalization_position,
                setting.normalization_angle,
                &mut total_translation,
                &mut total_angle,
            );
        }
        let rad_angle = (-total_angle).to_radians();
        total_translation = rotate_like_cubism(total_translation, rad_angle);
        let threshold = MOVEMENT_THRESHOLD * setting.normalization_position.maximum;
        update_particles(
            &mut setting.particles,
            total_translation,
            total_angle,
            self.wind,
            threshold,
            if self.fps > 0.0 { 1.0 / self.fps } else { 0.0 },
            AIR_RESISTANCE,
        );
        let mut output_writes = 0;
        for (output_index, output) in setting.outputs.iter_mut().enumerate() {
            let Some(destination_index) = output.destination_index else {
                continue;
            };
            if output.vertex_index < 1 || output.vertex_index >= setting.particles.len() {
                continue;
            }
            let translation = setting.particles[output.vertex_index].position
                - setting.particles[output.vertex_index - 1].position;
            let output_value = output_value(output, translation, &setting.particles, self.gravity);
            self.current_outputs[setting_index][output_index] = output_value;
            update_cached_output_parameter_value(
                &mut self.parameter_cache[destination_index],
                parameters[destination_index].minimum,
                parameters[destination_index].maximum,
                output_value,
                output,
            );
            output_writes += 1;
        }
        output_writes
    }

    fn interpolate(&mut self, parameters: &mut [ParameterInfo], weight: f32) -> usize {
        let mut output_writes = 0;
        for setting_index in 0..self.settings.len() {
            let setting = &mut self.settings[setting_index];
            for (output_index, output) in setting.outputs.iter_mut().enumerate() {
                let Some(destination_index) = output.destination_index else {
                    continue;
                };
                let value = self.previous_outputs[setting_index][output_index] * (1.0 - weight)
                    + self.current_outputs[setting_index][output_index] * weight;
                update_output_parameter_value(&mut parameters[destination_index], value, output);
                output_writes += 1;
            }
        }
        output_writes
    }
}

fn parse_setting(value: &Value) -> Result<PhysicsSetting, String> {
    let normalization = value
        .get("Normalization")
        .ok_or_else(|| "physics_normalization_missing".to_string())?;
    let inputs = value
        .get("Input")
        .and_then(Value::as_array)
        .ok_or_else(|| "physics_inputs_missing".to_string())?
        .iter()
        .map(parse_input)
        .collect::<Result<Vec<_>, _>>()?;
    let outputs = value
        .get("Output")
        .and_then(Value::as_array)
        .ok_or_else(|| "physics_outputs_missing".to_string())?
        .iter()
        .map(parse_output)
        .collect::<Result<Vec<_>, _>>()?;
    let particles = value
        .get("Vertices")
        .and_then(Value::as_array)
        .ok_or_else(|| "physics_vertices_missing".to_string())?
        .iter()
        .map(parse_particle)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PhysicsSetting {
        normalization_position: parse_normalization(
            normalization
                .get("Position")
                .ok_or_else(|| "physics_position_normalization_missing".to_string())?,
        )?,
        normalization_angle: parse_normalization(
            normalization
                .get("Angle")
                .ok_or_else(|| "physics_angle_normalization_missing".to_string())?,
        )?,
        inputs,
        outputs,
        particles,
    })
}

fn parse_normalization(value: &Value) -> Result<Normalization, String> {
    Ok(Normalization {
        minimum: read_f32(value.get("Minimum"))
            .ok_or_else(|| "physics_normalization_minimum_missing".to_string())?,
        maximum: read_f32(value.get("Maximum"))
            .ok_or_else(|| "physics_normalization_maximum_missing".to_string())?,
        default: read_f32(value.get("Default"))
            .ok_or_else(|| "physics_normalization_default_missing".to_string())?,
    })
}

fn parse_input(value: &Value) -> Result<PhysicsInput, String> {
    Ok(PhysicsInput {
        source_id: source_id(value, "Source", "physics_input_source_missing")?,
        source_index: None,
        weight: read_f32(value.get("Weight")).unwrap_or(0.0),
        source_type: parse_source_type(value.get("Type").and_then(Value::as_str))?,
        reflect: value
            .get("Reflect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_output(value: &Value) -> Result<PhysicsOutput, String> {
    Ok(PhysicsOutput {
        destination_id: source_id(value, "Destination", "physics_output_destination_missing")?,
        destination_index: None,
        vertex_index: value
            .get("VertexIndex")
            .and_then(Value::as_u64)
            .ok_or_else(|| "physics_output_vertex_index_missing".to_string())?
            as usize,
        scale: read_f32(value.get("Scale")).unwrap_or(1.0),
        weight: read_f32(value.get("Weight")).unwrap_or(MAXIMUM_WEIGHT),
        source_type: parse_source_type(value.get("Type").and_then(Value::as_str))?,
        reflect: value
            .get("Reflect")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        value_below_minimum: 0.0,
        value_exceeded_maximum: 0.0,
    })
}

fn parse_particle(value: &Value) -> Result<PhysicsParticle, String> {
    let position = read_vec2(value.get("Position")).unwrap_or_default();
    Ok(PhysicsParticle {
        initial_position: position,
        mobility: read_f32(value.get("Mobility")).unwrap_or(0.0),
        delay: read_f32(value.get("Delay")).unwrap_or(0.0),
        acceleration: read_f32(value.get("Acceleration")).unwrap_or(0.0),
        radius: read_f32(value.get("Radius")).unwrap_or(0.0),
        position,
        last_position: position,
        last_gravity: Vec2 { x: 0.0, y: 1.0 },
        force: Vec2::default(),
        velocity: Vec2::default(),
    })
}

fn source_id(value: &Value, key: &str, error: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(|source| source.get("Id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| error.to_string())
}

fn parse_source_type(value: Option<&str>) -> Result<PhysicsSource, String> {
    match value.unwrap_or_default() {
        "X" => Ok(PhysicsSource::X),
        "Y" => Ok(PhysicsSource::Y),
        "Angle" => Ok(PhysicsSource::Angle),
        _ => Err("unsupported_physics_source_type".into()),
    }
}

fn read_vec2(value: Option<&Value>) -> Option<Vec2> {
    let value = value?;
    Some(Vec2 {
        x: read_f32(value.get("X")).unwrap_or(0.0),
        y: read_f32(value.get("Y")).unwrap_or(0.0),
    })
}

fn read_f32(value: Option<&Value>) -> Option<f32> {
    value.and_then(Value::as_f64).map(|value| value as f32)
}

fn apply_input(
    input: &PhysicsInput,
    value: f32,
    minimum: f32,
    maximum: f32,
    default: f32,
    normalization_position: Normalization,
    normalization_angle: Normalization,
    total_translation: &mut Vec2,
    total_angle: &mut f32,
) {
    let weight = input.weight / MAXIMUM_WEIGHT;
    match input.source_type {
        PhysicsSource::X => {
            total_translation.x += normalize_parameter_value(
                value,
                minimum,
                maximum,
                default,
                normalization_position,
                input.reflect,
            ) * weight;
        }
        PhysicsSource::Y => {
            total_translation.y += normalize_parameter_value(
                value,
                minimum,
                maximum,
                default,
                normalization_position,
                input.reflect,
            ) * weight;
        }
        PhysicsSource::Angle => {
            *total_angle += normalize_parameter_value(
                value,
                minimum,
                maximum,
                default,
                normalization_angle,
                input.reflect,
            ) * weight;
        }
    }
}

fn normalize_parameter_value(
    value: f32,
    parameter_minimum: f32,
    parameter_maximum: f32,
    _parameter_default: f32,
    normalization: Normalization,
    is_inverted: bool,
) -> f32 {
    let max_value = parameter_maximum.max(parameter_minimum);
    let min_value = parameter_maximum.min(parameter_minimum);
    let value = value.clamp(min_value, max_value);
    let min_norm_value = normalization.minimum.min(normalization.maximum);
    let max_norm_value = normalization.minimum.max(normalization.maximum);
    let middle_norm_value = normalization.default;
    let middle_value = min_value + ((max_value - min_value).abs() / 2.0);
    let param_value = value - middle_value;
    let mut result = if param_value > 0.0 {
        let norm_len = max_norm_value - middle_norm_value;
        let param_len = max_value - middle_value;
        if param_len != 0.0 {
            param_value * (norm_len / param_len) + middle_norm_value
        } else {
            0.0
        }
    } else if param_value < 0.0 {
        let norm_len = min_norm_value - middle_norm_value;
        let param_len = min_value - middle_value;
        if param_len != 0.0 {
            param_value * (norm_len / param_len) + middle_norm_value
        } else {
            0.0
        }
    } else {
        middle_norm_value
    };
    if !is_inverted {
        result *= -1.0;
    }
    result
}

fn update_particles(
    particles: &mut [PhysicsParticle],
    total_translation: Vec2,
    total_angle: f32,
    wind: Vec2,
    threshold: f32,
    delta_time_seconds: f32,
    air_resistance: f32,
) {
    if particles.is_empty() {
        return;
    }
    particles[0].position = total_translation;
    let mut current_gravity = radian_to_direction(total_angle.to_radians());
    current_gravity.normalize();
    for index in 1..particles.len() {
        let previous_position = particles[index - 1].position;
        particles[index].force = current_gravity * particles[index].acceleration + wind;
        particles[index].last_position = particles[index].position;
        let delay = particles[index].delay * delta_time_seconds * 30.0;
        let mut direction = particles[index].position - previous_position;
        let radian =
            direction_to_radian(particles[index].last_gravity, current_gravity) / air_resistance;
        direction = rotate_like_cubism(direction, radian);
        particles[index].position = previous_position + direction;
        let velocity = particles[index].velocity * delay;
        let force = particles[index].force * delay * delay;
        particles[index].position = particles[index].position + velocity + force;
        let mut new_direction = particles[index].position - previous_position;
        new_direction.normalize();
        particles[index].position = previous_position + new_direction * particles[index].radius;
        if particles[index].position.x.abs() < threshold {
            particles[index].position.x = 0.0;
        }
        if delay != 0.0 {
            particles[index].velocity =
                (particles[index].position - particles[index].last_position) / delay
                    * particles[index].mobility;
        }
        particles[index].force = Vec2::default();
        particles[index].last_gravity = current_gravity;
    }
}

fn output_value(
    output: &PhysicsOutput,
    translation: Vec2,
    particles: &[PhysicsParticle],
    gravity: Vec2,
) -> f32 {
    match output.source_type {
        PhysicsSource::X => {
            let value = translation.x;
            if output.reflect {
                -value
            } else {
                value
            }
        }
        PhysicsSource::Y => {
            let value = translation.y;
            if output.reflect {
                -value
            } else {
                value
            }
        }
        PhysicsSource::Angle => {
            let mut parent_gravity = if output.vertex_index >= 2 {
                particles[output.vertex_index - 1].position
                    - particles[output.vertex_index - 2].position
            } else {
                gravity * -1.0
            };
            if parent_gravity.length_squared() == 0.0 {
                parent_gravity = Vec2 { x: 0.0, y: -1.0 };
            }
            let value = direction_to_radian(parent_gravity, translation);
            if output.reflect {
                -value
            } else {
                value
            }
        }
    }
}

fn update_output_parameter_value(
    parameter: &mut ParameterInfo,
    translation: f32,
    output: &mut PhysicsOutput,
) {
    let mut value = scaled_output_value(translation, output);
    if value < parameter.minimum {
        if value < output.value_below_minimum {
            output.value_below_minimum = value;
        }
        value = parameter.minimum;
    } else if value > parameter.maximum {
        if value > output.value_exceeded_maximum {
            output.value_exceeded_maximum = value;
        }
        value = parameter.maximum;
    }
    let weight = output.weight / MAXIMUM_WEIGHT;
    parameter.value = if weight >= 1.0 {
        value
    } else {
        parameter.value * (1.0 - weight) + value * weight
    };
}

fn update_cached_output_parameter_value(
    parameter_value: &mut f32,
    minimum: f32,
    maximum: f32,
    translation: f32,
    output: &mut PhysicsOutput,
) {
    let mut value = scaled_output_value(translation, output);
    if value < minimum {
        if value < output.value_below_minimum {
            output.value_below_minimum = value;
        }
        value = minimum;
    } else if value > maximum {
        if value > output.value_exceeded_maximum {
            output.value_exceeded_maximum = value;
        }
        value = maximum;
    }
    let weight = output.weight / MAXIMUM_WEIGHT;
    *parameter_value = if weight >= 1.0 {
        value
    } else {
        *parameter_value * (1.0 - weight) + value * weight
    };
}

fn scaled_output_value(translation: f32, output: &PhysicsOutput) -> f32 {
    translation * output.scale
}

fn changed_parameter_writes(
    before: &[ParameterInfo],
    after: &[ParameterInfo],
) -> Vec<(String, f32)> {
    before
        .iter()
        .zip(after.iter())
        .filter_map(|(before, after)| {
            ((before.value - after.value).abs() > f32::EPSILON)
                .then(|| (after.id.0.clone(), after.value))
        })
        .collect()
}

fn rotate_like_cubism(value: Vec2, radian: f32) -> Vec2 {
    let x = value.x * radian.cos() - value.y * radian.sin();
    let y = x * radian.sin() + value.y * radian.cos();
    Vec2 { x, y }
}

fn direction_to_radian(from: Vec2, to: Vec2) -> f32 {
    let mut result = to.y.atan2(to.x) - from.y.atan2(from.x);
    while result < -std::f32::consts::PI {
        result += std::f32::consts::PI * 2.0;
    }
    while result > std::f32::consts::PI {
        result -= std::f32::consts::PI * 2.0;
    }
    result
}

fn radian_to_direction(radian: f32) -> Vec2 {
    Vec2 {
        x: radian.sin(),
        y: radian.cos(),
    }
}

impl Vec2 {
    fn length_squared(self) -> f32 {
        self.x * self.x + self.y * self.y
    }

    fn normalize(&mut self) {
        let length = self.length_squared().sqrt();
        if length != 0.0 {
            self.x /= length;
            self.y /= length;
        }
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;

    fn mul(self, rhs: f32) -> Self::Output {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

impl std::ops::Div<f32> for Vec2 {
    type Output = Self;

    fn div(self, rhs: f32) -> Self::Output {
        Self {
            x: self.x / rhs,
            y: self.y / rhs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_physics_stats() {
        let physics = Live2DPhysics::from_json_str(minimal_physics_json()).unwrap();
        assert_eq!(
            physics.stats(),
            PhysicsStats {
                settings: 1,
                inputs: 1,
                outputs: 1,
                particles: 2,
            }
        );
    }

    #[test]
    fn evaluation_changes_destination_parameter() {
        let mut physics = Live2DPhysics::from_json_str(minimal_physics_json()).unwrap();
        let mut parameters = test_parameters(1.0);
        let stats = physics.evaluate(&mut parameters, 1.0 / 30.0);
        assert!(stats.output_writes > 0);
        physics.evaluate(&mut parameters, 1.0 / 30.0);
        let output = parameters
            .iter()
            .find(|parameter| parameter.id.as_ref() == "ParamHair")
            .unwrap();
        assert_ne!(output.value, 0.0);
    }

    #[test]
    fn reset_makes_fixed_input_deterministic() {
        let mut physics = Live2DPhysics::from_json_str(minimal_physics_json()).unwrap();
        let mut first = test_parameters(1.0);
        physics.evaluate(&mut first, 1.0 / 30.0);
        let first_output = first
            .iter()
            .find(|parameter| parameter.id.as_ref() == "ParamHair")
            .unwrap()
            .value;

        physics.reset();
        let mut second = test_parameters(1.0);
        physics.evaluate(&mut second, 1.0 / 30.0);
        let second_output = second
            .iter()
            .find(|parameter| parameter.id.as_ref() == "ParamHair")
            .unwrap()
            .value;
        assert!((first_output - second_output).abs() < 0.0001);
    }

    fn test_parameters(input: f32) -> Vec<ParameterInfo> {
        vec![
            ParameterInfo {
                id: crate::ParameterId::from("ParamAngleX"),
                minimum: -1.0,
                maximum: 1.0,
                default: 0.0,
                value: input,
            },
            ParameterInfo {
                id: crate::ParameterId::from("ParamHair"),
                minimum: -30.0,
                maximum: 30.0,
                default: 0.0,
                value: 0.0,
            },
        ]
    }

    fn minimal_physics_json() -> &'static str {
        r#"{
            "Meta": {
                "EffectiveForces": {
                    "Gravity": { "X": 0, "Y": -1 },
                    "Wind": { "X": 0.2, "Y": 0 }
                },
                "Fps": 30
            },
            "PhysicsSettings": [{
                "Normalization": {
                    "Position": { "Minimum": -10, "Maximum": 10, "Default": 0 },
                    "Angle": { "Minimum": -30, "Maximum": 30, "Default": 0 }
                },
                "Input": [{
                    "Source": { "Id": "ParamAngleX" },
                    "Weight": 100,
                    "Type": "X",
                    "Reflect": false
                }],
                "Output": [{
                    "Destination": { "Id": "ParamHair" },
                    "VertexIndex": 1,
                    "Scale": 30,
                    "Weight": 100,
                    "Type": "Angle",
                    "Reflect": false
                }],
                "Vertices": [
                    { "Position": { "X": 0, "Y": 0 }, "Mobility": 1, "Delay": 1, "Acceleration": 1, "Radius": 0 },
                    { "Position": { "X": 0, "Y": 1 }, "Mobility": 1, "Delay": 1, "Acceleration": 1, "Radius": 1 }
                ]
            }]
        }"#
    }
}
