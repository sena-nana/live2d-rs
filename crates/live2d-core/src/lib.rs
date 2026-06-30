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
    pub texture_index: usize,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub clipping: Option<ClippingInfo>,
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
