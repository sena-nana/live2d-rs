use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    RuntimeModel3Read,
    RuntimeModel3Parse,
    RuntimeAssetResolve,
    RuntimeTextureDecode,
    RuntimeMocRead,
    RuntimeMocRevive,
    RuntimeModelAllocation,
    RuntimeModelInit,
    RuntimeModelUpdate,
    RuntimeMotionUpdate,
    RuntimeSnapshotExtract,
    RuntimeLoadSnapshot,
    RenderPlanTotal,
    RenderModelCtxBuild,
    RenderOrderSort,
    RenderMaskDedup,
    RenderDrawCommandBuild,
    RenderDispatchTotal,
    RenderMaskLookup,
    RenderMainDrawDispatch,
    WgpuRendererInit,
    WgpuPipelineCreation,
    WgpuPrepareRender,
    WgpuTextureCacheHit,
    WgpuTextureCacheMiss,
    WgpuTextureUpload,
    WgpuSceneTopologyHit,
    WgpuSceneTopologyMiss,
    WgpuPositionUpload,
    WgpuBufferRebuild,
    WgpuUniformCapacityGrow,
    WgpuOffscreenResize,
    WgpuMaskAtlasLayout,
    WgpuMaskAtlasRebuild,
    WgpuMaskPassEncode,
    WgpuMainPassEncode,
    WgpuPostProcessPassEncode,
    WgpuQueueSubmit,
    WgpuGpuTimestampSupport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeAttr {
    pub key: String,
    pub value: ProbeValue,
}

impl ProbeAttr {
    pub fn new(key: impl Into<String>, value: impl Into<ProbeValue>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProbeValue {
    Str(String),
    U64(u64),
    I64(i64),
    F64(f64),
    Bool(bool),
}

impl From<&str> for ProbeValue {
    fn from(value: &str) -> Self {
        Self::Str(value.to_owned())
    }
}

impl From<String> for ProbeValue {
    fn from(value: String) -> Self {
        Self::Str(value)
    }
}

impl From<usize> for ProbeValue {
    fn from(value: usize) -> Self {
        Self::U64(value as u64)
    }
}

impl From<u64> for ProbeValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<u32> for ProbeValue {
    fn from(value: u32) -> Self {
        Self::U64(value as u64)
    }
}

impl From<i64> for ProbeValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<f64> for ProbeValue {
    fn from(value: f64) -> Self {
        Self::F64(value)
    }
}

impl From<f32> for ProbeValue {
    fn from(value: f32) -> Self {
        Self::F64(value as f64)
    }
}

impl From<bool> for ProbeValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpanRecord {
    pub stage: Stage,
    pub elapsed_nanos: u64,
    pub depth: usize,
    pub parent_stage: Option<Stage>,
    pub attrs: Vec<ProbeAttr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CounterRecord {
    pub stage: Stage,
    pub name: String,
    pub value: u64,
    pub attrs: Vec<ProbeAttr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GaugeRecord {
    pub stage: Stage,
    pub name: String,
    pub value: f64,
    pub attrs: Vec<ProbeAttr>,
}

pub trait ProbeSink {
    fn record_span(&self, span: SpanRecord);
    fn record_counter(&self, counter: CounterRecord);
    fn record_gauge(&self, gauge: GaugeRecord);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopProbe;

impl ProbeSink for NoopProbe {
    fn record_span(&self, _span: SpanRecord) {}
    fn record_counter(&self, _counter: CounterRecord) {}
    fn record_gauge(&self, _gauge: GaugeRecord) {}
}

#[derive(Debug, Default, Clone)]
pub struct ProbeRecorder {
    inner: Arc<Mutex<ProbeData>>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeData {
    pub spans: Vec<SpanRecord>,
    pub counters: Vec<CounterRecord>,
    pub gauges: Vec<GaugeRecord>,
}

impl ProbeRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn data(&self) -> ProbeData {
        self.inner.lock().expect("probe recorder poisoned").clone()
    }

    pub fn report(
        &self,
        scenario: impl Into<String>,
        config: BTreeMap<String, String>,
        warnings: Vec<String>,
    ) -> RunReport {
        RunReport {
            scenario: scenario.into(),
            config,
            environment: EnvironmentReport::current(),
            data: self.data(),
            analysis: ProbeAnalysis::from_data(&self.data()),
            warnings,
        }
    }
}

impl ProbeSink for ProbeRecorder {
    fn record_span(&self, span: SpanRecord) {
        self.inner
            .lock()
            .expect("probe recorder poisoned")
            .spans
            .push(span);
    }

    fn record_counter(&self, counter: CounterRecord) {
        self.inner
            .lock()
            .expect("probe recorder poisoned")
            .counters
            .push(counter);
    }

    fn record_gauge(&self, gauge: GaugeRecord) {
        self.inner
            .lock()
            .expect("probe recorder poisoned")
            .gauges
            .push(gauge);
    }
}

thread_local! {
    static SPAN_STACK: RefCell<Vec<Stage>> = const { RefCell::new(Vec::new()) };
}

pub fn measure<T, F>(probe: &impl ProbeSink, stage: Stage, attrs: Vec<ProbeAttr>, f: F) -> T
where
    F: FnOnce() -> T,
{
    let (depth, parent_stage) = SPAN_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let depth = stack.len();
        let parent_stage = stack.last().copied();
        stack.push(stage);
        (depth, parent_stage)
    });
    let started = Instant::now();
    let result = f();
    let elapsed_nanos = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
    SPAN_STACK.with(|stack| {
        let popped = stack.borrow_mut().pop();
        debug_assert_eq!(popped, Some(stage));
    });
    probe.record_span(SpanRecord {
        stage,
        elapsed_nanos,
        depth,
        parent_stage,
        attrs,
    });
    result
}

pub fn counter(
    probe: &impl ProbeSink,
    stage: Stage,
    name: impl Into<String>,
    value: u64,
    attrs: Vec<ProbeAttr>,
) {
    probe.record_counter(CounterRecord {
        stage,
        name: name.into(),
        value,
        attrs,
    });
}

pub fn gauge(
    probe: &impl ProbeSink,
    stage: Stage,
    name: impl Into<String>,
    value: f64,
    attrs: Vec<ProbeAttr>,
) {
    probe.record_gauge(GaugeRecord {
        stage,
        name: name.into(),
        value,
        attrs,
    });
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunReport {
    pub scenario: String,
    pub config: BTreeMap<String, String>,
    pub environment: EnvironmentReport,
    pub data: ProbeData,
    pub analysis: ProbeAnalysis,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentReport {
    pub os: String,
    pub arch: String,
    pub rust_debug_assertions: bool,
}

impl EnvironmentReport {
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            rust_debug_assertions: cfg!(debug_assertions),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeAnalysis {
    pub stages: BTreeMap<Stage, StageStats>,
    pub gauges: BTreeMap<String, f64>,
}

impl ProbeAnalysis {
    pub fn from_data(data: &ProbeData) -> Self {
        let mut stages = BTreeMap::<Stage, StageStatsBuilder>::new();
        for span in &data.spans {
            stages
                .entry(span.stage)
                .or_default()
                .elapsed
                .push(span.elapsed_nanos);
            if let Some(parent) = span.parent_stage {
                let parent_stats = stages.entry(parent).or_default();
                parent_stats.children_nanos = parent_stats
                    .children_nanos
                    .saturating_add(span.elapsed_nanos);
            }
        }
        for counter in &data.counters {
            let stats = stages.entry(counter.stage).or_default();
            *stats.counters.entry(counter.name.clone()).or_default() += counter.value;
        }

        let gauges = data
            .gauges
            .iter()
            .map(|gauge| (format!("{:?}.{}", gauge.stage, gauge.name), gauge.value))
            .collect::<BTreeMap<_, _>>();

        Self {
            stages: stages
                .into_iter()
                .map(|(stage, builder)| (stage, builder.finish()))
                .collect(),
            gauges,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StageStats {
    pub calls: u64,
    pub total_nanos: u64,
    pub self_nanos: u64,
    pub children_nanos: u64,
    pub min_nanos: u64,
    pub p50_nanos: u64,
    pub p90_nanos: u64,
    pub p99_nanos: u64,
    pub max_nanos: u64,
    pub bytes: u64,
    pub draw_calls: u64,
    pub buffer_writes: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub resource_rebuilds: u64,
    pub counters: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default)]
struct StageStatsBuilder {
    elapsed: Vec<u64>,
    children_nanos: u64,
    counters: BTreeMap<String, u64>,
}

impl StageStatsBuilder {
    fn finish(mut self) -> StageStats {
        self.elapsed.sort_unstable();
        let total_nanos = self.elapsed.iter().copied().sum();
        let calls = self.elapsed.len() as u64;
        let counters = self.counters;
        let bytes = *counters.get("bytes").unwrap_or(&0);
        let draw_calls = *counters.get("draw_calls").unwrap_or(&0);
        let buffer_writes = *counters.get("buffer_writes").unwrap_or(&0);
        let cache_hits = *counters.get("cache_hits").unwrap_or(&0);
        let cache_misses = *counters.get("cache_misses").unwrap_or(&0);
        let resource_rebuilds = *counters.get("resource_rebuilds").unwrap_or(&0);
        StageStats {
            calls,
            total_nanos,
            self_nanos: total_nanos.saturating_sub(self.children_nanos),
            children_nanos: self.children_nanos,
            min_nanos: self.elapsed.first().copied().unwrap_or_default(),
            p50_nanos: percentile(&self.elapsed, 50),
            p90_nanos: percentile(&self.elapsed, 90),
            p99_nanos: percentile(&self.elapsed, 99),
            max_nanos: self.elapsed.last().copied().unwrap_or_default(),
            bytes,
            draw_calls,
            buffer_writes,
            cache_hits,
            cache_misses,
            resource_rebuilds,
            counters,
        }
    }
}

fn percentile(values: &[u64], pct: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) * pct).div_ceil(100);
    values[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_aggregates_spans_and_counters_by_stage() {
        let recorder = ProbeRecorder::new();
        recorder.record_span(SpanRecord {
            stage: Stage::RenderPlanTotal,
            elapsed_nanos: 100,
            depth: 0,
            parent_stage: None,
            attrs: Vec::new(),
        });
        recorder.record_span(SpanRecord {
            stage: Stage::RenderPlanTotal,
            elapsed_nanos: 300,
            depth: 0,
            parent_stage: None,
            attrs: Vec::new(),
        });
        recorder.record_span(SpanRecord {
            stage: Stage::RenderModelCtxBuild,
            elapsed_nanos: 80,
            depth: 1,
            parent_stage: Some(Stage::RenderPlanTotal),
            attrs: Vec::new(),
        });
        counter(
            &recorder,
            Stage::RenderPlanTotal,
            "draw_calls",
            7,
            Vec::new(),
        );
        counter(&recorder, Stage::RenderPlanTotal, "bytes", 64, Vec::new());

        let analysis = ProbeAnalysis::from_data(&recorder.data());
        let plan = analysis.stages.get(&Stage::RenderPlanTotal).unwrap();

        assert_eq!(plan.calls, 2);
        assert_eq!(plan.total_nanos, 400);
        assert_eq!(plan.children_nanos, 80);
        assert_eq!(plan.self_nanos, 320);
        assert_eq!(plan.min_nanos, 100);
        assert_eq!(plan.p50_nanos, 300);
        assert_eq!(plan.p90_nanos, 300);
        assert_eq!(plan.draw_calls, 7);
        assert_eq!(plan.bytes, 64);
    }

    #[test]
    fn measure_tracks_parent_child_relationships() {
        let recorder = ProbeRecorder::new();
        measure(&recorder, Stage::RenderPlanTotal, Vec::new(), || {
            measure(&recorder, Stage::RenderModelCtxBuild, Vec::new(), || {});
        });

        let data = recorder.data();
        let child = data
            .spans
            .iter()
            .find(|span| span.stage == Stage::RenderModelCtxBuild)
            .unwrap();

        assert_eq!(child.depth, 1);
        assert_eq!(child.parent_stage, Some(Stage::RenderPlanTotal));
    }
}
