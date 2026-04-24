//! Linear memory layout shared by the WASM module and the host.
//!
//! Initial memory uses two 64 KiB pages (128 KiB). The **lower half** is reserved
//! for guest data/stack/conventions; the **upper half** starting at [`HEAP_BASE`]
//! is the only region the host bump allocator uses for [`crate::wasm::host_heap`].

/// Size of one WASM page (64 KiB).
pub const WASM_PAGE_SIZE: u32 = 65536;

/// Minimum memory size (pages). Two pages ⇒ 128 KiB total; [`HEAP_BASE`] is the midpoint.
pub const MEMORY_MIN_PAGES: u64 = 2;

/// Byte offset where the host-managed heap starts (upper half of initial memory).
pub const HEAP_BASE: i32 = (MEMORY_MIN_PAGES as i32 / 2) * (WASM_PAGE_SIZE as i32);
