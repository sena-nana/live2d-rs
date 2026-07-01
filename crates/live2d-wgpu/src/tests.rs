use super::*;
use live2d_core::{
    BlendMode, CanvasInfo, ClippingInfo, DrawableId, MaterialKey, TextureAsset, Vertex,
};

pub(crate) fn snapshot_with_drawable(
    id: &str,
    render_order: i32,
    vertex_count: usize,
    index_count: usize,
    texture_index: usize,
    texture_count: usize,
) -> ModelSnapshot {
    ModelSnapshot {
        model_key: "sample".into(),
        canvas: CanvasInfo::default(),
        art_meshes: Vec::new(),
        drawables: vec![Drawable {
            id: DrawableId::from(id),
            render_order,
            texture_index,
            vertices: (0..vertex_count)
                .map(|index| Vertex {
                    position: [index as f32, index as f32 + 1.0],
                    uv: [0.0, 0.0],
                })
                .collect(),
            indices: (0..index_count).map(|index| index as u16).collect(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            clipping: None,
        }],
        textures: (0..texture_count)
            .map(|_| TextureAsset {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 255],
            })
            .collect(),
    }
}

pub(crate) fn snapshot_with_drawables(drawables: &[(&str, i32, usize, usize)]) -> ModelSnapshot {
    ModelSnapshot {
        model_key: "sample".into(),
        canvas: CanvasInfo::default(),
        art_meshes: Vec::new(),
        drawables: drawables
            .iter()
            .map(|(id, render_order, vertex_count, index_count)| Drawable {
                id: DrawableId::from(*id),
                render_order: *render_order,
                texture_index: 0,
                vertices: (0..*vertex_count)
                    .map(|index| Vertex {
                        position: [index as f32, index as f32 + 1.0],
                        uv: [0.0, 0.0],
                    })
                    .collect(),
                indices: (0..*index_count).map(|index| index as u16).collect(),
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            })
            .collect(),
        textures: vec![TextureAsset {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        }],
    }
}

pub(crate) fn masked_snapshot() -> ModelSnapshot {
    ModelSnapshot {
        model_key: "sample".into(),
        canvas: CanvasInfo::default(),
        art_meshes: Vec::new(),
        drawables: vec![
            Drawable {
                id: DrawableId::from("mask"),
                render_order: 0,
                texture_index: 0,
                vertices: vec![Vertex {
                    position: [0.0, 0.0],
                    uv: [0.0, 0.0],
                }],
                indices: vec![0],
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: None,
            },
            Drawable {
                id: DrawableId::from("masked"),
                render_order: 1,
                texture_index: 0,
                vertices: vec![Vertex {
                    position: [1.0, 1.0],
                    uv: [0.0, 0.0],
                }],
                indices: vec![0],
                visible: true,
                opacity: 1.0,
                blend_mode: BlendMode::Normal,
                clipping: Some(ClippingInfo {
                    drawable_ids: vec![DrawableId::from("mask")],
                    inverted: false,
                }),
            },
        ],
        textures: vec![TextureAsset {
            width: 1,
            height: 1,
            rgba: vec![255, 255, 255, 255],
        }],
    }
}

pub(crate) fn draw_command(id: &str) -> DrawCommand {
    DrawCommand {
        drawable_id: DrawableId::from(id),
        texture_index: 0,
        vertex_range: 0..3,
        index_range: 0..3,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        mask: None,
        inverted_mask: false,
        material: MaterialKey::Default,
    }
}

pub(crate) fn assert_blend_component(
    component: wgpu::BlendComponent,
    src_factor: wgpu::BlendFactor,
    dst_factor: wgpu::BlendFactor,
) {
    assert_eq!(component.src_factor, src_factor);
    assert_eq!(component.dst_factor, dst_factor);
    assert_eq!(component.operation, wgpu::BlendOperation::Add);
}
