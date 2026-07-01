use live2d_core::{ArtMeshInfo, ModelSnapshot};
#[cfg(feature = "live2d-cubism")]
use live2d_core::{
    BlendMode, CanvasInfo, ClippingInfo, Drawable, DrawableId, TextureAsset, Vertex,
};
#[cfg(all(feature = "probe", feature = "live2d-cubism"))]
use live2d_probe::ProbeAttr;
#[cfg(feature = "probe")]
use live2d_probe::{counter, measure, ProbeSink, Stage};
use serde_json::Value;
#[cfg(feature = "live2d-cubism")]
use std::collections::HashMap;
use std::{
    fs,
    path::{Path, PathBuf},
};

mod motion;
pub use motion::{
    Live2DMotion, MotionEvaluation, MotionEvent, MotionPlayOptions, MotionPlaybackState,
    MotionPlayer, MotionPriority, MotionStartResult,
};

#[cfg(not(feature = "live2d-cubism"))]
const RUNTIME_UNAVAILABLE: &str = "live2d_runtime_unavailable";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ParameterId(pub String);

impl From<String> for ParameterId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ParameterId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for ParameterId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParameterInfo {
    pub id: ParameterId,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PartId(pub String);

impl From<String> for PartId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for PartId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for PartId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartInfo {
    pub id: PartId,
    pub default_opacity: f32,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelFiles {
    pub model_json_path: PathBuf,
    pub model_root: PathBuf,
    pub moc_path: PathBuf,
    pub texture_paths: Vec<PathBuf>,
    pub motion_groups: Vec<ModelMotionGroup>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelMotionGroup {
    pub name: String,
    pub motions: Vec<ModelMotionFile>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelMotionFile {
    pub path: PathBuf,
    pub relative_path: String,
    pub fade_in_seconds: Option<f32>,
    pub fade_out_seconds: Option<f32>,
}

impl ModelMotionFile {
    pub fn load_motion(&self) -> Result<Live2DMotion, String> {
        Live2DMotion::load_file(&self.path)
    }

    pub fn play_options(&self, loop_playback: bool) -> MotionPlayOptions {
        MotionPlayOptions {
            loop_playback,
            fade_in_seconds: self.fade_in_seconds.unwrap_or(0.0),
            fade_out_seconds: self.fade_out_seconds.unwrap_or(0.0),
            ..MotionPlayOptions::default()
        }
    }
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

#[derive(Debug)]
pub struct Live2DInstance {
    snapshot: ModelSnapshot,
    elapsed_seconds: f32,
    motion_player: MotionPlayer,
    motion_evaluation: MotionEvaluation,
    #[cfg(feature = "live2d-cubism")]
    model: runtime::CubismLive2DModel,
}

impl Live2DInstance {
    pub fn load(resolver: &FsAssetResolver, model_json_path: &str) -> Result<Self, String> {
        Self::load_file(resolver.resolve(model_json_path))
    }

    pub fn load_file(model_json_path: impl AsRef<Path>) -> Result<Self, String> {
        runtime::load_instance(model_json_path.as_ref())
    }

    pub fn update(&mut self, dt: f32) -> Result<(), String> {
        let _ = self.update_motion(dt)?;
        Ok(())
    }

    pub fn update_motion(&mut self, dt: f32) -> Result<bool, String> {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        self.elapsed_seconds += dt;
        if self
            .motion_player
            .advance_into(dt, &mut self.motion_evaluation)
        {
            self.apply_buffered_motion_frame()?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed_seconds
    }

    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snapshot
    }

    pub fn play_motion(&mut self, motion: Live2DMotion, loop_playback: bool) -> Result<(), String> {
        self.motion_player.play(motion, loop_playback);
        if let Err(error) = self.apply_current_motion_frame() {
            self.motion_player.stop();
            return Err(error);
        }
        Ok(())
    }

    pub fn play_motion_with_options(
        &mut self,
        motion: Live2DMotion,
        options: MotionPlayOptions,
    ) -> Result<(), String> {
        self.motion_player.play_with_options(motion, options);
        if let Err(error) = self.apply_current_motion_frame() {
            self.motion_player.stop();
            return Err(error);
        }
        Ok(())
    }

    pub fn play_motion_file(
        &mut self,
        motion_file: &ModelMotionFile,
        loop_playback: bool,
    ) -> Result<(), String> {
        let motion = motion_file.load_motion()?;
        self.play_motion_with_options(motion, motion_file.play_options(loop_playback))
    }

    pub fn request_motion(
        &mut self,
        motion: Live2DMotion,
        options: MotionPlayOptions,
    ) -> Result<MotionStartResult, String> {
        let result = self.motion_player.request_motion(motion, options);
        if result == MotionStartResult::Started {
            if let Err(error) = self.apply_current_motion_frame() {
                self.motion_player.stop();
                return Err(error);
            }
        }
        Ok(result)
    }

    pub fn request_motion_file(
        &mut self,
        motion_file: &ModelMotionFile,
        loop_playback: bool,
    ) -> Result<MotionStartResult, String> {
        let motion = motion_file.load_motion()?;
        self.request_motion(motion, motion_file.play_options(loop_playback))
    }

    pub fn queue_motion(&mut self, motion: Live2DMotion, options: MotionPlayOptions) {
        self.motion_player.queue_motion(motion, options);
    }

    pub fn queue_motion_file(
        &mut self,
        motion_file: &ModelMotionFile,
        loop_playback: bool,
    ) -> Result<(), String> {
        let motion = motion_file.load_motion()?;
        self.queue_motion(motion, motion_file.play_options(loop_playback));
        Ok(())
    }

    pub fn set_idle_motion(&mut self, motion: Live2DMotion, options: MotionPlayOptions) {
        self.motion_player.set_idle_motion(motion, options);
    }

    pub fn set_idle_motion_file(&mut self, motion_file: &ModelMotionFile) -> Result<(), String> {
        let motion = motion_file.load_motion()?;
        self.set_idle_motion(motion, motion_file.play_options(true));
        Ok(())
    }

    pub fn clear_idle_motion(&mut self) {
        self.motion_player.clear_idle_motion();
    }

    pub fn queued_motion_count(&self) -> usize {
        self.motion_player.queued_motion_count()
    }

    pub fn pause_motion(&mut self) {
        self.motion_player.pause();
    }

    pub fn resume_motion(&mut self) {
        self.motion_player.resume();
    }

    pub fn stop_motion(&mut self) {
        self.motion_player.stop();
    }

    pub fn stop_motion_with_fade(&mut self, fade_out_seconds: f32) {
        self.motion_player.stop_with_fade(fade_out_seconds);
    }

    pub fn seek_motion(&mut self, elapsed_seconds: f32) -> Result<(), String> {
        self.motion_player.seek(elapsed_seconds);
        self.apply_current_motion_frame()
    }

    pub fn set_motion_loop(&mut self, loop_playback: bool) {
        self.motion_player.set_loop(loop_playback);
    }

    pub fn set_motion_speed(&mut self, speed: f32) {
        self.motion_player.set_speed(speed);
    }

    pub fn motion_state(&self) -> MotionPlaybackState {
        self.motion_player.state()
    }

    pub fn motion_elapsed_seconds(&self) -> f32 {
        self.motion_player.elapsed_seconds()
    }

    pub fn motion_player(&self) -> &MotionPlayer {
        &self.motion_player
    }

    pub fn motion_events(&self) -> &[MotionEvent] {
        &self.motion_evaluation.events
    }

    pub fn parameters(&self) -> Result<Vec<ParameterInfo>, String> {
        #[cfg(feature = "live2d-cubism")]
        {
            return self.model.parameters();
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    pub fn parameter(&self, id: impl AsRef<str>) -> Result<Option<ParameterInfo>, String> {
        let id = id.as_ref();
        Ok(self
            .parameters()?
            .into_iter()
            .find(|parameter| parameter.id.as_ref() == id))
    }

    pub fn parts(&self) -> Result<Vec<PartInfo>, String> {
        #[cfg(feature = "live2d-cubism")]
        {
            return self.model.parts();
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    pub fn part(&self, id: impl AsRef<str>) -> Result<Option<PartInfo>, String> {
        let id = id.as_ref();
        Ok(self
            .parts()?
            .into_iter()
            .find(|part| part.id.as_ref() == id))
    }

    pub fn set_parameter(&mut self, id: impl AsRef<str>, value: f32) -> Result<(), String> {
        self.set_parameters(std::iter::once((id, value)))
    }

    pub fn set_parameters<I, S>(&mut self, parameters: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (S, f32)>,
        S: AsRef<str>,
    {
        #[cfg(feature = "live2d-cubism")]
        {
            let parameters = parameters
                .into_iter()
                .map(|(id, value)| (id.as_ref().to_owned(), value))
                .collect::<Vec<_>>();
            self.model.set_parameters(&parameters)?;
            self.model.update_snapshot(&mut self.snapshot, 1.0)?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            let _ = parameters.into_iter();
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    pub fn set_part_opacity(&mut self, id: impl AsRef<str>, opacity: f32) -> Result<(), String> {
        self.set_part_opacities(std::iter::once((id, opacity)))
    }

    pub fn set_part_opacities<I, S>(&mut self, parts: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (S, f32)>,
        S: AsRef<str>,
    {
        #[cfg(feature = "live2d-cubism")]
        {
            let parts = parts
                .into_iter()
                .map(|(id, opacity)| (id.as_ref().to_owned(), opacity))
                .collect::<Vec<_>>();
            self.model.set_part_opacities(&parts)?;
            self.model.update_snapshot(&mut self.snapshot, 1.0)?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            let _ = parts.into_iter();
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    pub fn apply_motion(
        &mut self,
        motion: &Live2DMotion,
        elapsed_seconds: f32,
        loop_playback: bool,
    ) -> Result<(), String> {
        let evaluation = motion.sample(elapsed_seconds, loop_playback);
        self.apply_evaluation(&evaluation)
    }

    #[cfg(feature = "probe")]
    pub fn apply_motion_with_probe<P>(
        &mut self,
        motion: &Live2DMotion,
        elapsed_seconds: f32,
        loop_playback: bool,
        probe: &P,
    ) -> Result<(), String>
    where
        P: ProbeSink,
    {
        let evaluation = motion.sample(elapsed_seconds, loop_playback);
        self.apply_evaluation_with_probe(evaluation, probe)
    }

    pub fn reset_pose(&mut self) -> Result<(), String> {
        let evaluation = MotionEvaluation {
            model_opacity: Some(1.0),
            parameters: Vec::new(),
            part_opacities: Vec::new(),
            events: Vec::new(),
        };
        self.apply_evaluation(&evaluation)
    }

    pub fn reset_parameters(&mut self) -> Result<(), String> {
        #[cfg(feature = "live2d-cubism")]
        {
            self.model.reset_parameters()?;
            self.model.reset_parts()?;
            self.model.update_snapshot(&mut self.snapshot, 1.0)?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    fn apply_evaluation(&mut self, evaluation: &MotionEvaluation) -> Result<(), String> {
        #[cfg(feature = "live2d-cubism")]
        {
            self.model.reset_parameters()?;
            self.model.reset_parts()?;
            self.model.write_parameters(&evaluation.parameters)?;
            self.model
                .write_part_opacities(&evaluation.part_opacities)?;
            self.model
                .update_snapshot(&mut self.snapshot, evaluation.model_opacity.unwrap_or(1.0))?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            let _ = evaluation;
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    fn apply_current_motion_frame(&mut self) -> Result<(), String> {
        if self
            .motion_player
            .evaluate_into(&mut self.motion_evaluation)
        {
            self.apply_buffered_motion_frame()?;
        }
        Ok(())
    }

    fn apply_buffered_motion_frame(&mut self) -> Result<(), String> {
        #[cfg(feature = "live2d-cubism")]
        {
            self.model.reset_parameters()?;
            self.model.reset_parts()?;
            self.model
                .write_parameters(&self.motion_evaluation.parameters)?;
            self.model
                .write_part_opacities(&self.motion_evaluation.part_opacities)?;
            self.model.update_snapshot(
                &mut self.snapshot,
                self.motion_evaluation.model_opacity.unwrap_or(1.0),
            )?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }

    #[cfg(feature = "probe")]
    fn apply_evaluation_with_probe<P>(
        &mut self,
        evaluation: MotionEvaluation,
        probe: &P,
    ) -> Result<(), String>
    where
        P: ProbeSink,
    {
        #[cfg(feature = "live2d-cubism")]
        {
            self.model.reset_parameters()?;
            self.model.reset_parts()?;
            self.model.write_parameters(&evaluation.parameters)?;
            self.model
                .write_part_opacities(&evaluation.part_opacities)?;
            self.model.update_snapshot_with_probe(
                &mut self.snapshot,
                evaluation.model_opacity.unwrap_or(1.0),
                probe,
            )?;
            return Ok(());
        }
        #[cfg(not(feature = "live2d-cubism"))]
        {
            let _ = evaluation;
            let _ = probe;
            Err(RUNTIME_UNAVAILABLE.into())
        }
    }
}

pub fn update_instances<'a, I>(instances: I, dt: f32) -> Result<usize, String>
where
    I: IntoIterator<Item = &'a mut Live2DInstance>,
{
    let mut changed = 0;
    for instance in instances {
        if instance.update_motion(dt)? {
            changed += 1;
        }
    }
    Ok(changed)
}

pub fn update_instances_into<'a, I>(
    instances: I,
    dt: f32,
    changed_indices: &mut Vec<usize>,
) -> Result<usize, String>
where
    I: IntoIterator<Item = &'a mut Live2DInstance>,
{
    changed_indices.clear();
    for (index, instance) in instances.into_iter().enumerate() {
        if instance.update_motion(dt)? {
            changed_indices.push(index);
        }
    }
    Ok(changed_indices.len())
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
    parse_model_files(model_json_path, &json)
}

#[cfg(feature = "probe")]
pub fn resolve_model_files_with_probe<P>(
    model_json_path: impl AsRef<Path>,
    probe: &P,
) -> Result<ModelFiles, String>
where
    P: ProbeSink,
{
    measure(probe, Stage::RuntimeAssetResolve, Vec::new(), || {
        let model_json_path = model_json_path.as_ref();
        if model_json_path.as_os_str().is_empty() {
            return Err("invalid_live2d_model_path".into());
        }
        if !model_json_path.exists() {
            return Err("live2d_model_not_found".into());
        }

        let raw = measure(probe, Stage::RuntimeModel3Read, Vec::new(), || {
            fs::read_to_string(model_json_path).map_err(|_| "live2d_model_unreadable")
        })?;
        counter(
            probe,
            Stage::RuntimeModel3Read,
            "bytes",
            raw.len() as u64,
            Vec::new(),
        );
        let json: Value = measure(probe, Stage::RuntimeModel3Parse, Vec::new(), || {
            serde_json::from_str(&raw).map_err(|_| "invalid_model3_json")
        })?;
        let files = parse_model_files(model_json_path, &json)?;
        counter(
            probe,
            Stage::RuntimeAssetResolve,
            "texture_refs",
            files.texture_paths.len() as u64,
            Vec::new(),
        );
        counter(
            probe,
            Stage::RuntimeAssetResolve,
            "missing_files",
            files.missing_files.len() as u64,
            Vec::new(),
        );
        counter(
            probe,
            Stage::RuntimeAssetResolve,
            "motion_refs",
            files
                .motion_groups
                .iter()
                .map(|group| group.motions.len() as u64)
                .sum(),
            Vec::new(),
        );

        Ok(files)
    })
}

pub fn inspect_art_meshes(model_json_path: impl AsRef<Path>) -> Result<Vec<ArtMeshInfo>, String> {
    runtime::inspect_art_meshes(model_json_path.as_ref())
}

pub fn inspect_art_mesh_metadata(
    model_json_path: impl AsRef<Path>,
) -> Result<Vec<ArtMeshInfo>, String> {
    runtime::inspect_art_mesh_metadata(model_json_path.as_ref())
}

pub fn load_snapshot(model_json_path: impl AsRef<Path>) -> Result<ModelSnapshot, String> {
    runtime::load_snapshot(model_json_path.as_ref())
}

#[cfg(feature = "probe")]
pub fn inspect_art_meshes_with_probe<P>(
    model_json_path: impl AsRef<Path>,
    probe: &P,
) -> Result<Vec<ArtMeshInfo>, String>
where
    P: ProbeSink,
{
    runtime::inspect_art_meshes_with_probe(model_json_path.as_ref(), probe)
}

#[cfg(feature = "probe")]
pub fn load_snapshot_with_probe<P>(
    model_json_path: impl AsRef<Path>,
    probe: &P,
) -> Result<ModelSnapshot, String>
where
    P: ProbeSink,
{
    runtime::load_snapshot_with_probe(model_json_path.as_ref(), probe)
}

fn normalize_relative_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn parse_model_files(model_json_path: &Path, json: &Value) -> Result<ModelFiles, String> {
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

    let texture_paths = parse_texture_paths(file_refs, &model_root, &mut missing_files);
    let motion_groups = parse_motion_groups(file_refs, &model_root, &mut missing_files);
    missing_files.sort();
    missing_files.dedup();

    Ok(ModelFiles {
        model_json_path: model_json_path.to_path_buf(),
        model_root,
        moc_path,
        texture_paths,
        motion_groups,
        missing_files,
    })
}

fn parse_texture_paths(
    file_refs: &Value,
    model_root: &Path,
    missing_files: &mut Vec<String>,
) -> Vec<PathBuf> {
    file_refs
        .get("Textures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|texture| {
            let path = model_root.join(texture);
            if !path.exists() {
                missing_files.push(normalize_relative_path(texture));
            }
            path
        })
        .collect()
}

fn parse_motion_groups(
    file_refs: &Value,
    model_root: &Path,
    missing_files: &mut Vec<String>,
) -> Vec<ModelMotionGroup> {
    let Some(motions) = file_refs.get("Motions").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut groups = motions
        .iter()
        .filter_map(|(name, value)| {
            let motion_files = value
                .as_array()?
                .iter()
                .filter_map(|motion| parse_motion_file(motion, model_root, missing_files))
                .collect::<Vec<_>>();
            Some(ModelMotionGroup {
                name: name.clone(),
                motions: motion_files,
            })
        })
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| left.name.cmp(&right.name));
    groups
}

fn parse_motion_file(
    value: &Value,
    model_root: &Path,
    missing_files: &mut Vec<String>,
) -> Option<ModelMotionFile> {
    let file = value
        .get("File")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let path = model_root.join(file);
    let relative_path = normalize_relative_path(file);
    if !path.exists() {
        missing_files.push(relative_path.clone());
    }
    Some(ModelMotionFile {
        path,
        relative_path,
        fade_in_seconds: read_f32(value.get("FadeInTime")),
        fade_out_seconds: read_f32(value.get("FadeOutTime")),
    })
}

fn read_f32(value: Option<&Value>) -> Option<f32> {
    value.and_then(Value::as_f64).map(|value| value as f32)
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

    pub fn inspect_art_mesh_metadata(model_json_path: &Path) -> Result<Vec<ArtMeshInfo>, String> {
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

    pub fn load_instance(model_json_path: &Path) -> Result<Live2DInstance, String> {
        let snapshot = load_snapshot(model_json_path)?;
        Ok(Live2DInstance {
            snapshot,
            elapsed_seconds: 0.0,
            motion_player: MotionPlayer::new(),
            motion_evaluation: MotionEvaluation::default(),
        })
    }

    #[cfg(feature = "probe")]
    pub fn inspect_art_meshes_with_probe<P>(
        model_json_path: &Path,
        probe: &P,
    ) -> Result<Vec<ArtMeshInfo>, String>
    where
        P: ProbeSink,
    {
        Ok(load_snapshot_with_probe(model_json_path, probe)?.art_meshes)
    }

    #[cfg(feature = "probe")]
    pub fn load_snapshot_with_probe<P>(
        model_json_path: &Path,
        probe: &P,
    ) -> Result<ModelSnapshot, String>
    where
        P: ProbeSink,
    {
        measure(probe, Stage::RuntimeLoadSnapshot, Vec::new(), || {
            let files = resolve_model_files_with_probe(model_json_path, probe)?;
            if !files.missing_files.is_empty() {
                return Err("live2d_model_assets_missing".into());
            }
            Err(RUNTIME_UNAVAILABLE.into())
        })
    }
}

#[cfg(feature = "live2d-cubism")]
mod runtime {
    use super::*;
    use std::ffi::CStr;

    #[derive(Debug)]
    pub(crate) struct CubismLive2DModel {
        _moc_bytes: Vec<u8>,
        _model_bytes: Vec<u8>,
        model: *mut live2d_sys::CsmModel,
        parameters: Vec<ParameterInfo>,
        parameter_indices: HashMap<String, usize>,
        parts: Vec<PartInfo>,
        part_indices: HashMap<String, usize>,
        canvas: CanvasInfo,
        art_meshes: Vec<ArtMeshInfo>,
        drawable_metas: Vec<DrawableStaticMeta>,
    }

    #[derive(Debug, Clone)]
    struct DrawableStaticMeta {
        id: DrawableId,
        source_index: usize,
        texture_index: usize,
        blend_mode: BlendMode,
        clipping: Option<ClippingInfo>,
        mask_indices: Vec<usize>,
        uvs: Vec<[f32; 2]>,
        indices: Vec<u16>,
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct DrawableDynamicState {
        render_order: i32,
        visible: bool,
        opacity: f32,
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct SnapshotCopyStats {
        drawables: usize,
        vertices: usize,
        indices: usize,
        bytes: usize,
    }

    unsafe impl Send for CubismLive2DModel {}
    unsafe impl Sync for CubismLive2DModel {}

    impl CubismLive2DModel {
        fn load_metadata(files: &ModelFiles) -> Result<Self, String> {
            let (moc_bytes, model_bytes, model) = load_cubism_model(files)?;
            unsafe { live2d_sys::csmUpdateModel(model) };
            Ok(Self {
                _moc_bytes: moc_bytes,
                _model_bytes: model_bytes,
                model,
                parameters: Vec::new(),
                parameter_indices: HashMap::new(),
                parts: Vec::new(),
                part_indices: HashMap::new(),
                canvas: CanvasInfo::default(),
                art_meshes: Vec::new(),
                drawable_metas: Vec::new(),
            })
        }

        fn load(files: &ModelFiles) -> Result<Self, String> {
            let (moc_bytes, model_bytes, model) = load_cubism_model(files)?;
            unsafe { live2d_sys::csmUpdateModel(model) };
            let parameters = unsafe { read_parameters(model)? };
            let parameter_indices = build_parameter_indices(&parameters);
            let parts = unsafe { read_parts(model)? };
            let part_indices = build_part_indices(&parts);
            let (canvas, drawable_metas, art_meshes) = unsafe { read_static_model(model)? };
            Ok(Self {
                _moc_bytes: moc_bytes,
                _model_bytes: model_bytes,
                model,
                parameters,
                parameter_indices,
                parts,
                part_indices,
                canvas,
                art_meshes,
                drawable_metas,
            })
        }

        pub(crate) fn parameters(&self) -> Result<Vec<ParameterInfo>, String> {
            let values = unsafe { parameter_values(self.model, self.parameters.len())? };
            if values.len() != self.parameters.len() {
                return Err("live2d_parameter_table_changed".into());
            }
            Ok(self
                .parameters
                .iter()
                .zip(values.iter())
                .map(|(parameter, value)| {
                    let mut parameter = parameter.clone();
                    parameter.value = *value;
                    parameter
                })
                .collect())
        }

        pub(crate) fn parts(&self) -> Result<Vec<PartInfo>, String> {
            let values = unsafe { part_opacity_values(self.model, self.parts.len())? };
            if values.len() != self.parts.len() {
                return Err("live2d_part_table_changed".into());
            }
            Ok(self
                .parts
                .iter()
                .zip(values.iter())
                .map(|(part, opacity)| {
                    let mut part = part.clone();
                    part.opacity = *opacity;
                    part
                })
                .collect())
        }

        pub(crate) fn reset_parameters(&mut self) -> Result<(), String> {
            let values = unsafe { parameter_values_mut(self.model, self.parameters.len())? };
            if values.len() != self.parameters.len() {
                return Err("live2d_parameter_table_changed".into());
            }
            for (value, parameter) in values.iter_mut().zip(self.parameters.iter()) {
                *value = parameter.default;
            }
            Ok(())
        }

        pub(crate) fn reset_parts(&mut self) -> Result<(), String> {
            let values = unsafe { part_opacity_values_mut(self.model, self.parts.len())? };
            if values.len() != self.parts.len() {
                return Err("live2d_part_table_changed".into());
            }
            for (value, part) in values.iter_mut().zip(self.parts.iter()) {
                *value = part.default_opacity;
            }
            Ok(())
        }

        pub(crate) fn set_parameters(
            &mut self,
            parameters: &[(String, f32)],
        ) -> Result<(), String> {
            if parameters.is_empty() {
                return Ok(());
            }
            let writes = self.checked_parameter_writes(parameters)?;
            let values = unsafe { parameter_values_mut(self.model, self.parameters.len())? };
            if values.len() != self.parameters.len() {
                return Err("live2d_parameter_table_changed".into());
            }
            for (index, value) in writes {
                values[index] = value;
            }
            Ok(())
        }

        pub(crate) fn set_part_opacities(&mut self, parts: &[(String, f32)]) -> Result<(), String> {
            if parts.is_empty() {
                return Ok(());
            }
            let writes = self.checked_part_opacity_writes(parts)?;
            let values = unsafe { part_opacity_values_mut(self.model, self.parts.len())? };
            if values.len() != self.parts.len() {
                return Err("live2d_part_table_changed".into());
            }
            for (index, opacity) in writes {
                values[index] = opacity;
            }
            Ok(())
        }

        pub(crate) fn write_parameters(
            &mut self,
            parameters: &[(String, f32)],
        ) -> Result<(), String> {
            if parameters.is_empty() {
                return Ok(());
            }
            let writes = parameters
                .iter()
                .filter_map(|(id, value)| {
                    self.parameter_indices.get(id).map(|index| (*index, *value))
                })
                .collect::<Vec<_>>();
            let values = unsafe { parameter_values_mut(self.model, self.parameters.len())? };
            for (index, value) in writes {
                values[index] = value;
            }
            Ok(())
        }

        pub(crate) fn write_part_opacities(
            &mut self,
            parts: &[(String, f32)],
        ) -> Result<(), String> {
            if parts.is_empty() {
                return Ok(());
            }
            let writes = parts
                .iter()
                .filter_map(|(id, opacity)| {
                    self.part_indices
                        .get(id)
                        .map(|index| (*index, opacity.clamp(0.0, 1.0)))
                })
                .collect::<Vec<_>>();
            let values = unsafe { part_opacity_values_mut(self.model, self.parts.len())? };
            for (index, opacity) in writes {
                values[index] = opacity;
            }
            Ok(())
        }

        fn checked_parameter_writes(
            &self,
            parameters: &[(String, f32)],
        ) -> Result<Vec<(usize, f32)>, String> {
            let mut writes = Vec::with_capacity(parameters.len());
            for (id, value) in parameters {
                let Some(index) = self.parameter_indices.get(id).copied() else {
                    return Err("live2d_parameter_not_found".into());
                };
                let parameter = &self.parameters[index];
                writes.push((index, value.max(parameter.minimum).min(parameter.maximum)));
            }
            Ok(writes)
        }

        fn checked_part_opacity_writes(
            &self,
            parts: &[(String, f32)],
        ) -> Result<Vec<(usize, f32)>, String> {
            let mut writes = Vec::with_capacity(parts.len());
            for (id, opacity) in parts {
                let Some(index) = self.part_indices.get(id).copied() else {
                    return Err("live2d_part_not_found".into());
                };
                writes.push((index, opacity.clamp(0.0, 1.0)));
            }
            Ok(writes)
        }

        pub(crate) fn snapshot(
            &mut self,
            model_key: String,
            textures: Vec<TextureAsset>,
            model_opacity: f32,
        ) -> Result<ModelSnapshot, String> {
            unsafe { live2d_sys::csmUpdateModel(self.model) };
            let mut snapshot = ModelSnapshot {
                model_key,
                canvas: self.canvas.clone(),
                art_meshes: self.art_meshes.clone(),
                drawables: self.empty_drawables(),
                textures,
            };
            self.refresh_snapshot_drawables(&mut snapshot, model_opacity)?;
            Ok(snapshot)
        }

        pub(crate) fn update_snapshot(
            &mut self,
            snapshot: &mut ModelSnapshot,
            model_opacity: f32,
        ) -> Result<(), String> {
            unsafe { live2d_sys::csmUpdateModel(self.model) };
            self.refresh_snapshot_drawables(snapshot, model_opacity)?;
            Ok(())
        }

        #[cfg(feature = "probe")]
        pub(crate) fn update_snapshot_with_probe<P>(
            &mut self,
            snapshot: &mut ModelSnapshot,
            model_opacity: f32,
            probe: &P,
        ) -> Result<(), String>
        where
            P: ProbeSink,
        {
            measure(probe, Stage::RuntimeModelUpdate, Vec::new(), || unsafe {
                live2d_sys::csmUpdateModel(self.model)
            });
            let stats = measure(probe, Stage::RuntimeSnapshotExtract, Vec::new(), || {
                self.refresh_snapshot_drawables(snapshot, model_opacity)
            })?;
            counter(
                probe,
                Stage::RuntimeSnapshotExtract,
                "draw_calls",
                stats.drawables as u64,
                Vec::new(),
            );
            counter(
                probe,
                Stage::RuntimeSnapshotExtract,
                "vertices",
                stats.vertices as u64,
                Vec::new(),
            );
            counter(
                probe,
                Stage::RuntimeSnapshotExtract,
                "indices",
                stats.indices as u64,
                Vec::new(),
            );
            counter(
                probe,
                Stage::RuntimeSnapshotExtract,
                "bytes",
                stats.bytes as u64,
                Vec::new(),
            );
            Ok(())
        }

        fn empty_drawables(&self) -> Vec<Drawable> {
            self.drawable_metas
                .iter()
                .map(|meta| Drawable {
                    id: meta.id.clone(),
                    render_order: 0,
                    texture_index: meta.texture_index,
                    vertices: Vec::with_capacity(meta.uvs.len()),
                    indices: Vec::with_capacity(meta.indices.len()),
                    visible: false,
                    opacity: 0.0,
                    blend_mode: meta.blend_mode,
                    clipping: meta.clipping.clone(),
                })
                .collect()
        }

        fn refresh_snapshot_drawables(
            &self,
            snapshot: &mut ModelSnapshot,
            model_opacity: f32,
        ) -> Result<SnapshotCopyStats, String> {
            let (states, retained_indices) = unsafe { self.dynamic_snapshot_state(model_opacity)? };
            if snapshot.drawables.len() != self.drawable_metas.len() {
                snapshot.drawables = self.empty_drawables();
            }

            let vertex_positions = unsafe { live2d_sys::csmGetDrawableVertexPositions(self.model) };
            if vertex_positions.is_null() && !self.drawable_metas.is_empty() {
                return Err("live2d_model_snapshot_failed".into());
            }

            let mut stats = SnapshotCopyStats::default();
            for index in 0..self.drawable_metas.len() {
                let meta = &self.drawable_metas[index];
                let state = states[index];
                let retained = retained_indices
                    .get(meta.source_index)
                    .copied()
                    .unwrap_or(false);
                let Some(drawable) = snapshot
                    .drawables
                    .iter_mut()
                    .find(|drawable| drawable.id == meta.id)
                else {
                    return Err("live2d_model_snapshot_failed".into());
                };

                drawable.render_order = state.render_order;
                drawable.texture_index = meta.texture_index;
                drawable.visible = state.visible;
                drawable.opacity = state.opacity;
                drawable.blend_mode = meta.blend_mode;
                if drawable.clipping != meta.clipping {
                    drawable.clipping = meta.clipping.clone();
                }

                if !retained {
                    drawable.vertices.clear();
                    drawable.indices.clear();
                    continue;
                }

                let vertex_count = meta.uvs.len();
                let pos_ptr = unsafe { *vertex_positions.add(meta.source_index) };
                if pos_ptr.is_null() && vertex_count > 0 {
                    return Err("live2d_model_snapshot_failed".into());
                }
                let positions = if vertex_count == 0 {
                    &[]
                } else {
                    unsafe { std::slice::from_raw_parts(pos_ptr, vertex_count) }
                };
                if drawable.vertices.len() != vertex_count {
                    drawable.vertices.clear();
                    drawable
                        .vertices
                        .extend(positions.iter().zip(meta.uvs.iter()).map(|(position, uv)| {
                            Vertex {
                                position: [position.x, position.y],
                                uv: *uv,
                            }
                        }));
                    stats.bytes += vertex_count * std::mem::size_of::<Vertex>();
                } else {
                    for (vertex, position) in drawable.vertices.iter_mut().zip(positions.iter()) {
                        vertex.position = [position.x, position.y];
                    }
                    stats.bytes += vertex_count * std::mem::size_of::<[f32; 2]>();
                }
                stats.vertices += vertex_count;

                if drawable.indices.len() != meta.indices.len() {
                    drawable.indices.clear();
                    drawable.indices.extend_from_slice(&meta.indices);
                    stats.indices += meta.indices.len();
                    stats.bytes += meta.indices.len() * std::mem::size_of::<u16>();
                }
            }

            snapshot
                .drawables
                .sort_by_key(|drawable| drawable.render_order);
            stats.drawables = snapshot
                .drawables
                .iter()
                .filter(|drawable| !drawable.vertices.is_empty() && !drawable.indices.is_empty())
                .count();
            Ok(stats)
        }

        unsafe fn dynamic_snapshot_state(
            &self,
            model_opacity: f32,
        ) -> Result<(Vec<DrawableDynamicState>, Vec<bool>), String> {
            let count = live2d_sys::csmGetDrawableCount(self.model).max(0) as usize;
            let dynamic_flags = live2d_sys::csmGetDrawableDynamicFlags(self.model);
            let render_orders = live2d_sys::csmGetRenderOrders(self.model);
            let opacities = live2d_sys::csmGetDrawableOpacities(self.model);
            if (dynamic_flags.is_null() || render_orders.is_null() || opacities.is_null())
                && count > 0
            {
                return Err("live2d_model_snapshot_failed".into());
            }

            let mut states = Vec::with_capacity(self.drawable_metas.len());
            let mut retained_indices = vec![false; count];
            let opacity_scale = model_opacity.clamp(0.0, 1.0);

            for meta in &self.drawable_metas {
                if meta.source_index >= count {
                    return Err("live2d_model_topology_changed".into());
                }
                let dynamic = *dynamic_flags.add(meta.source_index);
                let state = DrawableDynamicState {
                    render_order: *render_orders.add(meta.source_index),
                    visible: dynamic & live2d_sys::csmIsVisible != 0,
                    opacity: (*opacities.add(meta.source_index) * opacity_scale).clamp(0.0, 1.0),
                };
                states.push(state);
            }

            for (meta, state) in self.drawable_metas.iter().zip(states.iter()) {
                if state.visible && state.opacity > 1e-6 {
                    retained_indices[meta.source_index] = true;
                    for mask_index in &meta.mask_indices {
                        if let Some(retained) = retained_indices.get_mut(*mask_index) {
                            *retained = true;
                        }
                    }
                }
            }
            Ok((states, retained_indices))
        }
    }

    fn load_cubism_model(
        files: &ModelFiles,
    ) -> Result<(Vec<u8>, Vec<u8>, *mut live2d_sys::CsmModel), String> {
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
        Ok((moc_bytes, model_bytes, model))
    }

    pub fn inspect_art_meshes(model_json_path: &Path) -> Result<Vec<ArtMeshInfo>, String> {
        Ok(load_snapshot(model_json_path)?.art_meshes)
    }

    pub fn inspect_art_mesh_metadata(model_json_path: &Path) -> Result<Vec<ArtMeshInfo>, String> {
        let files = resolve_model_files(model_json_path)?;
        if !files.missing_files.is_empty() {
            return Err("live2d_model_assets_missing".into());
        }
        let model = CubismLive2DModel::load_metadata(&files)?;
        unsafe { read_art_mesh_metadata(model.model) }
    }

    pub fn load_instance(model_json_path: &Path) -> Result<Live2DInstance, String> {
        let files = resolve_model_files(model_json_path)?;
        if !files.missing_files.is_empty() {
            return Err("live2d_model_assets_missing".into());
        }
        let textures = load_textures(&files.texture_paths)?;
        let mut model = CubismLive2DModel::load(&files)?;
        let snapshot = model.snapshot(
            files.model_json_path.to_string_lossy().into_owned(),
            textures,
            1.0,
        )?;
        Ok(Live2DInstance {
            snapshot,
            elapsed_seconds: 0.0,
            motion_player: MotionPlayer::new(),
            motion_evaluation: MotionEvaluation::default(),
            model,
        })
    }

    pub fn load_snapshot(model_json_path: &Path) -> Result<ModelSnapshot, String> {
        Ok(load_instance(model_json_path)?.snapshot)
    }

    #[cfg(feature = "probe")]
    pub fn inspect_art_meshes_with_probe<P>(
        model_json_path: &Path,
        probe: &P,
    ) -> Result<Vec<ArtMeshInfo>, String>
    where
        P: ProbeSink,
    {
        Ok(load_snapshot_with_probe(model_json_path, probe)?.art_meshes)
    }

    #[cfg(feature = "probe")]
    pub fn load_snapshot_with_probe<P>(
        model_json_path: &Path,
        probe: &P,
    ) -> Result<ModelSnapshot, String>
    where
        P: ProbeSink,
    {
        measure(probe, Stage::RuntimeLoadSnapshot, Vec::new(), || {
            let files = resolve_model_files_with_probe(model_json_path, probe)?;
            if !files.missing_files.is_empty() {
                return Err("live2d_model_assets_missing".into());
            }
            let textures = load_textures_with_probe(&files.texture_paths, probe)?;
            let mut moc_bytes = measure(probe, Stage::RuntimeMocRead, Vec::new(), || {
                fs::read(&files.moc_path).map_err(|_| "live2d_moc_unreadable")
            })?;
            counter(
                probe,
                Stage::RuntimeMocRead,
                "bytes",
                moc_bytes.len() as u64,
                Vec::new(),
            );
            let moc = measure(probe, Stage::RuntimeMocRevive, Vec::new(), || unsafe {
                live2d_sys::csmReviveMocInPlace(moc_bytes.as_mut_ptr().cast(), moc_bytes.len() as _)
            });
            if moc.is_null() {
                return Err("live2d_moc_invalid".into());
            }
            let model_size = measure(
                probe,
                Stage::RuntimeModelAllocation,
                Vec::new(),
                || unsafe { live2d_sys::csmGetSizeofModel(moc) as usize },
            );
            if model_size == 0 {
                return Err("live2d_model_allocation_failed".into());
            }
            counter(
                probe,
                Stage::RuntimeModelAllocation,
                "bytes",
                model_size as u64,
                Vec::new(),
            );
            let mut model_bytes = vec![0_u8; model_size];
            let model = measure(probe, Stage::RuntimeModelInit, Vec::new(), || unsafe {
                live2d_sys::csmInitializeModelInPlace(
                    moc,
                    model_bytes.as_mut_ptr().cast(),
                    model_bytes.len() as _,
                )
            });
            if model.is_null() {
                return Err("live2d_model_initialization_failed".into());
            }
            measure(probe, Stage::RuntimeModelUpdate, Vec::new(), || unsafe {
                live2d_sys::csmUpdateModel(model)
            });
            let (canvas, drawables, art_meshes) = measure(
                probe,
                Stage::RuntimeSnapshotExtract,
                Vec::new(),
                || unsafe { snapshot_model(model, 1.0) },
            )?;
            counter(
                probe,
                Stage::RuntimeSnapshotExtract,
                "draw_calls",
                drawables.len() as u64,
                Vec::new(),
            );

            Ok(ModelSnapshot {
                model_key: files.model_json_path.to_string_lossy().into_owned(),
                canvas,
                art_meshes,
                drawables,
                textures,
            })
        })
    }

    unsafe fn read_parameters(
        model: *mut live2d_sys::CsmModel,
    ) -> Result<Vec<ParameterInfo>, String> {
        let count = live2d_sys::csmGetParameterCount(model).max(0) as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let ids = live2d_sys::csmGetParameterIds(model);
        let minimums = live2d_sys::csmGetParameterMinimumValues(model);
        let maximums = live2d_sys::csmGetParameterMaximumValues(model);
        let defaults = live2d_sys::csmGetParameterDefaultValues(model);
        let values = live2d_sys::csmGetParameterValues(model);
        if ids.is_null()
            || minimums.is_null()
            || maximums.is_null()
            || defaults.is_null()
            || values.is_null()
        {
            return Err("live2d_parameter_table_unavailable".into());
        }
        let id_slice = std::slice::from_raw_parts(ids, count);
        let minimum_slice = std::slice::from_raw_parts(minimums, count);
        let maximum_slice = std::slice::from_raw_parts(maximums, count);
        let default_slice = std::slice::from_raw_parts(defaults, count);
        let value_slice = std::slice::from_raw_parts(values, count);
        let mut parameters = Vec::with_capacity(count);
        for index in 0..count {
            let id = if id_slice[index].is_null() {
                String::new()
            } else {
                CStr::from_ptr(id_slice[index])
                    .to_string_lossy()
                    .into_owned()
            };
            parameters.push(ParameterInfo {
                id: ParameterId(id),
                minimum: minimum_slice[index],
                maximum: maximum_slice[index],
                default: default_slice[index],
                value: value_slice[index],
            });
        }
        Ok(parameters)
    }

    unsafe fn read_parts(model: *mut live2d_sys::CsmModel) -> Result<Vec<PartInfo>, String> {
        let count = live2d_sys::csmGetPartCount(model).max(0) as usize;
        if count == 0 {
            return Ok(Vec::new());
        }
        let ids = live2d_sys::csmGetPartIds(model);
        let opacities = live2d_sys::csmGetPartOpacities(model);
        if ids.is_null() || opacities.is_null() {
            return Err("live2d_part_table_unavailable".into());
        }
        let id_slice = std::slice::from_raw_parts(ids, count);
        let opacity_slice = std::slice::from_raw_parts(opacities, count);
        let mut parts = Vec::with_capacity(count);
        for index in 0..count {
            let id = if id_slice[index].is_null() {
                String::new()
            } else {
                CStr::from_ptr(id_slice[index])
                    .to_string_lossy()
                    .into_owned()
            };
            parts.push(PartInfo {
                id: PartId(id),
                default_opacity: opacity_slice[index],
                opacity: opacity_slice[index],
            });
        }
        Ok(parts)
    }

    fn build_parameter_indices(parameters: &[ParameterInfo]) -> HashMap<String, usize> {
        parameters
            .iter()
            .enumerate()
            .map(|(index, parameter)| (parameter.id.0.clone(), index))
            .collect()
    }

    fn build_part_indices(parts: &[PartInfo]) -> HashMap<String, usize> {
        parts
            .iter()
            .enumerate()
            .map(|(index, part)| (part.id.0.clone(), index))
            .collect()
    }

    unsafe fn read_art_mesh_metadata(
        model: *mut live2d_sys::CsmModel,
    ) -> Result<Vec<ArtMeshInfo>, String> {
        let count = live2d_sys::csmGetDrawableCount(model).max(0) as usize;
        let ids = live2d_sys::csmGetDrawableIds(model);
        let mask_counts = live2d_sys::csmGetDrawableMaskCounts(model);
        if (ids.is_null() || mask_counts.is_null()) && count > 0 {
            return Err("live2d_model_metadata_failed".into());
        }

        let mut art_meshes = Vec::with_capacity(count);
        for index in 0..count {
            let id_ptr = *ids.add(index);
            if id_ptr.is_null() {
                continue;
            }
            let id = CStr::from_ptr(id_ptr).to_string_lossy().into_owned();
            let drawable_id = DrawableId(id);
            let mask_count = (*mask_counts.add(index)).max(0);
            art_meshes.push(ArtMeshInfo {
                id: drawable_id.clone(),
                label: drawable_id.0.clone(),
                original_name: drawable_id.0.clone(),
                index,
                mask_type: if mask_count > 0 { "masked" } else { "plain" }.into(),
            });
        }
        Ok(art_meshes)
    }

    unsafe fn parameter_values(
        model: *mut live2d_sys::CsmModel,
        count: usize,
    ) -> Result<&'static [f32], String> {
        if count == 0 {
            return Ok(&[]);
        }
        let values = live2d_sys::csmGetParameterValues(model);
        if values.is_null() {
            return Err("live2d_parameter_values_unavailable".into());
        }
        Ok(std::slice::from_raw_parts(values, count))
    }

    unsafe fn parameter_values_mut(
        model: *mut live2d_sys::CsmModel,
        count: usize,
    ) -> Result<&'static mut [f32], String> {
        if count == 0 {
            return Ok(&mut []);
        }
        let values = live2d_sys::csmGetParameterValues(model);
        if values.is_null() {
            return Err("live2d_parameter_values_unavailable".into());
        }
        Ok(std::slice::from_raw_parts_mut(values, count))
    }

    unsafe fn part_opacity_values(
        model: *mut live2d_sys::CsmModel,
        count: usize,
    ) -> Result<&'static [f32], String> {
        if count == 0 {
            return Ok(&[]);
        }
        let values = live2d_sys::csmGetPartOpacities(model);
        if values.is_null() {
            return Err("live2d_part_opacities_unavailable".into());
        }
        Ok(std::slice::from_raw_parts(values, count))
    }

    unsafe fn part_opacity_values_mut(
        model: *mut live2d_sys::CsmModel,
        count: usize,
    ) -> Result<&'static mut [f32], String> {
        if count == 0 {
            return Ok(&mut []);
        }
        let values = live2d_sys::csmGetPartOpacities(model);
        if values.is_null() {
            return Err("live2d_part_opacities_unavailable".into());
        }
        Ok(std::slice::from_raw_parts_mut(values, count))
    }

    unsafe fn read_static_model(
        model: *mut live2d_sys::CsmModel,
    ) -> Result<(CanvasInfo, Vec<DrawableStaticMeta>, Vec<ArtMeshInfo>), String> {
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
        let constant_flags = live2d_sys::csmGetDrawableConstantFlags(model);
        let blend_modes = live2d_sys::csmGetDrawableBlendModes(model);
        let texture_indices = live2d_sys::csmGetDrawableTextureIndices(model);
        let mask_counts = live2d_sys::csmGetDrawableMaskCounts(model);
        let masks = live2d_sys::csmGetDrawableMasks(model);
        let vertex_counts = live2d_sys::csmGetDrawableVertexCounts(model);
        let vertex_uvs = live2d_sys::csmGetDrawableVertexUvs(model);
        let index_counts = live2d_sys::csmGetDrawableIndexCounts(model);
        let indices = live2d_sys::csmGetDrawableIndices(model);
        if (ids.is_null()
            || constant_flags.is_null()
            || texture_indices.is_null()
            || mask_counts.is_null()
            || masks.is_null()
            || vertex_counts.is_null()
            || vertex_uvs.is_null()
            || index_counts.is_null()
            || indices.is_null())
            && count > 0
        {
            return Err("live2d_model_snapshot_failed".into());
        }

        let mut id_by_index = Vec::with_capacity(count);
        let mut art_meshes = Vec::new();
        for index in 0..count {
            let id_ptr = *ids.add(index);
            if id_ptr.is_null() {
                id_by_index.push(None);
                continue;
            }
            let id = CStr::from_ptr(id_ptr).to_string_lossy().into_owned();
            let drawable_id = DrawableId(id);
            art_meshes.push(ArtMeshInfo {
                id: drawable_id.clone(),
                label: drawable_id.0.clone(),
                original_name: drawable_id.0.clone(),
                index,
                mask_type: "unknown".into(),
            });
            id_by_index.push(Some(drawable_id));
        }

        let mut drawable_metas = Vec::new();
        for index in 0..count {
            let Some(drawable_id) = id_by_index[index].clone() else {
                continue;
            };
            let constant = *constant_flags.add(index);
            let mask_count = (*mask_counts.add(index)).max(0) as usize;
            let mut mask_indices = Vec::new();
            let mut mask_ids = Vec::new();
            if mask_count > 0 {
                let mask_ptr = *masks.add(index);
                if mask_ptr.is_null() {
                    return Err("live2d_model_snapshot_failed".into());
                }
                for mask_index in std::slice::from_raw_parts(mask_ptr, mask_count) {
                    let mask_index = *mask_index;
                    if mask_index < 0 {
                        continue;
                    }
                    let mask_index = mask_index as usize;
                    if let Some(Some(mask_id)) = id_by_index.get(mask_index) {
                        mask_indices.push(mask_index);
                        mask_ids.push(mask_id.clone());
                    }
                }
            }
            let clipping = if mask_ids.is_empty() {
                None
            } else {
                Some(ClippingInfo {
                    drawable_ids: mask_ids,
                    inverted: false,
                })
            };
            let blend_mode = if blend_modes.is_null() {
                blend_mode_from_constant_flags(constant)
            } else {
                BlendMode::from_cubism_blend_mode(*blend_modes.add(index))
            };

            let vertex_count = *vertex_counts.add(index) as usize;
            let index_count = *index_counts.add(index) as usize;
            let uv_ptr = *vertex_uvs.add(index);
            let index_ptr = *indices.add(index);
            if (uv_ptr.is_null() && vertex_count > 0) || (index_ptr.is_null() && index_count > 0) {
                return Err("live2d_model_snapshot_failed".into());
            }
            let uvs = if vertex_count == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(uv_ptr, vertex_count)
                    .iter()
                    .map(|uv| [uv.x, uv.y])
                    .collect::<Vec<_>>()
            };
            let drawable_indices = if index_count == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(index_ptr, index_count).to_vec()
            };

            drawable_metas.push(DrawableStaticMeta {
                id: drawable_id,
                source_index: index,
                texture_index: (*texture_indices.add(index)).max(0) as usize,
                blend_mode,
                clipping,
                mask_indices,
                uvs,
                indices: drawable_indices,
            });
        }

        Ok((
            CanvasInfo {
                size: [canvas_size.x, canvas_size.y],
                origin: [canvas_origin.x, canvas_origin.y],
                pixels_per_unit,
            },
            drawable_metas,
            art_meshes,
        ))
    }

    fn blend_mode_from_constant_flags(flags: live2d_sys::csmFlags) -> BlendMode {
        if flags & live2d_sys::csmBlendAdditive != 0 {
            BlendMode::Additive
        } else if flags & live2d_sys::csmBlendMultiplicative != 0 {
            BlendMode::Multiplicative
        } else {
            BlendMode::Normal
        }
    }

    #[cfg(feature = "probe")]
    unsafe fn snapshot_model(
        model: *mut live2d_sys::CsmModel,
        model_opacity: f32,
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
        let constant_flags = live2d_sys::csmGetDrawableConstantFlags(model);
        let dynamic_flags = live2d_sys::csmGetDrawableDynamicFlags(model);
        let render_orders = live2d_sys::csmGetRenderOrders(model);
        let texture_indices = live2d_sys::csmGetDrawableTextureIndices(model);
        let opacities = live2d_sys::csmGetDrawableOpacities(model);
        let mask_counts = live2d_sys::csmGetDrawableMaskCounts(model);
        let masks = live2d_sys::csmGetDrawableMasks(model);
        let vertex_counts = live2d_sys::csmGetDrawableVertexCounts(model);
        let vertex_positions = live2d_sys::csmGetDrawableVertexPositions(model);
        let vertex_uvs = live2d_sys::csmGetDrawableVertexUvs(model);
        let index_counts = live2d_sys::csmGetDrawableIndexCounts(model);
        let indices = live2d_sys::csmGetDrawableIndices(model);
        if ids.is_null()
            || constant_flags.is_null()
            || dynamic_flags.is_null()
            || render_orders.is_null()
            || texture_indices.is_null()
            || opacities.is_null()
            || mask_counts.is_null()
            || masks.is_null()
            || vertex_counts.is_null()
            || vertex_positions.is_null()
            || vertex_uvs.is_null()
            || index_counts.is_null()
            || indices.is_null()
        {
            return Err("live2d_model_snapshot_failed".into());
        }

        struct DrawableMeta {
            id: DrawableId,
            source_index: usize,
            render_order: i32,
            texture_index: usize,
            visible: bool,
            opacity: f32,
            blend_mode: BlendMode,
            clipping: Option<ClippingInfo>,
            mask_indices: Vec<usize>,
        }

        let opacity_scale = model_opacity.clamp(0.0, 1.0);
        let mut id_by_index = Vec::with_capacity(count);
        let mut art_meshes = Vec::new();
        for index in 0..count {
            let id_ptr = *ids.add(index);
            if id_ptr.is_null() {
                id_by_index.push(None);
                continue;
            }
            let id = CStr::from_ptr(id_ptr).to_string_lossy().into_owned();
            let drawable_id = DrawableId(id);
            art_meshes.push(ArtMeshInfo {
                id: drawable_id.clone(),
                label: drawable_id.0.clone(),
                original_name: drawable_id.0.clone(),
                index,
                mask_type: "unknown".into(),
            });
            id_by_index.push(Some(drawable_id));
        }

        let mut metas = Vec::new();
        for index in 0..count {
            let Some(drawable_id) = id_by_index[index].clone() else {
                continue;
            };
            let constant = *constant_flags.add(index);
            let dynamic = *dynamic_flags.add(index);
            let mask_count = (*mask_counts.add(index)).max(0) as usize;
            let mut mask_indices = Vec::new();
            let mut mask_ids = Vec::new();
            if mask_count > 0 {
                let mask_ptr = *masks.add(index);
                if mask_ptr.is_null() {
                    return Err("live2d_model_snapshot_failed".into());
                }
                for mask_index in std::slice::from_raw_parts(mask_ptr, mask_count) {
                    let mask_index = *mask_index;
                    if mask_index < 0 {
                        continue;
                    }
                    let mask_index = mask_index as usize;
                    if let Some(Some(mask_id)) = id_by_index.get(mask_index) {
                        mask_indices.push(mask_index);
                        mask_ids.push(mask_id.clone());
                    }
                }
            }
            let clipping = if mask_ids.is_empty() {
                None
            } else {
                Some(ClippingInfo {
                    drawable_ids: mask_ids,
                    inverted: false,
                })
            };
            let blend_mode = if constant & live2d_sys::csmBlendAdditive != 0 {
                BlendMode::Additive
            } else if constant & live2d_sys::csmBlendMultiplicative != 0 {
                BlendMode::Multiplicative
            } else {
                BlendMode::Normal
            };
            metas.push(DrawableMeta {
                id: drawable_id,
                source_index: index,
                render_order: *render_orders.add(index),
                texture_index: (*texture_indices.add(index)).max(0) as usize,
                visible: dynamic & live2d_sys::csmIsVisible != 0,
                opacity: (*opacities.add(index) * opacity_scale).clamp(0.0, 1.0),
                blend_mode,
                clipping,
                mask_indices,
            });
        }

        let mut retained_indices = vec![false; count];
        for meta in &metas {
            if meta.visible && meta.opacity > 1e-6 {
                retained_indices[meta.source_index] = true;
                for mask_index in &meta.mask_indices {
                    retained_indices[*mask_index] = true;
                }
            }
        }

        let mut drawables = Vec::with_capacity(metas.len());
        for meta in metas {
            let mut vertices = Vec::new();
            let mut drawable_indices = Vec::new();
            if retained_indices[meta.source_index] {
                let vertex_count = *vertex_counts.add(meta.source_index) as usize;
                let index_count = *index_counts.add(meta.source_index) as usize;
                let pos_ptr = *vertex_positions.add(meta.source_index);
                let uv_ptr = *vertex_uvs.add(meta.source_index);
                let index_ptr = *indices.add(meta.source_index);
                if pos_ptr.is_null() || uv_ptr.is_null() || index_ptr.is_null() {
                    return Err("live2d_model_snapshot_failed".into());
                }
                let positions = std::slice::from_raw_parts(pos_ptr, vertex_count);
                let uvs = std::slice::from_raw_parts(uv_ptr, vertex_count);
                vertices = positions
                    .iter()
                    .zip(uvs.iter())
                    .map(|(position, uv)| Vertex {
                        position: [position.x, position.y],
                        uv: [uv.x, uv.y],
                    })
                    .collect::<Vec<_>>();
                drawable_indices = std::slice::from_raw_parts(index_ptr, index_count).to_vec();
            }
            drawables.push(Drawable {
                id: meta.id,
                render_order: meta.render_order,
                texture_index: meta.texture_index,
                vertices,
                indices: drawable_indices,
                visible: meta.visible,
                opacity: meta.opacity,
                blend_mode: meta.blend_mode,
                clipping: meta.clipping,
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

    #[cfg(feature = "probe")]
    fn load_textures_with_probe<P>(
        paths: &[PathBuf],
        probe: &P,
    ) -> Result<Vec<TextureAsset>, String>
    where
        P: ProbeSink,
    {
        paths
            .iter()
            .map(|path| {
                measure(
                    probe,
                    Stage::RuntimeTextureDecode,
                    vec![ProbeAttr::new("path", path.to_string_lossy().into_owned())],
                    || {
                        let image = image::open(path)
                            .map_err(|_| "live2d_texture_unreadable")?
                            .into_rgba8();
                        let (width, height) = image.dimensions();
                        let rgba = image.into_raw();
                        counter(
                            probe,
                            Stage::RuntimeTextureDecode,
                            "bytes",
                            rgba.len() as u64,
                            Vec::new(),
                        );
                        Ok(TextureAsset {
                            width,
                            height,
                            rgba,
                        })
                    },
                )
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
        fs::create_dir_all(root.join("motions")).unwrap();
        fs::write(root.join("texture_ok.png"), "").unwrap();
        fs::write(root.join("motions/idle.motion3.json"), test_motion_json()).unwrap();
        let model = root.join("sample.model3.json");
        fs::write(
            &model,
            r#"{
                "FileReferences": {
                    "Moc": "sample.moc3",
                    "Textures": ["texture_ok.png", "textures/missing.png"],
                    "Motions": {
                        "TapBody": [
                            { "File": "motions/tap.motion3.json", "FadeInTime": 0.25, "FadeOutTime": 0.5 }
                        ],
                        "Idle": [
                            { "File": "motions/idle.motion3.json", "FadeInTime": 0.75 }
                        ]
                    }
                }
            }"#,
        )
        .unwrap();

        let files = resolve_model_files(&model).unwrap();

        assert_eq!(files.moc_path, root.join("sample.moc3"));
        assert_eq!(files.texture_paths.len(), 2);
        assert_eq!(files.motion_groups.len(), 2);
        let idle = files
            .motion_groups
            .iter()
            .find(|group| group.name == "Idle")
            .unwrap();
        assert_eq!(idle.motions[0].path, root.join("motions/idle.motion3.json"));
        assert_eq!(idle.motions[0].relative_path, "motions/idle.motion3.json");
        assert_eq!(idle.motions[0].fade_in_seconds, Some(0.75));
        assert_eq!(idle.motions[0].fade_out_seconds, None);
        assert_eq!(idle.motions[0].load_motion().unwrap().duration(), 2.0);
        assert_eq!(idle.motions[0].play_options(true).fade_in_seconds, 0.75);
        assert!(idle.motions[0].play_options(true).loop_playback);
        let tap = files
            .motion_groups
            .iter()
            .find(|group| group.name == "TapBody")
            .unwrap();
        assert_eq!(tap.motions[0].fade_in_seconds, Some(0.25));
        assert_eq!(tap.motions[0].fade_out_seconds, Some(0.5));
        assert_eq!(
            files.missing_files,
            vec![
                "motions/tap.motion3.json",
                "sample.moc3",
                "textures/missing.png"
            ]
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn instance_update_advances_active_motion_without_faking_runtime() {
        let mut instance = Live2DInstance {
            snapshot: empty_snapshot(),
            elapsed_seconds: 0.0,
            motion_player: MotionPlayer::new(),
            motion_evaluation: MotionEvaluation::default(),
        };
        instance.motion_player.play(test_motion(), false);

        let result = instance.update(0.5);

        assert_eq!(result.unwrap_err(), RUNTIME_UNAVAILABLE);
        assert_eq!(instance.motion_state(), MotionPlaybackState::Playing);
        assert!((instance.motion_elapsed_seconds() - 0.5).abs() < 0.001);

        instance.pause_motion();
        instance.update(0.5).unwrap();
        assert_eq!(instance.motion_state(), MotionPlaybackState::Paused);
        assert!((instance.motion_elapsed_seconds() - 0.5).abs() < 0.001);

        instance.resume_motion();
        let result = instance.update(2.0);
        assert_eq!(result.unwrap_err(), RUNTIME_UNAVAILABLE);
        assert_eq!(instance.motion_state(), MotionPlaybackState::Finished);
        assert!((instance.motion_elapsed_seconds() - 2.0).abs() < 0.001);

        instance.stop_motion();
        assert_eq!(instance.motion_state(), MotionPlaybackState::Stopped);
        instance.update(1.0).unwrap();
        assert_eq!(instance.motion_state(), MotionPlaybackState::Stopped);
    }

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn batch_update_advances_instances_without_active_motion() {
        let mut first = test_instance();
        let mut second = test_instance();

        let changed = update_instances([&mut first, &mut second], 0.25).unwrap();

        assert_eq!(changed, 0);
        assert!((first.elapsed_seconds() - 0.25).abs() < 0.001);
        assert!((second.elapsed_seconds() - 0.25).abs() < 0.001);

        let mut changed_indices = vec![99];
        let changed =
            update_instances_into([&mut first, &mut second], 0.25, &mut changed_indices).unwrap();
        assert_eq!(changed, 0);
        assert!(changed_indices.is_empty());
    }

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn instance_update_exposes_motion_events_without_snapshot_change() {
        let mut instance = test_instance();
        instance.play_motion(event_motion(), false).unwrap();

        let changed = instance.update_motion(0.6).unwrap();

        assert!(!changed);
        assert_eq!(instance.motion_events()[0].value, "blink");
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

    #[test]
    #[cfg(not(feature = "live2d-cubism"))]
    fn reports_unavailable_metadata_without_fake_art_meshes() {
        let root = unique_temp_dir("live2d-runtime-metadata-unavailable");
        fs::create_dir_all(&root).unwrap();
        let moc = root.join("sample.moc3");
        let model = root.join("sample.model3.json");
        fs::write(&model, r#"{"FileReferences":{"Moc":"sample.moc3"}}"#).unwrap();
        fs::write(&moc, b"noise\0ArtMeshZ\0").unwrap();

        let result = inspect_art_mesh_metadata(model);

        assert_eq!(result.unwrap_err(), "live2d_runtime_unavailable");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(feature = "live2d-cubism")]
    fn sdk_metadata_inspection_reads_art_meshes_when_test_model_is_available() -> Result<(), String>
    {
        let Ok(model_path) = std::env::var("LIVE2D_CUBISM_TEST_MODEL") else {
            return Ok(());
        };

        let meshes = inspect_art_mesh_metadata(model_path)?;

        assert!(!meshes.is_empty());
        assert!(meshes
            .iter()
            .all(|mesh| mesh.mask_type == "plain" || mesh.mask_type == "masked"));
        Ok(())
    }

    #[test]
    #[cfg(feature = "live2d-cubism")]
    fn sdk_parameter_write_refreshes_snapshot_when_test_model_is_available() -> Result<(), String> {
        let Ok(model_path) = std::env::var("LIVE2D_CUBISM_TEST_MODEL") else {
            return Ok(());
        };
        let mut instance = Live2DInstance::load_file(model_path)?;
        let initial_drawables = instance.snapshot().drawables.clone();
        let parameters = instance.parameters()?;

        for parameter in parameters.iter().filter(|parameter| {
            parameter.minimum.is_finite()
                && parameter.maximum.is_finite()
                && parameter.minimum < parameter.maximum
        }) {
            let target = if (parameter.value - parameter.minimum).abs() > 0.001 {
                parameter.minimum
            } else {
                parameter.maximum
            };
            instance.set_parameter(parameter.id.as_ref(), target)?;
            let current = instance
                .parameter(parameter.id.as_ref())?
                .ok_or_else(|| "live2d_parameter_not_found".to_string())?
                .value;
            assert_close(current, target);

            if snapshot_vertices_changed(&initial_drawables, &instance.snapshot().drawables) {
                return Ok(());
            }
            instance.reset_parameters()?;
        }

        Err("live2d_parameter_snapshot_unchanged".into())
    }

    #[cfg(feature = "live2d-cubism")]
    fn snapshot_vertices_changed(before: &[Drawable], after: &[Drawable]) -> bool {
        before.iter().any(|before_drawable| {
            let Some(after_drawable) = after
                .iter()
                .find(|after_drawable| after_drawable.id == before_drawable.id)
            else {
                return true;
            };
            before_drawable.vertices.len() != after_drawable.vertices.len()
                || before_drawable
                    .vertices
                    .iter()
                    .zip(after_drawable.vertices.iter())
                    .any(|(before_vertex, after_vertex)| {
                        (before_vertex.position[0] - after_vertex.position[0]).abs() > 0.001
                            || (before_vertex.position[1] - after_vertex.position[1]).abs() > 0.001
                    })
        })
    }

    #[cfg(feature = "live2d-cubism")]
    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.001);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}"))
    }

    #[cfg(not(feature = "live2d-cubism"))]
    fn empty_snapshot() -> ModelSnapshot {
        ModelSnapshot {
            model_key: "test".into(),
            canvas: live2d_core::CanvasInfo::default(),
            art_meshes: Vec::new(),
            drawables: Vec::new(),
            textures: Vec::new(),
        }
    }

    #[cfg(not(feature = "live2d-cubism"))]
    fn test_instance() -> Live2DInstance {
        Live2DInstance {
            snapshot: empty_snapshot(),
            elapsed_seconds: 0.0,
            motion_player: MotionPlayer::new(),
            motion_evaluation: MotionEvaluation::default(),
        }
    }

    #[cfg(not(feature = "live2d-cubism"))]
    fn test_motion() -> Live2DMotion {
        Live2DMotion::from_json_str(test_motion_json()).unwrap()
    }

    #[cfg(not(feature = "live2d-cubism"))]
    fn event_motion() -> Live2DMotion {
        Live2DMotion::from_json_str(
            r#"{
                "Meta": { "Duration": 1.0, "Loop": false },
                "Curves": [],
                "UserData": [
                    { "Time": 0.5, "Value": "blink" }
                ]
            }"#,
        )
        .unwrap()
    }

    fn test_motion_json() -> &'static str {
        r#"{
                "Meta": { "Duration": 2.0, "Loop": false },
                "Curves": [
                    { "Target": "Parameter", "Id": "ParamAngleX", "Segments": [0, 0, 0, 2, 10] }
                ]
            }"#
    }
}
