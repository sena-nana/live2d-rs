use live2d_core::{
    BlendMode, ClippingInfo, DrawableId, DrawableRanges, MaskRef, MaterialKey, ModelSnapshot,
};
#[cfg(feature = "probe")]
use live2d_probe::{counter, measure, ProbeAttr, ProbeSink, Stage};
use std::collections::HashMap;
use std::ops::Range;

const DRAW_LOOKUP_INDEX_THRESHOLD: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub model: ModelRenderCtx,
    pub masks: Vec<MaskPass>,
    pub draws: Vec<DrawCommand>,
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

#[derive(Debug, Clone, Default)]
pub struct RenderPlanner;

#[derive(Debug, Clone, Default)]
pub struct RenderWorld {
    cache: Option<RenderWorldCache>,
}

#[derive(Debug, Clone)]
struct RenderWorldCache {
    model_key: String,
    model: ModelRenderCtx,
    table: DrawableTable,
    ordered_rows: Vec<usize>,
    mask_table: MaskGroupTable,
    clipping_by_row: Vec<Option<ClippingInfo>>,
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
            let draw_lookup = DrawLookup::new(&self.draws, mask_drawable_count(&self.masks));
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
                        DrawLookup::new(&self.draws, mask_drawable_count(&self.masks));
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

        let draws = ordered_rows
            .into_iter()
            .map(|row| {
                let mask = mask_table.mask_refs_by_row[row];
                draw_command_for_row(snapshot, &table, row, mask)
            })
            .collect();

        RenderPlan {
            model,
            masks: mask_table.masks,
            draws,
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
                let draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new("candidate_drawables", ordered_rows.len())],
                    || {
                        ordered_rows
                            .into_iter()
                            .map(|row| {
                                let mask = mask_table.mask_refs_by_row[row];
                                draw_command_for_row(snapshot, &table, row, mask)
                            })
                            .collect::<Vec<_>>()
                    },
                );

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
                    draws,
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
        self.cache = None;
    }

    pub fn build(&mut self, snapshot: &ModelSnapshot) -> RenderPlan {
        self.ensure_cache(snapshot);
        let cache = self
            .cache
            .as_mut()
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
                    .cache
                    .as_ref()
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
                    self.cache = Some(RenderWorldCache {
                        model_key: snapshot.model_key.clone(),
                        clipping_by_row: clipping_by_row(snapshot, &table),
                        model,
                        table,
                        ordered_rows,
                        mask_table,
                    });
                }

                let cache = self
                    .cache
                    .as_mut()
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
                let draws = measure(
                    probe,
                    Stage::RenderDrawCommandBuild,
                    vec![ProbeAttr::new(
                        "candidate_drawables",
                        cache.ordered_rows.len(),
                    )],
                    || draw_commands_from_cache(snapshot, cache),
                );
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
                    draws,
                }
            },
        )
    }

    fn ensure_cache(&mut self, snapshot: &ModelSnapshot) {
        let rebuild = self
            .cache
            .as_ref()
            .map_or(true, |cache| !render_world_cache_matches(cache, snapshot));
        if rebuild {
            self.cache = Some(build_render_world_cache(snapshot));
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

fn build_drawable_table(snapshot: &ModelSnapshot) -> DrawableTable {
    let mut vertex_offset = 0;
    let mut index_offset = 0;
    let drawable_count = snapshot.drawables.len();
    let mut table = DrawableTable {
        vertex_ranges: Vec::with_capacity(drawable_count),
        index_ranges: Vec::with_capacity(drawable_count),
        render_orders: Vec::with_capacity(drawable_count),
        source_indices: Vec::with_capacity(drawable_count),
    };

    for (source_index, drawable) in snapshot.drawables.iter().enumerate() {
        if drawable.vertices.is_empty() || drawable.indices.is_empty() {
            continue;
        }

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
    RenderWorldCache {
        model_key: snapshot.model_key.clone(),
        model,
        table,
        ordered_rows,
        mask_table,
        clipping_by_row,
    }
}

fn render_world_cache_matches(cache: &RenderWorldCache, snapshot: &ModelSnapshot) -> bool {
    if cache.model_key != snapshot.model_key {
        return false;
    }
    let mut renderable_count = 0;
    for drawable in &snapshot.drawables {
        if !drawable.vertices.is_empty() && !drawable.indices.is_empty() {
            renderable_count += 1;
        }
    }
    if renderable_count != cache.table.len() {
        return false;
    }

    for row in 0..cache.table.len() {
        let source_index = cache.table.source_indices[row];
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
    cache
        .ordered_rows
        .iter()
        .map(|row| {
            let mask = cache.mask_table.mask_refs_by_row[*row];
            draw_command_for_row(snapshot, &cache.table, *row, mask)
        })
        .collect()
}

fn render_plan_from_cache(snapshot: &ModelSnapshot, cache: &RenderWorldCache) -> RenderPlan {
    RenderPlan {
        model: cache.model.clone(),
        masks: cache.mask_table.masks.clone(),
        draws: draw_commands_from_cache(snapshot, cache),
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

    for &row in ordered_rows {
        let drawable = &snapshot.drawables[table.source_indices[row]];
        let Some(clipping) = drawable.clipping.as_ref() else {
            continue;
        };
        let mask_key = MaskKey {
            drawable_ids: &clipping.drawable_ids,
            inverted: clipping.inverted,
        };
        let mask_ref = if let Some(mask_ref) = mask_refs.get(&mask_key) {
            *mask_ref
        } else {
            let mask_ref = MaskRef(masks.len());
            masks.push(MaskPass {
                id: mask_ref,
                drawable_ids: clipping.drawable_ids.clone(),
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct MaskKey<'a> {
    drawable_ids: &'a [DrawableId],
    inverted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use live2d_core::{CanvasInfo, ClippingInfo, Drawable, TextureAsset, Vertex};

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
    fn allocates_mask_passes_for_clipped_drawables() {
        let snapshot = ModelSnapshot {
            model_key: "sample".into(),
            canvas: CanvasInfo::default(),
            art_meshes: Vec::new(),
            textures: Vec::new(),
            drawables: vec![drawable(
                "masked",
                0,
                Some(ClippingInfo {
                    drawable_ids: vec![DrawableId::from("mask")],
                    inverted: true,
                }),
            )],
        };

        let plan = RenderPlanner::new().build(&snapshot);

        assert_eq!(plan.masks.len(), 1);
        assert_eq!(plan.draws[0].mask, Some(MaskRef(0)));
        assert!(plan.masks[0].inverted);
        assert!(plan.draws[0].inverted_mask);
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
            texture_index: 0,
            vertices: vec![Vertex {
                position: [0.0, 0.0],
                uv: [0.0, 0.0],
            }],
            indices: vec![0],
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
            texture_index: 0,
            vertices: (0..vertex_count)
                .map(|index| Vertex {
                    position: [index as f32, 0.0],
                    uv: [0.0, 0.0],
                })
                .collect(),
            indices: (0..index_count).map(|index| index as u16).collect(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping: None,
        }
    }
}
