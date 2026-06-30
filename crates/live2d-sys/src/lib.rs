use std::os::raw::c_void;
#[cfg(feature = "cubism-core")]
use std::os::raw::{c_char, c_int, c_uint};

#[repr(C)]
pub struct CsmMoc(c_void);

#[repr(C)]
pub struct CsmModel(c_void);

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
    pub fn csmGetRenderOrders(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableTextureIndices(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableVertexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableVertexPositions(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableVertexUvs(model: *const CsmModel) -> *const *const CsmVector2;
    pub fn csmGetDrawableIndexCounts(model: *const CsmModel) -> *const c_int;
    pub fn csmGetDrawableIndices(model: *const CsmModel) -> *const *const u16;
}
