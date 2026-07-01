use live2d_core::{
    BlendMode, ClippingInfo, Drawable, DrawableId, DrawableRanges, MaskRef, MaterialKey,
    ModelSnapshot, Offscreen, RenderObject,
};
#[cfg(feature = "probe")]
use live2d_probe::{counter, measure, ProbeAttr, ProbeSink, Stage};
use std::collections::{HashMap, HashSet};
use std::ops::Range;

const DRAW_LOOKUP_INDEX_THRESHOLD: usize = 8 * 1024;
const DRAWABLE_OPACITY_EPSILON: f32 = 1e-6;
pub const POST_PROCESS_PARAM_VEC4S: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub model: ModelRenderCtx,
    pub masks: Vec<MaskPass>,
    pub mask_draws: Vec<DrawCommand>,
    pub draws: Vec<DrawCommand>,
    pub offscreens: Vec<Offscreen>,
    pub commands: Vec<RenderCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRenderCtx {
    pub vertex_count: u32,
    pub index_count: u32,
    pub drawables: Vec<DrawableRenderCtx>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawableRenderCtx {
    pub drawable_id: DrawableId,
    pub texture_index: usize,
    pub ranges: DrawableRanges,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct DrawableTable {
    vertex_ranges: Vec<Range<u32>>,
    index_ranges: Vec<Range<u32>>,
    render_orders: Vec<i32>,
    source_indices: Vec<usize>,
}

impl DrawableTable {
    fn len(&self) -> usize {
        self.source_indices.len()
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
struct MaskGroupTable {
    masks: Vec<MaskPass>,
    mask_refs_by_row: Vec<Option<MaskRef>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskPass {
    pub id: MaskRef,
    pub drawable_ids: Vec<DrawableId>,
    pub inverted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawCommand {
    pub drawable_id: DrawableId,
    pub texture_index: usize,
    pub vertex_range: Range<u32>,
    pub index_range: Range<u32>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub mask: Option<MaskRef>,
    pub inverted_mask: bool,
    pub material: MaterialKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderCommand {
    BeginOffscreen { offscreen_index: usize },
    Draw { draw_index: usize },
    CompositeOffscreen { offscreen_index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PostProcessShaderId(pub String);

impl From<String> for PostProcessShaderId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for PostProcessShaderId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for PostProcessShaderId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostProcessInput {
    Scene,
    Pass(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostProcessOutput {
    Temporary,
    Final,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostProcessParams {
    pub values: [[f32; 4]; POST_PROCESS_PARAM_VEC4S],
}

impl Default for PostProcessParams {
    fn default() -> Self {
        Self {
            values: [[0.0; 4]; POST_PROCESS_PARAM_VEC4S],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostProcessPass {
    pub shader_id: PostProcessShaderId,
    pub input: PostProcessInput,
    pub output: PostProcessOutput,
    pub params: PostProcessParams,
}

impl PostProcessPass {
    pub fn new(
        shader_id: impl Into<PostProcessShaderId>,
        input: PostProcessInput,
        output: PostProcessOutput,
    ) -> Self {
        Self {
            shader_id: shader_id.into(),
            input,
            output,
            params: PostProcessParams::default(),
        }
    }

    pub fn with_params(mut self, params: PostProcessParams) -> Self {
        self.params = params;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostProcessPlan {
    passes: Vec<PostProcessPass>,
}

impl PostProcessPlan {
    pub fn empty() -> Self {
        Self { passes: Vec::new() }
    }

    pub fn linear<I, S>(shader_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<PostProcessShaderId>,
    {
        let shader_ids = shader_ids.into_iter().map(Into::into).collect::<Vec<_>>();
        let pass_count = shader_ids.len();
        let passes = shader_ids
            .into_iter()
            .enumerate()
            .map(|(index, shader_id)| PostProcessPass {
                shader_id,
                input: if index == 0 {
                    PostProcessInput::Scene
                } else {
                    PostProcessInput::Pass(index - 1)
                },
                output: if index + 1 == pass_count {
                    PostProcessOutput::Final
                } else {
                    PostProcessOutput::Temporary
                },
                params: PostProcessParams::default(),
            })
            .collect();
        Self { passes }
    }

    pub fn new(passes: Vec<PostProcessPass>) -> Result<Self, PostProcessPlanError> {
        validate_post_process_passes(&passes)?;
        Ok(Self { passes })
    }

    pub fn passes(&self) -> &[PostProcessPass] {
        &self.passes
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }
}

impl Default for PostProcessPlan {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostProcessPlanError {
    EmptyShaderId {
        pass: usize,
    },
    UnsupportedInput {
        pass: usize,
        input: PostProcessInput,
    },
    FinalOutputBeforeLast {
        pass: usize,
    },
    MissingFinalOutput,
}

fn validate_post_process_passes(passes: &[PostProcessPass]) -> Result<(), PostProcessPlanError> {
    if passes.is_empty() {
        return Ok(());
    }

    for (index, pass) in passes.iter().enumerate() {
        if pass.shader_id.as_ref().is_empty() {
            return Err(PostProcessPlanError::EmptyShaderId { pass: index });
        }

        let expected_input = if index == 0 {
            PostProcessInput::Scene
        } else {
            PostProcessInput::Pass(index - 1)
        };
        if pass.input != expected_input {
            return Err(PostProcessPlanError::UnsupportedInput {
                pass: index,
                input: pass.input,
            });
        }

        if pass.output == PostProcessOutput::Final && index + 1 != passes.len() {
            return Err(PostProcessPlanError::FinalOutputBeforeLast { pass: index });
        }
    }

    if passes.last().unwrap().output != PostProcessOutput::Final {
        return Err(PostProcessPlanError::MissingFinalOutput);
    }

    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct RenderPlanner;

#[derive(Debug, Clone, Default)]
pub struct RenderWorld {
    caches: HashMap<String, RenderWorldCache>,
}

#[derive(Debug, Clone)]
struct RenderWorldCache {
    model: ModelRenderCtx,
    table: DrawableTable,
    ordered_rows: Vec<usize>,
    mask_table: MaskGroupTable,
    clipping_by_row: Vec<Option<ClippingInfo>>,
    draw_enabled_by_row: Vec<bool>,
}

pub trait Live2DRenderBackend {
    fn begin_model(&mut self, _ctx: &ModelRenderCtx) {}
    fn begin_clip_masks(&mut self, _masks: &[MaskPass]) {}
    fn begin_clip_mask(&mut self, _mask: &MaskPass) {}
    fn draw_mask_drawable(&mut self, _mask: &MaskPass, _call: &DrawCommand) {}
    fn end_clip_mask(&mut self, _mask: &MaskPass) {}
    fn end_clip_masks(&mut self) {}
    fn begin_main_pass(&mut self) {}
    fn draw_drawable(&mut self, call: &DrawCommand);
    fn end_model(&mut self) {}
}

impl RenderPlan {
    pub fn dispatch<B>(&self, backend: &mut B)
    where
        B: Live2DRenderBackend,
    {
        backend.begin_model(&self.model);
        if !self.masks.is_empty() {
            let draw_lookup = DrawLookup::new(&self.mask_draws, mask_drawable_count(&self.masks));
            backend.begin_clip_masks(&self.masks);
            for mask in &self.masks {
                backend.begin_clip_mask(mask);
                for drawable_id in &mask.drawable_ids {
                    if let Some(call) = draw_lookup.get(drawable_id) {
                        backend.draw_mask_drawable(mask, call);
                    }
                }
                backend.end_clip_mask(mask);
            }
            backend.end_clip_masks();
        }
        backend.begin_main_pass();
        for draw in &self.draws {
            backend.draw_drawable(draw);
        }
        backend.end_model();
    }

    #[cfg(feature = "probe")]
    pub fn dispatch_with_probe<B, P>(&self, backend: &mut B, probe: &P)
    where
        B: Live2DRenderBackend,
        P: ProbeSink,
    {
        measure(
            probe,
            Stage::RenderDispatchTotal,
            vec![
                ProbeAttr::new("draws", self.draws.len()),
                ProbeAttr::new("masks", self.masks.len()),
            ],
            || {
                backend.begin_model(&self.model);
                if !self.masks.is_empty() {
                    let draw_lookup =
                        DrawLookup::new(&self.mask_draws, mask_drawable_count(&self.masks));
                    backend.begin_clip_masks(&self.masks);
                    for mask in &self.masks {
                        backend.begin_clip_mask(mask);
                        for drawable_id in &mask.drawable_ids {
                            let call = measure(
                                probe,
                                Stage::RenderMaskLookup,
                                vec![ProbeAttr::new("mask", mask.id.0)],
                                || draw_lookup.get(drawable_id),
                            );
                            if let Some(call) = call {
                                backend.draw_mask_drawable(mask, call);
                                counter(
                                    probe,
                                    Stage::RenderMaskLookup,
                                    "draw_calls",
                                    1,
                                    Vec::new(),
                                );
                            }
                        }
                        backend.end_clip_mask(mask);
                    }
                    backend.end_clip_masks();
                }
                backend.begin_main_pass();
                for draw in &self.draws {
                    measure(
                        probe,
                        Stage::RenderMainDrawDispatch,
                        vec![ProbeAttr::new("drawable", draw.drawable_id.as_ref())],
                        || backend.draw_drawable(draw),
                    );
                    counter(
                        probe,
                        Stage::RenderMainDrawDispatch,
                        "draw_calls",
                        1,
                        Vec::new(),
                    );
                }
                backend.end_model();
            },
        );
    }
}

impl RenderPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn build(&self, snapshot: &ModelSnapshot) -> RenderPlan {
        let (model, table) = build_model_ctx_and_table(snapshot);
        let ordered_rows = sorted_drawable_rows(&table);
        let mask_table = build_mask_group_table(snapshot, &table, &ordered_rows);

        let mask_draws = mask_draw_commands_from_table(snapshot, &table, &mask_table);
        let draws = draw_commands_for_visible_rows(snapshot, &table, &ordered_rows, &mask_table);
        let commands = render_commands(snapshot, &draws);

        RenderPlan {
            model,
            masks: mask_table.masks,
            mask_draws,
            draws,
            offscreens: snapshot.offscreens.clone(),
            commands,
        }
    }

    #[cfg(feature = "probe")]
    pub fn build_with_probe<P>(&self, snapshot: &ModelSnapshot, probe: &P) -> RenderPlan
    where
        P: ProbeSink,
    {
        measure(
            probe,
            Stage::RenderPlanTotal,
            vec![
                ProbeAttr::new("drawables", snapshot.drawables.len()),
                ProbeAttr::new("textures", snapshot.textures.len()),
            ],
            || {
                let (model, table) = measure(probe, Stage::RenderModelCtxBuild, Vec::new(), || {
                    build_model_ctx_and_table(snapshot)
                });
                let ordered_rows = measure(probe, Stage::RenderOrderSort, Vec::new(), || {
                    sorted_drawable_rows(&table)
                });

                let mask_table = measure(
                    probe,
                    Stage::RenderMaskDedup,
                    vec![ProbeAttr::new("candidate_drawables", ordered_rows.len())],
                    || build_mask_group_table(snapshot, &table, &ordered_rows),
                );
                let mask_draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new("candidate_drawables", ordered_rows.len())],
                    || mask_draw_commands_from_table(snapshot, &table, &mask_table),
                );
                let draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new("candidate_drawables", ordered_rows.len())],
                    || draw_commands_for_visible_rows(snapshot, &table, &ordered_rows, &mask_table),
                );
                let commands = render_commands(snapshot, &draws);

                counter(
                    probe,
                    Stage::RenderPlanTotal,
                    "draw_calls",
                    draws.len() as u64,
                    Vec::new(),
                );
                counter(
                    probe,
                    Stage::RenderPlanTotal,
                    "resource_rebuilds",
                    mask_table.masks.len() as u64,
                    vec![ProbeAttr::new("resource", "mask_pass")],
                );
                RenderPlan {
                    model,
                    masks: mask_table.masks,
                    mask_draws,
                    draws,
                    offscreens: snapshot.offscreens.clone(),
                    commands,
                }
            },
        )
    }
}

impl RenderWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.caches.clear();
    }

    pub fn build(&mut self, snapshot: &ModelSnapshot) -> RenderPlan {
        self.ensure_cache(snapshot);
        let cache = self
            .caches
            .get_mut(&snapshot.model_key)
            .expect("render world cache is initialized");
        refresh_cached_render_order(snapshot, cache);
        render_plan_from_cache(snapshot, cache)
    }

    #[cfg(feature = "probe")]
    pub fn build_with_probe<P>(&mut self, snapshot: &ModelSnapshot, probe: &P) -> RenderPlan
    where
        P: ProbeSink,
    {
        measure(
            probe,
            Stage::RenderPlanTotal,
            vec![
                ProbeAttr::new("drawables", snapshot.drawables.len()),
                ProbeAttr::new("textures", snapshot.textures.len()),
            ],
            || {
                let cache_rebuild = self
                    .caches
                    .get(&snapshot.model_key)
                    .map_or(true, |cache| !render_world_cache_matches(cache, snapshot));
                if cache_rebuild {
                    let (model, table) =
                        measure(probe, Stage::RenderModelCtxBuild, Vec::new(), || {
                            build_model_ctx_and_table(snapshot)
                        });
                    let ordered_rows = measure(probe, Stage::RenderOrderSort, Vec::new(), || {
                        sorted_drawable_rows(&table)
                    });
                    let mask_table = measure(
                        probe,
                        Stage::RenderMaskDedup,
                        vec![ProbeAttr::new("candidate_drawables", ordered_rows.len())],
                        || build_mask_group_table(snapshot, &table, &ordered_rows),
                    );
                    self.caches.insert(
                        snapshot.model_key.clone(),
                        RenderWorldCache {
                            clipping_by_row: clipping_by_row(snapshot, &table),
                            draw_enabled_by_row: draw_enabled_by_row(snapshot, &table),
                            model,
                            table,
                            ordered_rows,
                            mask_table,
                        },
                    );
                }

                let cache = self
                    .caches
                    .get_mut(&snapshot.model_key)
                    .expect("render world cache is initialized");
                let order_changed = if cache_rebuild {
                    false
                } else {
                    measure(probe, Stage::RenderOrderSort, Vec::new(), || {
                        refresh_cached_render_order(snapshot, cache)
                    })
                };
                if order_changed {
                    cache.mask_table = measure(
                        probe,
                        Stage::RenderMaskDedup,
                        vec![ProbeAttr::new(
                            "candidate_drawables",
                            cache.ordered_rows.len(),
                        )],
                        || build_mask_group_table(snapshot, &cache.table, &cache.ordered_rows),
                    );
                }
                let mask_draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new(
                        "candidate_drawables",
                        cache.ordered_rows.len(),
                    )],
                    || mask_draw_commands_from_table(snapshot, &cache.table, &cache.mask_table),
                );
                let draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new(
                        "candidate_drawables",
                        cache.ordered_rows.len(),
                    )],
                    || draw_commands_from_cache(snapshot, cache),
                );
                let commands = render_commands(snapshot, &draws);
                counter(
                    probe,
                    Stage::RenderPlanTotal,
                    "draw_calls",
                    draws.len() as u64,
                    Vec::new(),
                );
                counter(
                    probe,
                    Stage::RenderPlanTotal,
                    "resource_rebuilds",
                    cache.mask_table.masks.len() as u64,
                    vec![ProbeAttr::new("resource", "mask_pass")],
                );
                RenderPlan {
                    model: cache.model.clone(),
                    masks: cache.mask_table.masks.clone(),
                    mask_draws,
                    draws,
                    offscreens: snapshot.offscreens.clone(),
                    commands,
                }
            },
        )
    }

    fn ensure_cache(&mut self, snapshot: &ModelSnapshot) {
        let rebuild = self
            .caches
            .get(&snapshot.model_key)
            .map_or(true, |cache| !render_world_cache_matches(cache, snapshot));
        if rebuild {
            self.caches.insert(
                snapshot.model_key.clone(),
                build_render_world_cache(snapshot),
            );
        }
    }
}

fn build_model_ctx_and_table(snapshot: &ModelSnapshot) -> (ModelRenderCtx, DrawableTable) {
    let table = build_drawable_table(snapshot);
    let model = ModelRenderCtx {
        vertex_count: table.vertex_ranges.last().map_or(0, |range| range.end),
        index_count: table.index_ranges.last().map_or(0, |range| range.end),
        drawables: (0..table.len())
            .map(|row| DrawableRenderCtx {
                drawable_id: snapshot.drawables[table.source_indices[row]].id.clone(),
                texture_index: snapshot.drawables[table.source_indices[row]].texture_index,
                ranges: DrawableRanges {
                    vertex_range: table.vertex_ranges[row].clone(),
                    index_range: table.index_ranges[row].clone(),
                },
            })
            .collect(),
    };

    (model, table)
}

fn drawable_has_geometry(drawable: &Drawable) -> bool {
    !drawable.vertices.is_empty() && !drawable.indices.is_empty()
}

fn drawable_should_draw(drawable: &Drawable) -> bool {
    drawable.visible
        && drawable.opacity > DRAWABLE_OPACITY_EPSILON
        && drawable_has_geometry(drawable)
}

fn resident_drawable_indices(snapshot: &ModelSnapshot) -> Vec<usize> {
    let drawable_index_by_id = snapshot
        .drawables
        .iter()
        .enumerate()
        .filter(|(_, drawable)| drawable_has_geometry(drawable))
        .map(|(index, drawable)| (&drawable.id, index))
        .collect::<HashMap<_, _>>();
    let mut resident = HashSet::new();

    for (index, drawable) in snapshot.drawables.iter().enumerate() {
        if !drawable_should_draw(drawable) {
            continue;
        }
        resident.insert(index);
        if let Some(clipping) = drawable.clipping.as_ref() {
            for drawable_id in &clipping.drawable_ids {
                if let Some(mask_index) = drawable_index_by_id.get(drawable_id) {
                    resident.insert(*mask_index);
                }
            }
        }
    }

    (0..snapshot.drawables.len())
        .filter(|index| resident.contains(index))
        .collect()
}

fn build_drawable_table(snapshot: &ModelSnapshot) -> DrawableTable {
    let mut vertex_offset = 0;
    let mut index_offset = 0;
    let resident_indices = resident_drawable_indices(snapshot);
    let drawable_count = resident_indices.len();
    let mut table = DrawableTable {
        vertex_ranges: Vec::with_capacity(drawable_count),
        index_ranges: Vec::with_capacity(drawable_count),
        render_orders: Vec::with_capacity(drawable_count),
        source_indices: Vec::with_capacity(drawable_count),
    };

    for source_index in resident_indices {
        let drawable = &snapshot.drawables[source_index];
        let vertex_count = drawable.vertices.len() as u32;
        let index_count = drawable.indices.len() as u32;
        table
            .vertex_ranges
            .push(vertex_offset..vertex_offset + vertex_count);
        table
            .index_ranges
            .push(index_offset..index_offset + index_count);
        table.render_orders.push(drawable.render_order);
        table.source_indices.push(source_index);
        vertex_offset += vertex_count;
        index_offset += index_count;
    }

    table
}

fn sorted_drawable_rows(table: &DrawableTable) -> Vec<usize> {
    let mut rows = (0..table.len()).collect::<Vec<_>>();
    rows.sort_by_key(|row| table.render_orders[*row]);
    rows
}

fn build_render_world_cache(snapshot: &ModelSnapshot) -> RenderWorldCache {
    let (model, table) = build_model_ctx_and_table(snapshot);
    let ordered_rows = sorted_drawable_rows(&table);
    let mask_table = build_mask_group_table(snapshot, &table, &ordered_rows);
    let clipping_by_row = clipping_by_row(snapshot, &table);
    let draw_enabled_by_row = draw_enabled_by_row(snapshot, &table);
    RenderWorldCache {
        model,
        table,
        ordered_rows,
        mask_table,
        clipping_by_row,
        draw_enabled_by_row,
    }
}

fn render_world_cache_matches(cache: &RenderWorldCache, snapshot: &ModelSnapshot) -> bool {
    let resident_indices = resident_drawable_indices(snapshot);
    if resident_indices.len() != cache.table.len() {
        return false;
    }

    for row in 0..cache.table.len() {
        let source_index = cache.table.source_indices[row];
        if resident_indices[row] != source_index {
            return false;
        }
        let Some(drawable) = snapshot.drawables.get(source_index) else {
            return false;
        };
        let model_drawable = &cache.model.drawables[row];
        let vertex_count =
            cache.table.vertex_ranges[row].end - cache.table.vertex_ranges[row].start;
        let index_count = cache.table.index_ranges[row].end - cache.table.index_ranges[row].start;
        if drawable.vertices.is_empty()
            || drawable.indices.is_empty()
            || drawable.id != model_drawable.drawable_id
            || drawable.texture_index != model_drawable.texture_index
            || drawable.vertices.len() as u32 != vertex_count
            || drawable.indices.len() as u32 != index_count
            || drawable.clipping.as_ref() != cache.clipping_by_row[row].as_ref()
            || drawable_should_draw(drawable) != cache.draw_enabled_by_row[row]
        {
            return false;
        }
    }

    true
}

fn clipping_by_row(snapshot: &ModelSnapshot, table: &DrawableTable) -> Vec<Option<ClippingInfo>> {
    table
        .source_indices
        .iter()
        .map(|source_index| snapshot.drawables[*source_index].clipping.clone())
        .collect()
}

fn draw_enabled_by_row(snapshot: &ModelSnapshot, table: &DrawableTable) -> Vec<bool> {
    table
        .source_indices
        .iter()
        .map(|source_index| drawable_should_draw(&snapshot.drawables[*source_index]))
        .collect()
}

fn refresh_cached_render_order(snapshot: &ModelSnapshot, cache: &mut RenderWorldCache) -> bool {
    let mut changed = false;
    for row in 0..cache.table.len() {
        let render_order = snapshot.drawables[cache.table.source_indices[row]].render_order;
        if cache.table.render_orders[row] != render_order {
            cache.table.render_orders[row] = render_order;
            changed = true;
        }
    }
    if changed {
        cache.ordered_rows = sorted_drawable_rows(&cache.table);
    }
    changed
}

fn draw_commands_from_cache(
    snapshot: &ModelSnapshot,
    cache: &RenderWorldCache,
) -> Vec<DrawCommand> {
    draw_commands_for_visible_rows(
        snapshot,
        &cache.table,
        &cache.ordered_rows,
        &cache.mask_table,
    )
}

fn render_plan_from_cache(snapshot: &ModelSnapshot, cache: &RenderWorldCache) -> RenderPlan {
    let draws = draw_commands_from_cache(snapshot, cache);
    let commands = render_commands(snapshot, &draws);
    RenderPlan {
        model: cache.model.clone(),
        masks: cache.mask_table.masks.clone(),
        mask_draws: mask_draw_commands_from_table(snapshot, &cache.table, &cache.mask_table),
        draws,
        offscreens: snapshot.offscreens.clone(),
        commands,
    }
}

fn build_mask_group_table(
    snapshot: &ModelSnapshot,
    table: &DrawableTable,
    ordered_rows: &[usize],
) -> MaskGroupTable {
    let mut mask_refs = HashMap::new();
    let mut masks = Vec::new();
    let mut mask_refs_by_row = vec![None; table.len()];
    let resident_ids = table
        .source_indices
        .iter()
        .map(|source_index| &snapshot.drawables[*source_index].id)
        .collect::<HashSet<_>>();

    for &row in ordered_rows {
        let drawable = &snapshot.drawables[table.source_indices[row]];
        if !drawable_should_draw(drawable) {
            continue;
        }
        let Some(clipping) = drawable.clipping.as_ref() else {
            continue;
        };
        let drawable_ids = clipping
            .drawable_ids
            .iter()
            .filter(|drawable_id| resident_ids.contains(drawable_id))
            .cloned()
            .collect::<Vec<_>>();
        if drawable_ids.is_empty() {
            continue;
        }
        let mask_key = MaskKey {
            drawable_ids: drawable_ids.clone(),
            inverted: clipping.inverted,
        };
        let mask_ref = if let Some(mask_ref) = mask_refs.get(&mask_key) {
            *mask_ref
        } else {
            let mask_ref = MaskRef(masks.len());
            masks.push(MaskPass {
                id: mask_ref,
                drawable_ids: drawable_ids.clone(),
                inverted: clipping.inverted,
            });
            mask_refs.insert(mask_key, mask_ref);
            mask_ref
        };
        mask_refs_by_row[row] = Some(mask_ref);
    }

    MaskGroupTable {
        masks,
        mask_refs_by_row,
    }
}

fn draw_commands_for_visible_rows(
    snapshot: &ModelSnapshot,
    table: &DrawableTable,
    ordered_rows: &[usize],
    mask_table: &MaskGroupTable,
) -> Vec<DrawCommand> {
    ordered_rows
        .iter()
        .filter_map(|row| {
            let drawable = &snapshot.drawables[table.source_indices[*row]];
            drawable_should_draw(drawable).then(|| {
                let mask = mask_table.mask_refs_by_row[*row];
                draw_command_for_row(snapshot, table, *row, mask)
            })
        })
        .collect()
}

fn mask_draw_commands_from_table(
    snapshot: &ModelSnapshot,
    table: &DrawableTable,
    mask_table: &MaskGroupTable,
) -> Vec<DrawCommand> {
    let referenced_ids = mask_table
        .masks
        .iter()
        .flat_map(|mask| mask.drawable_ids.iter())
        .collect::<HashSet<_>>();

    (0..table.len())
        .filter_map(|row| {
            let drawable = &snapshot.drawables[table.source_indices[row]];
            referenced_ids.contains(&drawable.id).then(|| {
                draw_command_for_row(snapshot, table, row, mask_table.mask_refs_by_row[row])
            })
        })
        .collect()
}

fn draw_command_for_row(
    snapshot: &ModelSnapshot,
    table: &DrawableTable,
    row: usize,
    mask: Option<MaskRef>,
) -> DrawCommand {
    let drawable = &snapshot.drawables[table.source_indices[row]];
    DrawCommand {
        drawable_id: drawable.id.clone(),
        texture_index: drawable.texture_index,
        vertex_range: table.vertex_ranges[row].clone(),
        index_range: table.index_ranges[row].clone(),
        opacity: drawable.opacity,
        blend_mode: drawable.blend_mode,
        mask,
        inverted_mask: drawable
            .clipping
            .as_ref()
            .map(|clipping| clipping.inverted)
            .unwrap_or(false),
        material: MaterialKey::Default,
    }
}

fn render_commands(snapshot: &ModelSnapshot, draws: &[DrawCommand]) -> Vec<RenderCommand> {
    if snapshot.offscreens.is_empty() {
        return (0..draws.len())
            .map(|draw_index| RenderCommand::Draw { draw_index })
            .collect();
    }

    let draw_index_by_id = draws
        .iter()
        .enumerate()
        .map(|(index, draw)| (&draw.drawable_id, index))
        .collect::<HashMap<_, _>>();
    let drawable_parent_by_id = snapshot
        .drawables
        .iter()
        .map(|drawable| (&drawable.id, drawable.parent_part_index))
        .collect::<HashMap<_, _>>();
    let mut commands = Vec::new();
    let mut active_offscreens: Vec<usize> = Vec::new();
    let ordered_objects = if snapshot.render_objects.is_empty() {
        draws
            .iter()
            .map(|draw| RenderObject::Drawable(draw.drawable_id.clone()))
            .collect::<Vec<_>>()
    } else {
        snapshot.render_objects.clone()
    };

    for object in ordered_objects {
        match object {
            RenderObject::Drawable(drawable_id) => {
                let Some(draw_index) = draw_index_by_id.get(&drawable_id).copied() else {
                    continue;
                };
                let parent_part_index = drawable_parent_by_id.get(&drawable_id).copied().flatten();
                close_completed_offscreens(
                    snapshot,
                    parent_part_index,
                    &mut active_offscreens,
                    &mut commands,
                );
                commands.push(RenderCommand::Draw { draw_index });
            }
            RenderObject::Offscreen(offscreen_index) => {
                let Some(offscreen) = snapshot.offscreens.get(offscreen_index) else {
                    continue;
                };
                let parent_part_index = offscreen
                    .owner_part_index
                    .and_then(|owner| part_parent_index(snapshot, owner));
                close_completed_offscreens(
                    snapshot,
                    parent_part_index,
                    &mut active_offscreens,
                    &mut commands,
                );
                active_offscreens.push(offscreen_index);
                commands.push(RenderCommand::BeginOffscreen { offscreen_index });
            }
        }
    }

    while let Some(offscreen_index) = active_offscreens.pop() {
        commands.push(RenderCommand::CompositeOffscreen { offscreen_index });
    }
    commands
}

fn close_completed_offscreens(
    snapshot: &ModelSnapshot,
    target_parent_part_index: Option<usize>,
    active_offscreens: &mut Vec<usize>,
    commands: &mut Vec<RenderCommand>,
) {
    while let Some(offscreen_index) = active_offscreens.last().copied() {
        let owner_part_index = snapshot
            .offscreens
            .get(offscreen_index)
            .and_then(|offscreen| offscreen.owner_part_index);
        if part_is_descendant_of(snapshot, target_parent_part_index, owner_part_index) {
            break;
        }
        active_offscreens.pop();
        commands.push(RenderCommand::CompositeOffscreen { offscreen_index });
    }
}

fn part_parent_index(snapshot: &ModelSnapshot, part_index: usize) -> Option<usize> {
    snapshot
        .part_parent_indices
        .get(part_index)
        .copied()
        .flatten()
}

fn part_is_descendant_of(
    snapshot: &ModelSnapshot,
    mut part_index: Option<usize>,
    ancestor_index: Option<usize>,
) -> bool {
    let Some(ancestor_index) = ancestor_index else {
        return false;
    };
    while let Some(current) = part_index {
        if current == ancestor_index {
            return true;
        }
        part_index = part_parent_index(snapshot, current);
    }
    false
}

enum DrawLookup<'a> {
    Linear(&'a [DrawCommand]),
    Indexed(HashMap<&'a DrawableId, &'a DrawCommand>),
}

impl<'a> DrawLookup<'a> {
    fn new(draws: &'a [DrawCommand], mask_drawables: usize) -> Self {
        if mask_drawables.saturating_mul(draws.len()) <= DRAW_LOOKUP_INDEX_THRESHOLD {
            return Self::Linear(draws);
        }

        let mut lookup = HashMap::with_capacity(draws.len());
        for draw in draws {
            lookup.insert(&draw.drawable_id, draw);
        }
        Self::Indexed(lookup)
    }

    fn get(&self, drawable_id: &DrawableId) -> Option<&'a DrawCommand> {
        match self {
            Self::Linear(draws) => draws.iter().find(|draw| draw.drawable_id == *drawable_id),
            Self::Indexed(lookup) => lookup.get(drawable_id).copied(),
        }
    }
}

fn mask_drawable_count(masks: &[MaskPass]) -> usize {
    masks.iter().map(|mask| mask.drawable_ids.len()).sum()
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct MaskKey {
    drawable_ids: Vec<DrawableId>,
    inverted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use live2d_core::{
        CanvasInfo, ClippingInfo, Drawable, Offscreen, RenderObject, TextureAsset, Vertex,
    };

    #[test]
    fn builds_draws_in_render_order() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: vec![TextureAsset {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            }],
            drawables: vec![drawable("b", 20, None), drawable("a", 10, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);

        let ids = plan
            .draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn render_commands_wrap_drawables_in_owner_offscreen_order() {
        let body = drawable("body", 0, None);
        let mut arm = drawable("arm", 2, None);
        arm.parent_part_index = Some(0);
        let mut sleeve = drawable("sleeve", 3, None);
        sleeve.parent_part_index = Some(1);
        let outside = drawable("outside", 4, None);
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![body, arm, sleeve, outside],
            offscreens: vec![Offscreen {
                index: 0,
                render_order: 1,
                owner_part_index: Some(0),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            }],
            render_objects: vec![
                RenderObject::Drawable(DrawableId::from("body")),
                RenderObject::Offscreen(0),
                RenderObject::Drawable(DrawableId::from("arm")),
                RenderObject::Drawable(DrawableId::from("sleeve")),
                RenderObject::Drawable(DrawableId::from("outside")),
            ],
            part_parent_indices: vec![None, Some(0)],
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(
            plan.commands,
            vec![
                RenderCommand::Draw { draw_index: 0 },
                RenderCommand::BeginOffscreen { offscreen_index: 0 },
                RenderCommand::Draw { draw_index: 1 },
                RenderCommand::Draw { draw_index: 2 },
                RenderCommand::CompositeOffscreen { offscreen_index: 0 },
                RenderCommand::Draw { draw_index: 3 },
            ]
        );
    }

    #[test]
    fn allocates_mask_passes_for_clipped_drawables() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                drawable("mask", 0, None),
                drawable(
                    "masked",
                    1,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: true,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.masks.len(), 1);
        assert_eq!(plan.draws[1].mask, Some(MaskRef(0)));
        assert!(plan.masks[0].inverted);
        assert!(plan.draws[1].inverted_mask);
    }

    #[test]
    fn visible_translucent_drawable_stays_in_main_draws() {
        let mut snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("half", 0, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        snapshot.drawables[0].opacity = 0.35;

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.draws.len(), 1);
        assert_eq!(plan.draws[0].drawable_id.as_ref(), "half");
        assert_eq!(plan.draws[0].opacity, 0.35);
        assert_eq!(plan.model.drawables.len(), 1);
    }

    #[test]
    fn transparent_unreferenced_drawables_are_not_resident() {
        let mut hidden = drawable("hidden", 0, None);
        hidden.visible = false;
        let mut zero_alpha = drawable("zero", 1, None);
        zero_alpha.opacity = 0.0;
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![hidden, zero_alpha, drawable("visible", 2, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.model.drawables.len(), 1);
        assert_eq!(plan.model.drawables[0].drawable_id.as_ref(), "visible");
        assert!(plan.mask_draws.is_empty());
        assert_eq!(plan.draws.len(), 1);
        assert_eq!(plan.draws[0].drawable_id.as_ref(), "visible");
    }

    #[test]
    fn transparent_mask_source_is_resident_but_not_main_drawn() {
        let mut mask = drawable("mask", 0, None);
        mask.visible = false;
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                mask,
                drawable(
                    "masked",
                    1,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);
        let resident_ids = plan
            .model
            .drawables
            .iter()
            .map(|drawable| drawable.drawable_id.as_ref())
            .collect::<Vec<_>>();
        let draw_ids = plan
            .draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();
        let mask_draw_ids = plan
            .mask_draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(resident_ids, ["mask", "masked"]);
        assert_eq!(draw_ids, ["masked"]);
        assert_eq!(mask_draw_ids, ["mask"]);
        assert_eq!(plan.masks[0].drawable_ids, vec![DrawableId::from("mask")]);
    }

    #[test]
    fn shares_mask_passes_for_matching_clip_groups() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                drawable("mask", 0, None),
                drawable(
                    "masked-a",
                    1,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
                drawable(
                    "masked-b",
                    2,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.masks.len(), 1);
        assert_eq!(plan.draws[1].mask, Some(MaskRef(0)));
        assert_eq!(plan.draws[2].mask, Some(MaskRef(0)));
    }

    #[test]
    fn packs_drawable_ranges_once_and_draws_in_render_order() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                drawable_with_shape("b", 20, 2, 3),
                drawable_with_shape("a", 10, 3, 6),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.model.vertex_count, 5);
        assert_eq!(plan.model.index_count, 9);
        assert_eq!(plan.model.drawables[0].drawable_id.as_ref(), "b");
        assert_eq!(plan.model.drawables[0].ranges.vertex_range, 0..2);
        assert_eq!(plan.model.drawables[0].ranges.index_range, 0..3);
        assert_eq!(plan.model.drawables[1].drawable_id.as_ref(), "a");
        assert_eq!(plan.model.drawables[1].ranges.vertex_range, 2..5);
        assert_eq!(plan.model.drawables[1].ranges.index_range, 3..9);

        let draw_ids = plan
            .draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(draw_ids, ["a", "b"]);
        assert_eq!(plan.draws[0].vertex_range, 2..5);
        assert_eq!(plan.draws[0].index_range, 3..9);
        assert_eq!(plan.draws[1].vertex_range, 0..2);
        assert_eq!(plan.draws[1].index_range, 0..3);
    }

    #[test]
    fn render_world_matches_planner_and_refreshes_render_order() {
        let mut snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("b", 20, None), drawable("a", 10, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let mut world = RenderWorld::new();

        assert_eq!(
            world.build(&snapshot),
            RenderPlanner::new().build(&snapshot)
        );

        snapshot.drawables[0].render_order = 0;
        snapshot.drawables[1].render_order = 30;
        let plan = world.build(&snapshot);
        let draw_ids = plan
            .draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(draw_ids, ["b", "a"]);
    }

    #[test]
    fn render_world_refreshes_offscreen_commands_when_object_order_changes() {
        let mut arm = drawable("arm", 2, None);
        arm.parent_part_index = Some(0);
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("body", 0, None), arm, drawable("outside", 3, None)],
            offscreens: vec![Offscreen {
                index: 0,
                render_order: 1,
                owner_part_index: Some(0),
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            }],
            render_objects: vec![
                RenderObject::Drawable(DrawableId::from("body")),
                RenderObject::Offscreen(0),
                RenderObject::Drawable(DrawableId::from("arm")),
                RenderObject::Drawable(DrawableId::from("outside")),
            ],
            part_parent_indices: vec![None],
        };
        let mut world = RenderWorld::new();

        assert_eq!(
            world.build(&snapshot).commands,
            vec![
                RenderCommand::Draw { draw_index: 0 },
                RenderCommand::BeginOffscreen { offscreen_index: 0 },
                RenderCommand::Draw { draw_index: 1 },
                RenderCommand::CompositeOffscreen { offscreen_index: 0 },
                RenderCommand::Draw { draw_index: 2 },
            ]
        );

        let mut moved = snapshot;
        moved.render_objects = vec![
            RenderObject::Drawable(DrawableId::from("body")),
            RenderObject::Drawable(DrawableId::from("outside")),
            RenderObject::Offscreen(0),
            RenderObject::Drawable(DrawableId::from("arm")),
        ];

        assert_eq!(
            world.build(&moved).commands,
            vec![
                RenderCommand::Draw { draw_index: 0 },
                RenderCommand::Draw { draw_index: 2 },
                RenderCommand::BeginOffscreen { offscreen_index: 0 },
                RenderCommand::Draw { draw_index: 1 },
                RenderCommand::CompositeOffscreen { offscreen_index: 0 },
            ]
        );
    }

    #[test]
    fn render_world_rebuilds_when_opacity_crosses_draw_threshold() {
        let mut snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("fading", 0, None), drawable("visible", 1, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let mut world = RenderWorld::new();

        assert_eq!(world.build(&snapshot).model.drawables.len(), 2);

        snapshot.drawables[0].opacity = 0.0;
        let plan = world.build(&snapshot);

        assert_eq!(plan, RenderPlanner::new().build(&snapshot));
        assert_eq!(plan.model.drawables.len(), 1);
        assert_eq!(plan.draws[0].drawable_id.as_ref(), "visible");
    }

    #[test]
    fn render_world_rebuilds_when_resident_mask_source_becomes_drawable() {
        let mut mask_source = drawable(
            "mask-source",
            1,
            Some(ClippingInfo {
                drawable_ids: vec![DrawableId::from("mask")],
                inverted: false,
            }),
        );
        mask_source.opacity = 0.0;
        let snapshot_masked = drawable(
            "masked",
            2,
            Some(ClippingInfo {
                drawable_ids: vec![DrawableId::from("mask"), DrawableId::from("mask-source")],
                inverted: false,
            }),
        );
        let mut mask = drawable("mask", 0, None);
        mask.visible = false;
        let mut snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![mask, mask_source, snapshot_masked],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let mut world = RenderWorld::new();

        assert_eq!(world.build(&snapshot).draws.len(), 1);

        snapshot.drawables[1].opacity = 1.0;
        let plan = world.build(&snapshot);

        assert_eq!(plan, RenderPlanner::new().build(&snapshot));
        assert_eq!(plan.draws.len(), 2);
        assert_eq!(plan.masks.len(), 2);
    }

    #[test]
    fn render_world_retains_multiple_model_caches() {
        let mut model_a = ModelSnapshot {
            model_key: "model-a".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("a", 0, None), drawable("b", 1, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let model_b = ModelSnapshot {
            model_key: "model-b".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable("a", 0, None), drawable("b", 1, None)],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let mut world = RenderWorld::new();

        assert_eq!(world.build(&model_a), RenderPlanner::new().build(&model_a));
        assert_eq!(world.build(&model_b), RenderPlanner::new().build(&model_b));
        assert_eq!(world.caches.len(), 2);

        model_a.drawables[0].render_order = 2;
        model_a.drawables[1].render_order = 1;
        let plan = world.build(&model_a);
        let draw_ids = plan
            .draws
            .iter()
            .map(|draw| draw.drawable_id.as_ref())
            .collect::<Vec<_>>();

        assert_eq!(world.caches.len(), 2);
        assert_eq!(draw_ids, ["b", "a"]);
    }

    #[test]
    fn dispatches_clip_masks_before_main_draws() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                drawable("mask", 0, None),
                drawable(
                    "masked",
                    1,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let plan = RenderPlanner::new().build(&snapshot);
        let mut backend = RecordingBackend::default();

        plan.dispatch(&mut backend);

        assert_eq!(
            backend.events,
            vec![
                Event::BeginModel {
                    vertices: 2,
                    indices: 2,
                },
                Event::BeginClipMasks(1),
                Event::BeginClipMask {
                    id: MaskRef(0),
                    inverted: false,
                },
                Event::MaskDrawable {
                    mask: MaskRef(0),
                    drawable: DrawableId::from("mask"),
                },
                Event::EndClipMask(MaskRef(0)),
                Event::BeginMainPass,
                Event::Draw(DrawableId::from("mask")),
                Event::Draw(DrawableId::from("masked")),
                Event::EndModel,
            ]
        );
    }

    #[test]
    fn post_process_plan_accepts_empty_chain() {
        let plan = PostProcessPlan::new(Vec::new()).unwrap();

        assert!(plan.is_empty());
        assert!(PostProcessPlan::empty().is_empty());
    }

    #[test]
    fn post_process_plan_builds_linear_chain() {
        let plan = PostProcessPlan::linear(["tone", "blur", "composite"]);

        assert_eq!(plan.passes().len(), 3);
        assert_eq!(plan.passes()[0].input, PostProcessInput::Scene);
        assert_eq!(plan.passes()[0].output, PostProcessOutput::Temporary);
        assert_eq!(plan.passes()[1].input, PostProcessInput::Pass(0));
        assert_eq!(plan.passes()[1].output, PostProcessOutput::Temporary);
        assert_eq!(plan.passes()[2].input, PostProcessInput::Pass(1));
        assert_eq!(plan.passes()[2].output, PostProcessOutput::Final);
    }

    #[test]
    fn post_process_plan_rejects_non_linear_input() {
        let result = PostProcessPlan::new(vec![
            PostProcessPass::new("a", PostProcessInput::Scene, PostProcessOutput::Temporary),
            PostProcessPass::new("b", PostProcessInput::Scene, PostProcessOutput::Final),
        ]);

        assert_eq!(
            result,
            Err(PostProcessPlanError::UnsupportedInput {
                pass: 1,
                input: PostProcessInput::Scene,
            })
        );
    }

    #[test]
    fn post_process_plan_rejects_missing_final_output() {
        let result = PostProcessPlan::new(vec![PostProcessPass::new(
            "tone",
            PostProcessInput::Scene,
            PostProcessOutput::Temporary,
        )]);

        assert_eq!(result, Err(PostProcessPlanError::MissingFinalOutput));
    }

    #[test]
    fn post_process_plan_preserves_pass_params() {
        let mut params = PostProcessParams::default();
        params.values[2] = [1.0, 2.0, 3.0, 4.0];
        let plan = PostProcessPlan::new(vec![PostProcessPass::new(
            "tone",
            PostProcessInput::Scene,
            PostProcessOutput::Final,
        )
        .with_params(params)])
        .unwrap();

        assert_eq!(plan.passes()[0].params.values[2], [1.0, 2.0, 3.0, 4.0]);
    }

    #[cfg(feature = "probe")]
    #[test]
    fn build_with_probe_matches_regular_build_and_records_stages() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: vec![TextureAsset {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            }],
            drawables: vec![
                drawable("mask", 0, None),
                drawable(
                    "face",
                    10,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let planner = RenderPlanner::new();
        let recorder = live2d_probe::ProbeRecorder::new();

        let expected = planner.build(&snapshot);
        let actual = planner.build_with_probe(&snapshot, &recorder);
        let analysis = live2d_probe::ProbeAnalysis::from_data(&recorder.data());

        assert_eq!(actual, expected);
        assert!(analysis
            .stages
            .contains_key(&live2d_probe::Stage::RenderPlanTotal));
        assert_eq!(
            analysis
                .stages
                .get(&live2d_probe::Stage::RenderPlanTotal)
                .unwrap()
                .draw_calls,
            expected.draws.len() as u64
        );
    }

    #[cfg(feature = "probe")]
    #[test]
    fn dispatch_with_probe_preserves_backend_events_and_counts_draws() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![
                drawable("mask", 0, None),
                drawable(
                    "masked",
                    1,
                    Some(ClippingInfo {
                        drawable_ids: vec![DrawableId::from("mask")],
                        inverted: false,
                    }),
                ),
            ],
            offscreens: Vec::new(),
            render_objects: Vec::new(),
            part_parent_indices: Vec::new(),
        };
        let plan = RenderPlanner::new().build(&snapshot);
        let recorder = live2d_probe::ProbeRecorder::new();
        let mut backend = RecordingBackend::default();

        plan.dispatch_with_probe(&mut backend, &recorder);
        let analysis = live2d_probe::ProbeAnalysis::from_data(&recorder.data());

        assert_eq!(
            backend.events,
            vec![
                Event::BeginModel {
                    vertices: 2,
                    indices: 2,
                },
                Event::BeginClipMasks(1),
                Event::BeginClipMask {
                    id: MaskRef(0),
                    inverted: false,
                },
                Event::MaskDrawable {
                    mask: MaskRef(0),
                    drawable: DrawableId::from("mask"),
                },
                Event::EndClipMask(MaskRef(0)),
                Event::BeginMainPass,
                Event::Draw(DrawableId::from("mask")),
                Event::Draw(DrawableId::from("masked")),
                Event::EndModel,
            ]
        );
        assert_eq!(
            analysis
                .stages
                .get(&live2d_probe::Stage::RenderMainDrawDispatch)
                .unwrap()
                .draw_calls,
            2
        );
        assert_eq!(
            analysis
                .stages
                .get(&live2d_probe::Stage::RenderMaskLookup)
                .unwrap()
                .draw_calls,
            1
        );
    }

    #[derive(Default)]
    struct RecordingBackend {
        events: Vec<Event>,
    }

    impl Live2DRenderBackend for RecordingBackend {
        fn begin_model(&mut self, ctx: &ModelRenderCtx) {
            self.events.push(Event::BeginModel {
                vertices: ctx.vertex_count,
                indices: ctx.index_count,
            });
        }

        fn begin_clip_masks(&mut self, masks: &[MaskPass]) {
            self.events.push(Event::BeginClipMasks(masks.len()));
        }

        fn begin_clip_mask(&mut self, mask: &MaskPass) {
            self.events.push(Event::BeginClipMask {
                id: mask.id,
                inverted: mask.inverted,
            });
        }

        fn draw_mask_drawable(&mut self, mask: &MaskPass, call: &DrawCommand) {
            self.events.push(Event::MaskDrawable {
                mask: mask.id,
                drawable: call.drawable_id.clone(),
            });
        }

        fn end_clip_mask(&mut self, mask: &MaskPass) {
            self.events.push(Event::EndClipMask(mask.id));
        }

        fn begin_main_pass(&mut self) {
            self.events.push(Event::BeginMainPass);
        }

        fn draw_drawable(&mut self, call: &DrawCommand) {
            self.events.push(Event::Draw(call.drawable_id.clone()));
        }

        fn end_model(&mut self) {
            self.events.push(Event::EndModel);
        }
    }

    #[derive(Debug, PartialEq)]
    enum Event {
        BeginModel { vertices: u32, indices: u32 },
        BeginClipMasks(usize),
        BeginClipMask { id: MaskRef, inverted: bool },
        MaskDrawable { mask: MaskRef, drawable: DrawableId },
        EndClipMask(MaskRef),
        BeginMainPass,
        Draw(DrawableId),
        EndModel,
    }

    fn drawable(id: &str, render_order: i32, clipping: Option<ClippingInfo>) -> Drawable {
        Drawable {
            id: DrawableId::from(id),
            render_order,
            parent_part_index: None,
            texture_index: 0,
            vertices: vec![Vertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
            }],
            indices: vec![0],
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping,
        }
    }

    fn drawable_with_shape(
        id: &str,
        render_order: i32,
        vertex_count: usize,
        index_count: usize,
    ) -> Drawable {
        Drawable {
            id: DrawableId::from(id),
            render_order,
            parent_part_index: None,
            texture_index: 0,
            vertices: (0..vertex_count)
                .map(|index| Vertex {
                    position: [index as f32, 0.0],
                    uv: [0.0, 0.0],
                })
                .collect(),
            indices: (0..index_count).map(|index| index as u16).collect(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping: None,
        }
    }
}
