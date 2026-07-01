use serde_json::Value;
use std::{fs, path::Path};

#[derive(Debug, Clone, PartialEq)]
pub struct Live2DMotion {
    duration: f32,
    looped: bool,
    curves: Vec<MotionCurve>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MotionEvaluation {
    pub model_opacity: Option<f32>,
    pub parameters: Vec<(String, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionPlaybackState {
    Stopped,
    Playing,
    Paused,
    Finished,
}

#[derive(Debug, Clone)]
pub struct MotionPlayer {
    motion: Option<Live2DMotion>,
    elapsed_seconds: f32,
    loop_playback: bool,
    speed: f32,
    state: MotionPlaybackState,
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
        Ok(Self {
            duration,
            looped,
            curves,
        })
    }

    pub fn duration(&self) -> f32 {
        self.duration
    }

    pub fn looped(&self) -> bool {
        self.looped
    }

    pub fn is_finished(&self, elapsed_seconds: f32, loop_playback: bool) -> bool {
        !self.should_loop(loop_playback) && self.duration > 0.0 && elapsed_seconds >= self.duration
    }

    pub fn sample(&self, elapsed_seconds: f32, loop_playback: bool) -> MotionEvaluation {
        let time = self.sample_time(elapsed_seconds, loop_playback);
        let mut model_opacity = None;
        let mut parameters = Vec::new();
        for curve in &self.curves {
            let value = evaluate_curve(curve, time);
            match curve.target {
                MotionTarget::Model if curve.id == "Opacity" => model_opacity = Some(value),
                MotionTarget::Parameter => parameters.push((curve.id.clone(), value)),
                MotionTarget::Model | MotionTarget::PartOpacity => {}
            }
        }
        MotionEvaluation {
            model_opacity,
            parameters,
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

impl Default for MotionPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionPlayer {
    pub fn new() -> Self {
        Self {
            motion: None,
            elapsed_seconds: 0.0,
            loop_playback: false,
            speed: 1.0,
            state: MotionPlaybackState::Stopped,
        }
    }

    pub fn play(&mut self, motion: Live2DMotion, loop_playback: bool) {
        self.motion = Some(motion);
        self.elapsed_seconds = 0.0;
        self.loop_playback = loop_playback;
        self.state = MotionPlaybackState::Playing;
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
        self.motion = None;
        self.elapsed_seconds = 0.0;
        self.loop_playback = false;
        self.state = MotionPlaybackState::Stopped;
    }

    pub fn seek(&mut self, elapsed_seconds: f32) {
        let was_finished = self.state == MotionPlaybackState::Finished;
        self.elapsed_seconds = self.clamp_elapsed(elapsed_seconds.max(0.0));
        if self.motion.is_none() {
            return;
        }
        self.update_finished_state();
        if was_finished && self.state != MotionPlaybackState::Finished {
            self.state = MotionPlaybackState::Paused;
        }
    }

    pub fn set_loop(&mut self, loop_playback: bool) {
        self.loop_playback = loop_playback;
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = if speed.is_finite() {
            speed.max(0.0)
        } else {
            1.0
        };
    }

    pub fn state(&self) -> MotionPlaybackState {
        self.state
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed_seconds
    }

    pub fn evaluate(&self) -> Option<MotionEvaluation> {
        self.motion
            .as_ref()
            .map(|motion| motion.sample(self.elapsed_seconds, self.loop_playback))
    }

    pub fn advance(&mut self, dt: f32) -> Option<MotionEvaluation> {
        if self.state != MotionPlaybackState::Playing {
            return None;
        }
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.elapsed_seconds = self.clamp_elapsed(self.elapsed_seconds + dt * self.speed);
        self.update_finished_state();
        self.evaluate()
    }

    fn clamp_elapsed(&self, elapsed_seconds: f32) -> f32 {
        let Some(motion) = &self.motion else {
            return 0.0;
        };
        if motion.should_loop(self.loop_playback) || motion.duration <= 0.0 {
            elapsed_seconds
        } else {
            elapsed_seconds.min(motion.duration)
        }
    }

    fn update_finished_state(&mut self) {
        let Some(motion) = &self.motion else {
            return;
        };
        if motion.is_finished(self.elapsed_seconds, self.loop_playback) {
            self.state = MotionPlaybackState::Finished;
        }
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
                    { "Target": "Model", "Id": "Opacity", "Segments": [0, 1, 0, 2, 0.5] }
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
        assert_close(value_for(&player.evaluate().unwrap(), "ParamAngleX"), 5.0);

        player.stop();
        assert_eq!(player.state(), MotionPlaybackState::Stopped);
        assert!(player.evaluate().is_none());
    }

    fn value_for(evaluation: &MotionEvaluation, id: &str) -> f32 {
        evaluation
            .parameters
            .iter()
            .find_map(|(parameter_id, value)| (parameter_id == id).then_some(*value))
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
}
