//! Linear memory layout shared by the WASM module and the host.
//!
//! Initial memory uses two 64 KiB pages (128 KiB). The **lower half** is reserved
//! for guest data/stack/conventions; the **upper half** starting at [`HEAP_BASE`]
//! is the only region the host bump allocator uses for [`crate::wasm::host_heap`].

use crate::vm::symbols::{ContextType, DefineType};

/// Size of one WASM page (64 KiB).
pub const WASM_PAGE_SIZE: u32 = 65536;

/// Minimum memory size (pages). Two pages ⇒ 128 KiB total; [`HEAP_BASE`] is the midpoint.
pub const MEMORY_MIN_PAGES: u64 = 2;

/// Byte offset where the host-managed heap starts (upper half of initial memory).
pub const HEAP_BASE: i32 = (MEMORY_MIN_PAGES as i32 / 2) * (WASM_PAGE_SIZE as i32);

/// Byte size of a single field in a heap-allocated struct.
pub fn field_byte_size(dt: &DefineType) -> u32 {
    match dt.unwrap_qualifiers() {
        DefineType::Int64 | DefineType::Uint64 => 8,
        DefineType::Float64 => 8,
        DefineType::String => 8, // ptr(4) + len(4)
        _ => 4,
    }
}

pub const SLICE_HEADER_SIZE: u32 = 12;
pub const SLICE_DATA_PTR_OFFSET: u32 = 0;
pub const SLICE_LEN_OFFSET: u32 = 4;
pub const SLICE_CAP_OFFSET: u32 = 8;

pub fn elem_byte_size(dt: &DefineType) -> u32 {
    field_byte_size(dt)
}

/// Top of the stack region (last usable byte in the lower half).
/// `$sp` starts here and grows downward.
pub const STACK_TOP: u32 = 65535;

/// Total byte size of a fixed-size array in linear memory.
pub fn array_byte_size(elem_dt: &DefineType, len: usize) -> u32 {
    elem_byte_size(elem_dt) * len as u32
}

/// Computes `(field_name, byte_offset, field_type)` for each field and the total struct size.
pub fn struct_field_layout(fields: &[ContextType]) -> (Vec<(String, u32, DefineType)>, u32) {
    let mut out = Vec::with_capacity(fields.len());
    let mut offset: u32 = 0;
    for ct in fields {
        let (name, dt) = match ct {
            ContextType::Named(n, d) => (n.clone(), d.clone()),
            ContextType::Embedded(n, d) => (n.clone(), d.clone()),
            ContextType::Unnamed(d) => (String::new(), d.clone()),
        };
        let size = field_byte_size(&dt);
        out.push((name, offset, dt));
        offset += size;
    }
    (out, offset)
}
