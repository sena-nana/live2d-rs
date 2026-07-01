use serde_json::Value;
use std::{collections::VecDeque, fs, path::Path};

#[derive(Debug, Clone, PartialEq)]
pub struct Live2DMotion {
    duration: f32,
    looped: bool,
    curves: Vec<MotionCurve>,
    events: Vec<MotionEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvaluation {
    pub model_opacity: Option<f32>,
    pub parameters: Vec<(String, f32)>,
    pub part_opacities: Vec<(String, f32)>,
    pub events: Vec<MotionEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvent {
    pub time_seconds: f32,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionStartResult {
    Started,
    Queued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct MotionPriority(pub u8);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionPlayOptions {
    pub loop_playback: bool,
    pub fade_in_seconds: f32,
    pub fade_out_seconds: f32,
    pub speed: f32,
    pub priority: MotionPriority,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MotionLayerId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionBlendMode {
    Override,
    Additive,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionLayerOptions {
    pub weight: f32,
    pub enabled: bool,
    pub blend: MotionBlendMode,
}

#[derive(Debug, Clone)]
pub struct MotionPlayer {
    active: Option<MotionTrack>,
    fading_out: Option<MotionTrack>,
    queued: VecDeque<QueuedMotion>,
    idle: Option<QueuedMotion>,
    scratch: MotionEvaluation,
    state: MotionPlaybackState,
}

#[derive(Debug, Clone)]
pub struct MotionMixer {
    primary: MotionPlayer,
    layers: Vec<MotionLayer>,
}

#[derive(Debug, Clone)]
struct MotionTrack {
    motion: Live2DMotion,
    elapsed_seconds: f32,
    fade_elapsed_seconds: f32,
    options: MotionPlayOptions,
}

#[derive(Debug, Clone)]
struct QueuedMotion {
    motion: Live2DMotion,
    options: MotionPlayOptions,
}

#[derive(Debug, Clone)]
struct MotionLayer {
    id: MotionLayerId,
    player: MotionPlayer,
    options: MotionLayerOptions,
}

#[derive(Debug, Clone, PartialEq)]
struct MotionCurve {
    target: MotionTarget,
    id: String,
    initial: MotionPoint,
    segments: Vec<MotionSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionTarget {
    Model,
    Parameter,
    PartOpacity,
}

#[derive(Debug, Clone, PartialEq)]
struct MotionSegment {
    kind: MotionSegmentKind,
    points: Vec<MotionPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionSegmentKind {
    Linear,
    Bezier,
    Stepped,
    InverseStepped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct MotionPoint {
    time: f32,
    value: f32,
}

impl Live2DMotion {
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let raw = fs::read_to_string(path).map_err(|_| "motion_file_unreadable".to_string())?;
        Self::from_json_str(&raw)
    }

    pub fn from_json_str(raw: &str) -> Result<Self, String> {
        let root: Value =
            serde_json::from_str(raw).map_err(|_| "invalid_motion3_json".to_string())?;
        let meta = root
            .get("Meta")
            .ok_or_else(|| "motion_meta_missing".to_string())?;
        let duration = read_f32(meta.get("Duration")).unwrap_or(0.0).max(0.0);
        let looped = meta.get("Loop").and_then(Value::as_bool).unwrap_or(false);
        let curves = root
            .get("Curves")
            .and_then(Value::as_array)
            .ok_or_else(|| "motion_curves_missing".to_string())?
            .iter()
            .map(parse_curve)
            .collect::<Result<Vec<_>, _>>()?;
        let events = parse_events(&root)?;
        Ok(Self {
            duration,
            looped,
            curves,
            events,
        })
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn looped(&self) -> bool {
        self.looped
    }

    pub fn events(&self) -> &[MotionEvent] {
        &self.events
    }

    pub fn is_finished(&self, elapsed_seconds: f32, loop_playback: bool) -> bool {
        !self.should_loop(loop_playback) && self.duration > 0.0 && elapsed_seconds >= self.duration
    }

    pub fn sample(&self, elapsed_seconds: f32, loop_playback: bool) -> MotionEvaluation {
        let mut evaluation = MotionEvaluation::default();
        self.sample_into(elapsed_seconds, loop_playback, &mut evaluation);
        evaluation
    }

    pub fn sample_into(
        &self,
        elapsed_seconds: f32,
        loop_playback: bool,
        evaluation: &mut MotionEvaluation,
    ) {
        evaluation.clear();
        let time = self.sample_time(elapsed_seconds, loop_playback);
        for curve in &self.curves {
            let value = evaluate_curve(curve, time);
            match curve.target {
                MotionTarget::Model if curve.id == "Opacity" => {
                    evaluation.model_opacity = Some(value)
                }
                MotionTarget::Parameter => evaluation.parameters.push((curve.id.clone(), value)),
                MotionTarget::PartOpacity => {
                    evaluation.part_opacities.push((curve.id.clone(), value))
                }
                MotionTarget::Model => {}
            }
        }
    }

    fn sample_time(&self, elapsed_seconds: f32, loop_playback: bool) -> f32 {
        let elapsed = elapsed_seconds.max(0.0);
        if self.should_loop(loop_playback) && self.duration > 0.0 {
            elapsed % self.duration
        } else if self.duration > 0.0 {
            elapsed.min(self.duration)
        } else {
            elapsed
        }
    }

    fn should_loop(&self, loop_playback: bool) -> bool {
        loop_playback || self.looped
    }
}

impl Default for MotionEvaluation {
    fn default() -> Self {
        Self {
            model_opacity: None,
            parameters: Vec::new(),
            part_opacities: Vec::new(),
            events: Vec::new(),
        }
    }
}

impl MotionEvaluation {
    pub fn clear(&mut self) {
        self.model_opacity = None;
        self.parameters.clear();
        self.part_opacities.clear();
        self.events.clear();
    }

    pub fn has_motion_values(&self) -> bool {
        self.model_opacity.is_some()
            || !self.parameters.is_empty()
            || !self.part_opacities.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.has_motion_values() && self.events.is_empty()
    }

    fn add_weighted(&mut self, source: &MotionEvaluation, weight: f32) {
        if weight <= 0.0 {
            return;
        }
        if let Some(opacity) = source.model_opacity {
            self.model_opacity = Some(self.model_opacity.unwrap_or(0.0) + opacity * weight);
        }
        add_weighted_values(&mut self.parameters, &source.parameters, weight);
        add_weighted_values(&mut self.part_opacities, &source.part_opacities, weight);
    }

    fn apply_layer(&mut self, source: &MotionEvaluation, options: MotionLayerOptions) {
        if !options.enabled {
            return;
        }
        self.events.extend(source.events.iter().cloned());
        let weight = finite_non_negative(options.weight);
        if weight <= 0.0 {
            return;
        }
        match options.blend {
            MotionBlendMode::Override => self.add_override(source, weight),
            MotionBlendMode::Additive => self.add_additive(source, weight),
        }
    }

    fn add_override(&mut self, source: &MotionEvaluation, weight: f32) {
        let weight = weight.clamp(0.0, 1.0);
        if weight <= 0.0 {
            return;
        }
        if let Some(opacity) = source.model_opacity {
            self.model_opacity = Some(blend_optional_opacity(self.model_opacity, opacity, weight));
        }
        add_override_values(&mut self.parameters, &source.parameters, weight);
        add_override_values(&mut self.part_opacities, &source.part_opacities, weight);
    }

    fn add_additive(&mut self, source: &MotionEvaluation, weight: f32) {
        add_weighted_values(&mut self.parameters, &source.parameters, weight);
        let override_weight = weight.clamp(0.0, 1.0);
        if let Some(opacity) = source.model_opacity {
            self.model_opacity = Some(blend_optional_opacity(
                self.model_opacity,
                opacity,
                override_weight,
            ));
        }
        add_override_values(
            &mut self.part_opacities,
            &source.part_opacities,
            override_weight,
        );
    }
}

impl Default for MotionPlayOptions {
    fn default() -> Self {
        Self {
            loop_playback: false,
            fade_in_seconds: 0.0,
            fade_out_seconds: 0.0,
            speed: 1.0,
            priority: MotionPriority::NORMAL,
        }
    }
}

impl MotionPriority {
    pub const IDLE: Self = Self(0);
    pub const NORMAL: Self = Self(100);
    pub const FORCE: Self = Self(200);
}

impl MotionPlayOptions {
    pub fn looped(loop_playback: bool) -> Self {
        Self {
            loop_playback,
            ..Self::default()
        }
    }

    fn normalized(self) -> Self {
        Self {
            loop_playback: self.loop_playback,
            fade_in_seconds: finite_non_negative(self.fade_in_seconds),
            fade_out_seconds: finite_non_negative(self.fade_out_seconds),
            speed: if self.speed.is_finite() {
                self.speed.max(0.0)
            } else {
                1.0
            },
            priority: self.priority,
        }
    }
}

impl From<String> for MotionLayerId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for MotionLayerId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for MotionLayerId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Default for MotionLayerOptions {
    fn default() -> Self {
        Self {
            weight: 1.0,
            enabled: true,
            blend: MotionBlendMode::Override,
        }
    }
}

impl MotionLayerOptions {
    fn normalized(self) -> Self {
        Self {
            weight: finite_non_negative(self.weight),
            enabled: self.enabled,
            blend: self.blend,
        }
    }
}

impl Default for MotionPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionPlayer {
    pub fn new() -> Self {
        Self {
            active: None,
            fading_out: None,
            queued: VecDeque::new(),
            idle: None,
            scratch: MotionEvaluation::default(),
            state: MotionPlaybackState::Stopped,
        }
    }

    pub fn play(&mut self, motion: Live2DMotion, loop_playback: bool) {
        self.play_with_options(motion, MotionPlayOptions::looped(loop_playback));
    }

    pub fn play_with_options(&mut self, motion: Live2DMotion, options: MotionPlayOptions) {
        let options = options.normalized();
        self.start_now(QueuedMotion { motion, options });
    }

    pub fn request_motion(
        &mut self,
        motion: Live2DMotion,
        options: MotionPlayOptions,
    ) -> MotionStartResult {
        let options = options.normalized();
        let request = QueuedMotion { motion, options };
        if self.can_start_priority(options.priority) {
            self.start_now(request);
            MotionStartResult::Started
        } else {
            self.enqueue(request);
            MotionStartResult::Queued
        }
    }

    pub fn queue_motion(&mut self, motion: Live2DMotion, options: MotionPlayOptions) {
        let options = options.normalized();
        self.enqueue(QueuedMotion { motion, options });
        if matches!(
            self.state,
            MotionPlaybackState::Stopped | MotionPlaybackState::Finished
        ) {
            self.start_next_motion();
        }
    }

    pub fn set_idle_motion(&mut self, motion: Live2DMotion, options: MotionPlayOptions) {
        let mut options = options.normalized();
        options.priority = MotionPriority::IDLE;
        options.loop_playback = true;
        self.idle = Some(QueuedMotion { motion, options });
        if matches!(
            self.state,
            MotionPlaybackState::Stopped | MotionPlaybackState::Finished
        ) && self.active.is_none()
        {
            self.start_next_motion();
        }
    }

    pub fn clear_idle_motion(&mut self) {
        self.idle = None;
    }

    pub fn queued_motion_count(&self) -> usize {
        self.queued.len()
    }

    pub fn pause(&mut self) {
        if self.state == MotionPlaybackState::Playing {
            self.state = MotionPlaybackState::Paused;
        }
    }

    pub fn resume(&mut self) {
        if self.state == MotionPlaybackState::Paused {
            self.state = MotionPlaybackState::Playing;
        }
    }

    pub fn stop(&mut self) {
        self.active = None;
        self.fading_out = None;
        self.queued.clear();
        self.state = MotionPlaybackState::Stopped;
    }

    pub fn stop_with_fade(&mut self, fade_out_seconds: f32) {
        let fade_out_seconds = finite_non_negative(fade_out_seconds);
        self.fading_out = self.active.take().and_then(|mut active| {
            active.options.fade_out_seconds = fade_out_seconds;
            active.into_fade_out()
        });
        self.state = if self.fading_out.is_some() {
            MotionPlaybackState::Playing
        } else {
            MotionPlaybackState::Stopped
        };
    }

    pub fn seek(&mut self, elapsed_seconds: f32) {
        let was_finished = self.state == MotionPlaybackState::Finished;
        let Some(active) = &mut self.active else {
            return;
        };
        active.elapsed_seconds = active.clamp_elapsed(elapsed_seconds.max(0.0));
        active.fade_elapsed_seconds = active.elapsed_seconds.min(active.options.fade_in_seconds);
        self.update_finished_state();
        if was_finished && self.state != MotionPlaybackState::Finished {
            self.state = MotionPlaybackState::Paused;
        }
    }

    pub fn set_loop(&mut self, loop_playback: bool) {
        if let Some(active) = &mut self.active {
            active.options.loop_playback = loop_playback;
        }
    }

    pub fn set_speed(&mut self, speed: f32) {
        if let Some(active) = &mut self.active {
            active.options.speed = if speed.is_finite() {
                speed.max(0.0)
            } else {
                1.0
            };
        }
    }

    pub fn state(&self) -> MotionPlaybackState {
        self.state
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.active
            .as_ref()
            .map(|track| track.elapsed_seconds)
            .unwrap_or(0.0)
    }

    pub fn evaluate(&self) -> Option<MotionEvaluation> {
        let mut evaluation = MotionEvaluation::default();
        self.evaluate_into(&mut evaluation).then_some(evaluation)
    }

    pub fn evaluate_into(&self, evaluation: &mut MotionEvaluation) -> bool {
        evaluation.clear();
        self.evaluate_tracks_into_with_local_scratch(evaluation);
        !evaluation.is_empty()
    }

    pub fn advance(&mut self, dt: f32) -> Option<MotionEvaluation> {
        let mut evaluation = MotionEvaluation::default();
        self.advance_into(dt, &mut evaluation).then_some(evaluation)
    }

    pub fn advance_into(&mut self, dt: f32, evaluation: &mut MotionEvaluation) -> bool {
        evaluation.clear();
        if self.state != MotionPlaybackState::Playing {
            if matches!(
                self.state,
                MotionPlaybackState::Stopped | MotionPlaybackState::Finished
            ) {
                self.start_next_motion();
            }
            if self.state != MotionPlaybackState::Playing {
                return false;
            }
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.advance_tracks(dt, evaluation);
        if self.active_finished() {
            if !self.start_next_motion() {
                self.state = MotionPlaybackState::Finished;
            }
        }
        self.evaluate_tracks_into(evaluation);
        evaluation.has_motion_values()
    }

    fn start_now(&mut self, request: QueuedMotion) {
        self.fading_out = self
            .active
            .take()
            .and_then(|mut active| active.into_fade_out());
        self.active = Some(MotionTrack {
            motion: request.motion,
            elapsed_seconds: 0.0,
            fade_elapsed_seconds: 0.0,
            options: request.options,
        });
        self.state = MotionPlaybackState::Playing;
    }

    fn enqueue(&mut self, request: QueuedMotion) {
        let priority = request.options.priority;
        let index = self
            .queued
            .iter()
            .position(|queued| queued.options.priority < priority)
            .unwrap_or(self.queued.len());
        self.queued.insert(index, request);
    }

    fn start_next_motion(&mut self) -> bool {
        if let Some(request) = self.queued.pop_front() {
            self.start_now(request);
            return true;
        }
        if let Some(idle) = self.idle.clone() {
            self.start_now(idle);
            return true;
        }
        false
    }

    fn can_start_priority(&self, priority: MotionPriority) -> bool {
        let Some(active) = &self.active else {
            return true;
        };
        matches!(
            self.state,
            MotionPlaybackState::Stopped | MotionPlaybackState::Finished
        ) || priority >= active.options.priority
    }

    fn advance_tracks(&mut self, dt: f32, evaluation: &mut MotionEvaluation) {
        if let Some(active) = &mut self.active {
            let previous = active.elapsed_seconds;
            active.advance(dt);
            collect_events_between(
                &active.motion,
                previous,
                active.elapsed_seconds,
                active.options.loop_playback,
                &mut evaluation.events,
            );
        }
        if let Some(fading_out) = &mut self.fading_out {
            let previous = fading_out.elapsed_seconds;
            fading_out.advance(dt);
            collect_events_between(
                &fading_out.motion,
                previous,
                fading_out.elapsed_seconds,
                fading_out.options.loop_playback,
                &mut evaluation.events,
            );
        }
        if self
            .fading_out
            .as_ref()
            .is_some_and(MotionTrack::is_fade_out_finished)
        {
            self.fading_out = None;
        }
    }

    fn active_finished(&self) -> bool {
        self.active.as_ref().is_some_and(MotionTrack::is_finished)
    }

    fn evaluate_tracks_into(&mut self, evaluation: &mut MotionEvaluation) {
        if let Some(fading_out) = &self.fading_out {
            fading_out.motion.sample_into(
                fading_out.elapsed_seconds,
                fading_out.options.loop_playback,
                &mut self.scratch,
            );
            evaluation.add_weighted(&self.scratch, fading_out.fade_out_weight());
        }
        if let Some(active) = &self.active {
            active.motion.sample_into(
                active.elapsed_seconds,
                active.options.loop_playback,
                &mut self.scratch,
            );
            evaluation.add_weighted(&self.scratch, active.fade_in_weight());
        }
    }

    fn evaluate_tracks_into_with_local_scratch(&self, evaluation: &mut MotionEvaluation) {
        let mut scratch = MotionEvaluation::default();
        if let Some(fading_out) = &self.fading_out {
            fading_out.motion.sample_into(
                fading_out.elapsed_seconds,
                fading_out.options.loop_playback,
                &mut scratch,
            );
            evaluation.add_weighted(&scratch, fading_out.fade_out_weight());
        }
        if let Some(active) = &self.active {
            active.motion.sample_into(
                active.elapsed_seconds,
                active.options.loop_playback,
                &mut scratch,
            );
            evaluation.add_weighted(&scratch, active.fade_in_weight());
        }
    }

    fn update_finished_state(&mut self) {
        let Some(active) = &self.active else {
            if self.fading_out.is_none() {
                self.state = MotionPlaybackState::Stopped;
            }
            return;
        };
        if active.is_finished() {
            self.state = MotionPlaybackState::Finished;
        }
    }
}

impl Default for MotionMixer {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionMixer {
    pub fn new() -> Self {
        Self {
            primary: MotionPlayer::new(),
            layers: Vec::new(),
        }
    }

    pub fn primary(&self) -> &MotionPlayer {
        &self.primary
    }

    pub fn primary_mut(&mut self) -> &mut MotionPlayer {
        &mut self.primary
    }

    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    pub fn layer_options(&self, id: impl AsRef<str>) -> Option<MotionLayerOptions> {
        self.layer(id.as_ref()).map(|layer| layer.options)
    }

    pub fn layer_player(&self, id: impl AsRef<str>) -> Option<&MotionPlayer> {
        self.layer(id.as_ref()).map(|layer| &layer.player)
    }

    pub fn set_layer(
        &mut self,
        id: impl Into<MotionLayerId>,
        motion: Live2DMotion,
        play_options: MotionPlayOptions,
        layer_options: MotionLayerOptions,
    ) {
        let id = id.into();
        let layer_options = layer_options.normalized();
        if let Some(layer) = self.layer_mut(id.as_ref()) {
            layer.options = layer_options;
            layer.player.play_with_options(motion, play_options);
            return;
        }
        let mut player = MotionPlayer::new();
        player.play_with_options(motion, play_options);
        self.layers.push(MotionLayer {
            id,
            player,
            options: layer_options,
        });
    }

    pub fn clear_layer(&mut self, id: impl AsRef<str>) -> bool {
        let Some(index) = self
            .layers
            .iter()
            .position(|layer| layer.id.as_ref() == id.as_ref())
        else {
            return false;
        };
        self.layers.remove(index);
        true
    }

    pub fn set_layer_weight(&mut self, id: impl AsRef<str>, weight: f32) -> bool {
        let Some(layer) = self.layer_mut(id.as_ref()) else {
            return false;
        };
        layer.options.weight = finite_non_negative(weight);
        true
    }

    pub fn evaluate_into(&self, evaluation: &mut MotionEvaluation) -> bool {
        evaluation.clear();
        self.primary.evaluate_into(evaluation);
        let mut scratch = MotionEvaluation::default();
        for layer in &self.layers {
            if layer.player.evaluate_into(&mut scratch) {
                evaluation.apply_layer(&scratch, layer.options);
            }
        }
        evaluation.has_motion_values()
    }

    pub fn advance_into(&mut self, dt: f32, evaluation: &mut MotionEvaluation) -> bool {
        evaluation.clear();
        self.primary.advance_into(dt, evaluation);
        let mut scratch = MotionEvaluation::default();
        for layer in &mut self.layers {
            layer.player.advance_into(dt, &mut scratch);
            evaluation.apply_layer(&scratch, layer.options);
        }
        evaluation.has_motion_values()
    }

    fn layer(&self, id: &str) -> Option<&MotionLayer> {
        self.layers.iter().find(|layer| layer.id.as_ref() == id)
    }

    fn layer_mut(&mut self, id: &str) -> Option<&mut MotionLayer> {
        self.layers.iter_mut().find(|layer| layer.id.as_ref() == id)
    }
}

impl MotionTrack {
    fn advance(&mut self, dt: f32) {
        self.elapsed_seconds = self.clamp_elapsed(self.elapsed_seconds + dt * self.options.speed);
        self.fade_elapsed_seconds += dt;
    }

    fn clamp_elapsed(&self, elapsed_seconds: f32) -> f32 {
        if self.motion.should_loop(self.options.loop_playback) || self.motion.duration <= 0.0 {
            elapsed_seconds
        } else {
            elapsed_seconds.min(self.motion.duration)
        }
    }

    fn fade_in_weight(&self) -> f32 {
        fade_weight(self.fade_elapsed_seconds, self.options.fade_in_seconds)
    }

    fn fade_out_weight(&self) -> f32 {
        1.0 - fade_weight(self.fade_elapsed_seconds, self.options.fade_out_seconds)
    }

    fn is_fade_out_finished(&self) -> bool {
        self.options.fade_out_seconds <= 0.0
            || self.fade_elapsed_seconds >= self.options.fade_out_seconds
    }

    fn is_finished(&self) -> bool {
        self.motion
            .is_finished(self.elapsed_seconds, self.options.loop_playback)
    }

    fn into_fade_out(&mut self) -> Option<Self> {
        let fade_out_seconds = self.options.fade_out_seconds;
        if fade_out_seconds <= 0.0 {
            return None;
        }
        self.fade_elapsed_seconds = 0.0;
        Some(self.clone())
    }
}

fn parse_curve(value: &Value) -> Result<MotionCurve, String> {
    let target = match value
        .get("Target")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "Model" => MotionTarget::Model,
        "Parameter" => MotionTarget::Parameter,
        "PartOpacity" => MotionTarget::PartOpacity,
        _ => return Err("unsupported_motion_curve_target".into()),
    };
    let id = value
        .get("Id")
        .and_then(Value::as_str)
        .ok_or_else(|| "motion_curve_id_missing".to_string())?
        .to_string();
    let values = value
        .get("Segments")
        .and_then(Value::as_array)
        .ok_or_else(|| "motion_curve_segments_missing".to_string())?
        .iter()
        .map(|value| {
            read_f32(Some(value)).ok_or_else(|| "invalid_motion_segment_value".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (initial, segments) = parse_segments(&values)?;
    Ok(MotionCurve {
        target,
        id,
        initial,
        segments,
    })
}

fn parse_events(root: &Value) -> Result<Vec<MotionEvent>, String> {
    let Some(events) = root.get("UserData").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = events
        .iter()
        .map(|event| {
            let time_seconds = read_f32(event.get("Time"))
                .ok_or_else(|| "motion_event_time_missing".to_string())?;
            let value = event
                .get("Value")
                .and_then(Value::as_str)
                .ok_or_else(|| "motion_event_value_missing".to_string())?
                .to_string();
            Ok::<MotionEvent, String>(MotionEvent {
                time_seconds: finite_non_negative(time_seconds),
                value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    parsed.sort_by(|left, right| left.time_seconds.total_cmp(&right.time_seconds));
    Ok(parsed)
}

fn parse_segments(values: &[f32]) -> Result<(MotionPoint, Vec<MotionSegment>), String> {
    if values.len() < 2 {
        return Err("motion_curve_initial_point_missing".into());
    }
    let initial = MotionPoint {
        time: values[0],
        value: values[1],
    };
    let mut previous = initial;
    let mut segments = Vec::new();
    let mut cursor = 2;
    while cursor < values.len() {
        let kind = match values[cursor] as i32 {
            0 => MotionSegmentKind::Linear,
            1 => MotionSegmentKind::Bezier,
            2 => MotionSegmentKind::Stepped,
            3 => MotionSegmentKind::InverseStepped,
            _ => return Err("unsupported_motion_segment_type".into()),
        };
        cursor += 1;
        let segment_points = match kind {
            MotionSegmentKind::Linear
            | MotionSegmentKind::Stepped
            | MotionSegmentKind::InverseStepped => {
                if cursor + 1 >= values.len() {
                    return Err("motion_segment_point_missing".into());
                }
                let end = MotionPoint {
                    time: values[cursor],
                    value: values[cursor + 1],
                };
                cursor += 2;
                vec![previous, end]
            }
            MotionSegmentKind::Bezier => {
                if cursor + 5 >= values.len() {
                    return Err("motion_bezier_points_missing".into());
                }
                let points = vec![
                    previous,
                    MotionPoint {
                        time: values[cursor],
                        value: values[cursor + 1],
                    },
                    MotionPoint {
                        time: values[cursor + 2],
                        value: values[cursor + 3],
                    },
                    MotionPoint {
                        time: values[cursor + 4],
                        value: values[cursor + 5],
                    },
                ];
                cursor += 6;
                points
            }
        };
        previous = *segment_points.last().expect("segment has an end point");
        segments.push(MotionSegment {
            kind,
            points: segment_points,
        });
    }
    Ok((initial, segments))
}

fn evaluate_curve(curve: &MotionCurve, time: f32) -> f32 {
    if time <= curve.initial.time {
        return curve.initial.value;
    }
    for segment in &curve.segments {
        let Some(end) = segment.points.last() else {
            continue;
        };
        if time <= end.time {
            return evaluate_segment(segment, time);
        }
    }
    curve
        .segments
        .last()
        .and_then(|segment| segment.points.last())
        .copied()
        .unwrap_or(curve.initial)
        .value
}

fn evaluate_segment(segment: &MotionSegment, time: f32) -> f32 {
    match segment.kind {
        MotionSegmentKind::Linear => {
            let start = segment.points[0];
            let end = segment.points[1];
            let t = normalized_time(start.time, end.time, time);
            lerp(start.value, end.value, t)
        }
        MotionSegmentKind::Bezier => evaluate_bezier(&segment.points, time),
        MotionSegmentKind::Stepped => segment.points[0].value,
        MotionSegmentKind::InverseStepped => segment.points[1].value,
    }
}

fn evaluate_bezier(points: &[MotionPoint], time: f32) -> f32 {
    if points.len() != 4 {
        return 0.0;
    }
    let start = points[0];
    let end = points[3];
    if (end.time - start.time).abs() <= f32::EPSILON {
        return end.value;
    }
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..24 {
        let t = (low + high) * 0.5;
        if cubic_bezier(start.time, points[1].time, points[2].time, end.time, t) < time {
            low = t;
        } else {
            high = t;
        }
    }
    let t = (low + high) * 0.5;
    cubic_bezier(start.value, points[1].value, points[2].value, end.value, t)
}

fn normalized_time(start: f32, end: f32, time: f32) -> f32 {
    if (end - start).abs() <= f32::EPSILON {
        return 1.0;
    }
    ((time - start) / (end - start)).clamp(0.0, 1.0)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn cubic_bezier(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let inv = 1.0 - t;
    inv * inv * inv * a + 3.0 * inv * inv * t * b + 3.0 * inv * t * t * c + t * t * t * d
}

fn read_f32(value: Option<&Value>) -> Option<f32> {
    value.and_then(Value::as_f64).map(|value| value as f32)
}

fn add_weighted_values(target: &mut Vec<(String, f32)>, source: &[(String, f32)], weight: f32) {
    for (id, value) in source {
        if let Some((_, current)) = target
            .iter_mut()
            .find(|(candidate_id, _)| candidate_id == id)
        {
            *current += value * weight;
        } else {
            target.push((id.clone(), value * weight));
        }
    }
}

fn add_override_values(target: &mut Vec<(String, f32)>, source: &[(String, f32)], weight: f32) {
    for (id, value) in source {
        if let Some((_, current)) = target
            .iter_mut()
            .find(|(candidate_id, _)| candidate_id == id)
        {
            *current = lerp(*current, *value, weight);
        } else {
            target.push((id.clone(), value * weight));
        }
    }
}

fn blend_optional_opacity(current: Option<f32>, next: f32, weight: f32) -> f32 {
    lerp(current.unwrap_or(1.0), next, weight)
}

fn collect_events_between(
    motion: &Live2DMotion,
    previous_seconds: f32,
    current_seconds: f32,
    loop_playback: bool,
    events: &mut Vec<MotionEvent>,
) {
    if motion.events.is_empty() || current_seconds <= previous_seconds {
        return;
    }
    if motion.should_loop(loop_playback) && motion.duration > 0.0 {
        collect_looped_events(motion, previous_seconds, current_seconds, events);
    } else {
        collect_event_window(&motion.events, previous_seconds, current_seconds, events);
    }
}

fn collect_looped_events(
    motion: &Live2DMotion,
    previous_seconds: f32,
    current_seconds: f32,
    events: &mut Vec<MotionEvent>,
) {
    let duration = motion.duration;
    let mut cycle_start = (previous_seconds / duration).floor() as i32;
    let cycle_end = (current_seconds / duration).floor() as i32;
    while cycle_start <= cycle_end {
        let base = cycle_start as f32 * duration;
        let window_start = (previous_seconds - base).clamp(0.0, duration);
        let window_end = (current_seconds - base).clamp(0.0, duration);
        if window_end > window_start {
            collect_event_window(&motion.events, window_start, window_end, events);
        }
        cycle_start += 1;
    }
}

fn collect_event_window(
    source: &[MotionEvent],
    previous_seconds: f32,
    current_seconds: f32,
    events: &mut Vec<MotionEvent>,
) {
    events.extend(
        source
            .iter()
            .filter(|event| {
                event.time_seconds > previous_seconds && event.time_seconds <= current_seconds
            })
            .cloned(),
    );
}

fn finite_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn fade_weight(elapsed_seconds: f32, duration_seconds: f32) -> f32 {
    if duration_seconds <= 0.0 {
        1.0
    } else {
        (elapsed_seconds / duration_seconds).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_supported_segment_types_and_looping() {
        let motion = Live2DMotion::from_json_str(
            r#"{
                "Meta": { "Duration": 4.0, "Loop": false },
                "Curves": [
                    { "Target": "Parameter", "Id": "Linear", "Segments": [0, 0, 0, 2, 10] },
                    { "Target": "Parameter", "Id": "Bezier", "Segments": [0, 0, 1, 0, 0, 2, 10, 2, 10] },
                    { "Target": "Parameter", "Id": "Stepped", "Segments": [0, 3, 2, 2, 9] },
                    { "Target": "Parameter", "Id": "Inverse", "Segments": [0, 3, 3, 2, 9] },
                    { "Target": "Model", "Id": "Opacity", "Segments": [0, 1, 0, 2, 0.5] },
                    { "Target": "PartOpacity", "Id": "PartArm", "Segments": [0, 0, 0, 2, 1] }
                ]
            }"#,
        )
        .unwrap();

        let sampled = motion.sample(1.0, false);

        assert_close(value_for(&sampled, "Linear"), 5.0);
        assert_close(value_for(&sampled, "Bezier"), 5.0);
        assert_close(value_for(&sampled, "Stepped"), 3.0);
        assert_close(value_for(&sampled, "Inverse"), 9.0);
        assert_close(sampled.model_opacity.unwrap(), 0.75);
        assert_close(part_opacity_for(&sampled, "PartArm"), 0.5);

        let looped = motion.sample(5.0, true);
        assert_close(value_for(&looped, "Linear"), 5.0);
        assert!(!motion.is_finished(5.0, true));
        assert!(motion.is_finished(5.0, false));
    }

    #[test]
    fn player_controls_playback_state_and_timing() {
        let motion = simple_motion(2.0, false);
        let mut player = MotionPlayer::new();

        player.play(motion.clone(), false);
        assert_eq!(player.state(), MotionPlaybackState::Playing);
        player.advance(0.5);
        assert_close(player.elapsed_seconds(), 0.5);

        player.pause();
        assert_eq!(player.state(), MotionPlaybackState::Paused);
        assert!(player.advance(1.0).is_none());
        assert_close(player.elapsed_seconds(), 0.5);

        player.resume();
        player.set_speed(2.0);
        player.advance(0.5);
        assert_close(player.elapsed_seconds(), 1.5);

        player.seek(3.0);
        assert_close(player.elapsed_seconds(), 2.0);
        assert_eq!(player.state(), MotionPlaybackState::Finished);
        player.advance(0.1);
        assert_eq!(player.state(), MotionPlaybackState::Finished);

        player.play(motion, true);
        player.advance(2.5);
        assert_eq!(player.state(), MotionPlaybackState::Playing);
        assert_close(value_for(&player.evaluate().unwrap(), "ParamAngleX"), 2.5);

        player.stop();
        assert_eq!(player.state(), MotionPlaybackState::Stopped);
        assert!(player.evaluate().is_none());
    }

    #[test]
    fn player_crossfades_motions_into_reused_output() {
        let mut player = MotionPlayer::new();
        player.play_with_options(
            named_motion("ParamAngleX", 0.0, 10.0),
            MotionPlayOptions {
                fade_in_seconds: 0.0,
                fade_out_seconds: 1.0,
                ..MotionPlayOptions::default()
            },
        );

        player.advance(0.5);
        player.play_with_options(
            named_motion("ParamAngleX", 20.0, 40.0),
            MotionPlayOptions {
                fade_in_seconds: 1.0,
                ..MotionPlayOptions::default()
            },
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(player.advance_into(0.5, &mut evaluation));

        assert_close(value_for(&evaluation, "ParamAngleX"), 20.0);
        assert!(player.advance_into(0.5, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), 40.0);
    }

    #[test]
    fn player_prioritizes_requests_and_drains_queue() {
        let mut player = MotionPlayer::new();
        player.play_with_options(
            named_motion("ParamAngleX", 0.0, 10.0),
            MotionPlayOptions {
                priority: MotionPriority::FORCE,
                ..MotionPlayOptions::default()
            },
        );

        let queued = player.request_motion(
            named_motion("ParamAngleX", 20.0, 30.0),
            MotionPlayOptions::default(),
        );

        assert_eq!(queued, MotionStartResult::Queued);
        assert_eq!(player.queued_motion_count(), 1);
        assert_close(value_for(&player.evaluate().unwrap(), "ParamAngleX"), 0.0);

        let started = player.request_motion(
            named_motion("ParamAngleX", 40.0, 50.0),
            MotionPlayOptions {
                priority: MotionPriority::FORCE,
                ..MotionPlayOptions::default()
            },
        );

        assert_eq!(started, MotionStartResult::Started);
        assert_eq!(player.queued_motion_count(), 1);
        assert_close(value_for(&player.evaluate().unwrap(), "ParamAngleX"), 40.0);

        let mut evaluation = MotionEvaluation::default();
        assert!(player.advance_into(1.0, &mut evaluation));
        assert_eq!(player.queued_motion_count(), 0);
        assert_close(value_for(&evaluation, "ParamAngleX"), 20.0);
    }

    #[test]
    fn player_falls_back_to_idle_without_waiting_for_an_extra_tick() {
        let mut player = MotionPlayer::new();
        player.set_idle_motion(
            named_motion("ParamAngleX", -10.0, -10.0),
            MotionPlayOptions::default(),
        );
        player.play_with_options(
            named_motion("ParamAngleX", 0.0, 10.0),
            MotionPlayOptions::default(),
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(player.advance_into(1.0, &mut evaluation));

        assert_eq!(player.state(), MotionPlaybackState::Playing);
        assert_close(value_for(&evaluation, "ParamAngleX"), -10.0);
    }

    #[test]
    fn player_collects_motion_events_without_reporting_snapshot_changes() {
        let mut player = MotionPlayer::new();
        player.play(event_motion(false), false);

        let mut evaluation = MotionEvaluation::default();
        assert!(!player.advance_into(0.6, &mut evaluation));
        assert_eq!(event_values(&evaluation), vec!["blink"]);
        assert!(!evaluation.has_motion_values());

        assert!(!player.advance_into(0.2, &mut evaluation));
        assert!(evaluation.events.is_empty());
    }

    #[test]
    fn looping_motion_events_fire_again_after_wrap() {
        let mut player = MotionPlayer::new();
        player.play(event_motion(true), false);

        let mut evaluation = MotionEvaluation::default();
        assert!(!player.advance_into(0.6, &mut evaluation));
        assert_eq!(event_values(&evaluation), vec!["blink"]);

        assert!(!player.advance_into(0.6, &mut evaluation));
        assert!(evaluation.events.is_empty());
        assert!(!player.advance_into(0.4, &mut evaluation));
        assert_eq!(event_values(&evaluation), vec!["blink"]);
    }

    #[test]
    fn mixer_combines_primary_idle_and_additive_breath_layer() {
        let mut mixer = MotionMixer::new();
        mixer.primary_mut().set_idle_motion(
            named_motion("ParamAngleX", 10.0, 10.0),
            MotionPlayOptions::default(),
        );
        mixer.set_layer(
            "breath",
            named_motion("ParamBreath", 1.0, 1.0),
            MotionPlayOptions::looped(true),
            MotionLayerOptions {
                blend: MotionBlendMode::Additive,
                ..MotionLayerOptions::default()
            },
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(mixer.advance_into(0.25, &mut evaluation));

        assert_close(value_for(&evaluation, "ParamAngleX"), 10.0);
        assert_close(value_for(&evaluation, "ParamBreath"), 1.0);
    }

    #[test]
    fn mixer_keeps_breath_layer_when_primary_action_returns_to_idle() {
        let mut mixer = MotionMixer::new();
        mixer.primary_mut().set_idle_motion(
            named_motion("ParamAngleX", -10.0, -10.0),
            MotionPlayOptions::default(),
        );
        mixer.set_layer(
            "breath",
            named_motion("ParamBreath", 2.0, 2.0),
            MotionPlayOptions::looped(true),
            MotionLayerOptions {
                blend: MotionBlendMode::Additive,
                ..MotionLayerOptions::default()
            },
        );
        mixer.primary_mut().play_with_options(
            named_motion("ParamAngleX", 0.0, 10.0),
            MotionPlayOptions::default(),
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(mixer.advance_into(0.5, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), 5.0);
        assert_close(value_for(&evaluation, "ParamBreath"), 2.0);

        assert!(mixer.advance_into(0.5, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), -10.0);
        assert_close(value_for(&evaluation, "ParamBreath"), 2.0);
    }

    #[test]
    fn mixer_additive_layer_uses_weight_for_same_parameter() {
        let mut mixer = MotionMixer::new();
        mixer.primary_mut().play_with_options(
            named_motion("ParamAngleX", 10.0, 10.0),
            MotionPlayOptions::looped(true),
        );
        mixer.set_layer(
            "breath",
            named_motion("ParamAngleX", 4.0, 4.0),
            MotionPlayOptions::looped(true),
            MotionLayerOptions {
                weight: 0.5,
                blend: MotionBlendMode::Additive,
                ..MotionLayerOptions::default()
            },
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(mixer.advance_into(0.25, &mut evaluation));

        assert_close(value_for(&evaluation, "ParamAngleX"), 12.0);
    }

    #[test]
    fn mixer_disabled_and_cleared_layers_do_not_affect_output() {
        let mut mixer = MotionMixer::new();
        mixer.primary_mut().play_with_options(
            named_motion("ParamAngleX", 10.0, 10.0),
            MotionPlayOptions::looped(true),
        );
        mixer.set_layer(
            "breath",
            named_motion("ParamAngleX", 5.0, 5.0),
            MotionPlayOptions::looped(true),
            MotionLayerOptions {
                enabled: false,
                blend: MotionBlendMode::Additive,
                ..MotionLayerOptions::default()
            },
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(mixer.advance_into(0.25, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), 10.0);

        mixer.set_layer(
            "breath",
            named_motion("ParamAngleX", 5.0, 5.0),
            MotionPlayOptions::looped(true),
            MotionLayerOptions {
                blend: MotionBlendMode::Additive,
                ..MotionLayerOptions::default()
            },
        );
        assert!(mixer.advance_into(0.25, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), 15.0);

        assert!(mixer.clear_layer("breath"));
        assert!(mixer.advance_into(0.25, &mut evaluation));
        assert_close(value_for(&evaluation, "ParamAngleX"), 10.0);
    }

    #[test]
    fn mixer_layer_events_fire_again_after_loop_wrap() {
        let mut mixer = MotionMixer::new();
        mixer.set_layer(
            "blink",
            event_motion(true),
            MotionPlayOptions::looped(true),
            MotionLayerOptions::default(),
        );

        let mut evaluation = MotionEvaluation::default();
        assert!(!mixer.advance_into(0.6, &mut evaluation));
        assert_eq!(event_values(&evaluation), vec!["blink"]);

        assert!(!mixer.advance_into(0.6, &mut evaluation));
        assert!(evaluation.events.is_empty());
        assert!(!mixer.advance_into(0.4, &mut evaluation));
        assert_eq!(event_values(&evaluation), vec!["blink"]);
    }

    fn value_for(evaluation: &MotionEvaluation, id: &str) -> f32 {
        evaluation
            .parameters
            .iter()
            .find_map(|(parameter_id, value)| (parameter_id == id).then_some(*value))
            .unwrap()
    }

    fn event_values(evaluation: &MotionEvaluation) -> Vec<&str> {
        evaluation
            .events
            .iter()
            .map(|event| event.value.as_str())
            .collect()
    }

    fn part_opacity_for(evaluation: &MotionEvaluation, id: &str) -> f32 {
        evaluation
            .part_opacities
            .iter()
            .find_map(|(part_id, value)| (part_id == id).then_some(*value))
            .unwrap()
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 0.001,
            "expected {expected}, got {actual}"
        );
    }

    fn simple_motion(duration: f32, looped: bool) -> Live2DMotion {
        Live2DMotion::from_json_str(&format!(
            r#"{{
                "Meta": {{ "Duration": {duration}, "Loop": {looped} }},
                "Curves": [
                    {{ "Target": "Parameter", "Id": "ParamAngleX", "Segments": [0, 0, 0, {duration}, 10] }}
                ]
            }}"#
        ))
        .unwrap()
    }

    fn named_motion(parameter_id: &str, start: f32, end: f32) -> Live2DMotion {
        Live2DMotion::from_json_str(&format!(
            r#"{{
                "Meta": {{ "Duration": 1.0, "Loop": false }},
                "Curves": [
                    {{ "Target": "Parameter", "Id": "{parameter_id}", "Segments": [0, {start}, 0, 1, {end}] }}
                ]
            }}"#
        ))
        .unwrap()
    }

    fn event_motion(looped: bool) -> Live2DMotion {
        Live2DMotion::from_json_str(&format!(
            r#"{{
                "Meta": {{ "Duration": 1.0, "Loop": {looped} }},
                "Curves": [],
                "UserData": [
                    {{ "Time": 0.5, "Value": "blink" }}
                ]
            }}"#
        ))
        .unwrap()
    }
}
