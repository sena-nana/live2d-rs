use std::os::raw::c_void;
#[cfg(feature = "cubism-core")]
use std::os::raw::{c_char, c_int, c_uchar, c_uint};

#[repr(C)]
pub struct CsmMoc(c_void);

#[repr(C)]
pub struct CsmModel(c_void);

#[cfg(feature = "cubism-core")]
#[allow(non_camel_case_types)]
pub type csmFlags = c_uchar;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmBlendAdditive: csmFlags = 1 << 0;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmBlendMultiplicative: csmFlags = 1 << 1;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Normal: c_int = 0;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_AddCompatible: c_int = 1;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_MultiplyCompatible: c_int = 2;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Add: c_int = 3;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_AddGlow: c_int = 4;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Darken: c_int = 5;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Multiply: c_int = 6;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_ColorBurn: c_int = 7;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_LinearBurn: c_int = 8;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Lighten: c_int = 9;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Screen: c_int = 10;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_ColorDodge: c_int = 11;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Overlay: c_int = 12;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_SoftLight: c_int = 13;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_HardLight: c_int = 14;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_LinearLight: c_int = 15;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Hue: c_int = 16;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmColorBlendType_Color: c_int = 17;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmAlphaBlendType_Over: c_int = 0;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmAlphaBlendType_Atop: c_int = 1;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmAlphaBlendType_Out: c_int = 2;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmAlphaBlendType_ConjointOver: c_int = 3;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmAlphaBlendType_DisjointOver: c_int = 4;

#[cfg(feature = "cubism-core")]
#[allow(non_upper_case_globals)]
pub const csmIsVisible: csmFlags = 1 << 0;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CsmVector2 {
    pub x: f32,
    pub y: f32,
}

#[cfg(feature = "cubism-core")]
extern "C" {
    pub fn csmReviveMocInPlace(address: *mut c_void, size: c_uint) -> *mut CsmMoc;
    pub fn csmGetSizeofModel(moc: *mut CsmMoc) -> c_uint;
    pub fn csmInitializeModelInPlace(
        moc: *mut CsmMoc,
        address: *mut c_void,
        size: c_uint,
    ) -> *mut CsmModel;
    pub fn csmUpdateModel(model: *mut CsmModel);
    pub fn csmGetParameterCount(model: *const CsmModel) -> c_int;
    pub fn csmGetParameterIds(model: *const CsmModel) -> *const *const c_char;
    pub fn csmGetParameterMinimumValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterMaximumValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterDefaultValues(model: *const CsmModel) -> *const f32;
    pub fn csmGetParameterValues(model: *mut CsmModel) -> *mut f32;
    pub fn csmReadCanvasInfo(
        model: *const CsmModel,
        out_size: *mut CsmVector2,
        out_origin: *mut CsmVector2,
        out_pixels_per_unit: *mut f32,
    );
    pub fn csmGetDrawableCount(model: *const CsmModel) -> c_int;
    pub fn csmGetDrawableIds(model: *const CsmModel) -> *const *const c_char;
    pub fn csmGetDrawableConstantFlags(model: *const CsmModel) -> *const csmFlags;
    pub fn csmGetDrawableDynamicFlags(model: *const CsmModel) -> *const csmFlags;
    pub fn csmGetRenderOrders(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableBlendModes(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableTextureIndices(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableOpacities(model: *const CsmModel) -> *const f32;
    pub fn csmGetDrawableMaskCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableMasks(model: *const CsmModel) -> *const *const c_int;
    pub fn csmGetDrawableVertexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableVertexPositions(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableVertexUvs(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableIndexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableIndices(model: *const CsmModel) -> *const *const u16;
}
