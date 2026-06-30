use live2d_core::{BlendMode, DrawableId, MaskRef, MaterialKey, ModelSnapshot};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq)]
pub struct RenderPlan {
    pub masks: Vec<MaskPass>,
    pub draws: Vec<DrawCommand>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskPass {
    pub id: MaskRef,
    pub drawable_ids: Vec<DrawableId>,
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

impl RenderPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn build(&self, snapshot: &ModelSnapshot) -> RenderPlan {
        let mut drawables = snapshot.drawables.iter().collect::<Vec<_>>();
        drawables.sort_by_key(|drawable| drawable.render_order);

        let mut masks = Vec::new();
        let draws = drawables
            .into_iter()
            .map(|drawable| {
                let mask = drawable.clipping.as_ref().map(|clipping| {
                    let mask_ref = MaskRef(masks.len());
                    masks.push(MaskPass {
                        id: mask_ref,
                        drawable_ids: clipping.drawable_ids.clone(),
                    });
                    mask_ref
                });
                DrawCommand {
                    drawable_id: drawable.id.clone(),
                    texture_index: drawable.texture_index,
                    vertex_range: 0..drawable.vertices.len() as u32,
                    index_range: 0..drawable.indices.len() as u32,
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
            })
            .collect();

        RenderPlan { masks, draws }
    }
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
        assert!(plan.draws[0].inverted_mask);
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
}
