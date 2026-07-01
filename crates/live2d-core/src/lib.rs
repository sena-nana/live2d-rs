use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawableId(pub String);

impl From<String> for DrawableId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for DrawableId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl AsRef<str> for DrawableId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BlendMode {
    #[default]
    Normal,
    Additive,
    Multiplicative,
    Advanced {
        color: ColorBlendMode,
        alpha: AlphaBlendMode,
    },
}

impl BlendMode {
    pub fn from_cubism_blend_mode(value: i32) -> Self {
        let color = value & 0xff;
        let alpha = (value >> 8) & 0xff;
        match (color, alpha) {
            (0, 0) => Self::Normal,
            (1, _) => Self::Additive,
            (2, _) => Self::Multiplicative,
            _ => Self::Advanced {
                color: ColorBlendMode::from_cubism_color_blend_type(color),
                alpha: AlphaBlendMode::from_cubism_alpha_blend_type(alpha),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ColorBlendMode {
    #[default]
    Normal,
    Add,
    AddGlow,
    Darken,
    Multiply,
    ColorBurn,
    LinearBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    LinearLight,
    Hue,
    Color,
}

impl ColorBlendMode {
    pub fn from_cubism_color_blend_type(value: i32) -> Self {
        match value {
            3 => Self::Add,
            4 => Self::AddGlow,
            5 => Self::Darken,
            6 => Self::Multiply,
            7 => Self::ColorBurn,
            8 => Self::LinearBurn,
            9 => Self::Lighten,
            10 => Self::Screen,
            11 => Self::ColorDodge,
            12 => Self::Overlay,
            13 => Self::SoftLight,
            14 => Self::HardLight,
            15 => Self::LinearLight,
            16 => Self::Hue,
            17 => Self::Color,
            _ => Self::Normal,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AlphaBlendMode {
    #[default]
    Over,
    Atop,
    Out,
    ConjointOver,
    DisjointOver,
}

impl AlphaBlendMode {
    pub fn from_cubism_alpha_blend_type(value: i32) -> Self {
        match value {
            1 => Self::Atop,
            2 => Self::Out,
            3 => Self::ConjointOver,
            4 => Self::DisjointOver,
            _ => Self::Over,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CanvasInfo {
    pub size: [f32; 2],
    pub origin: [f32; 2],
    pub pixels_per_unit: f32,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TextureAsset {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Vertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ClippingInfo {
    pub drawable_ids: Vec<DrawableId>,
    pub inverted: bool,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Drawable {
    pub id: DrawableId,
    pub render_order: i32,
    pub parent_part_index: Option<usize>,
    pub texture_index: usize,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub visible: bool,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub clipping: Option<ClippingInfo>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Offscreen {
    pub index: usize,
    pub render_order: i32,
    pub owner_part_index: Option<usize>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub clipping: Option<ClippingInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RenderObject {
    Drawable(DrawableId),
    Offscreen(usize),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ArtMeshInfo {
    pub id: DrawableId,
    pub label: String,
    pub original_name: String,
    pub index: usize,
    pub mask_type: String,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ModelSnapshot {
    pub model_key: String,
    pub canvas: CanvasInfo,
    pub art_meshes: Vec<ArtMeshInfo>,
    pub drawables: Vec<Drawable>,
    pub offscreens: Vec<Offscreen>,
    pub render_objects: Vec<RenderObject>,
    pub part_parent_indices: Vec<Option<usize>>,
    pub textures: Vec<TextureAsset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaterialKey {
    #[default]
    Default,
    Custom(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaskRef(pub usize);

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DrawableRanges {
    pub vertex_range: Range<u32>,
    pub index_range: Range<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cubism_compatible_blend_modes_map_to_legacy_modes() {
        assert_eq!(BlendMode::from_cubism_blend_mode(0), BlendMode::Normal);
        assert_eq!(BlendMode::from_cubism_blend_mode(1), BlendMode::Additive);
        assert_eq!(
            BlendMode::from_cubism_blend_mode(2),
            BlendMode::Multiplicative
        );
    }

    #[test]
    fn cubism_advanced_blend_mode_splits_color_and_alpha() {
        assert_eq!(
            BlendMode::from_cubism_blend_mode(6 | (3 << 8)),
            BlendMode::Advanced {
                color: ColorBlendMode::Multiply,
                alpha: AlphaBlendMode::ConjointOver,
            }
        );
    }
}
