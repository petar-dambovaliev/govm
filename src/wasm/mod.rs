//! WASM support on the `wasm-host-heap-8bit` line: MVP emission + host heap in upper linear memory.
//!
//! - **No** WebAssembly GC proposal types — only MVP `i32` / memory / imports.
//! - **Layout**: initial memory is [`layout::MEMORY_MIN_PAGES`] pages; the upper half
//!   (from [`layout::HEAP_BASE`]) is reserved for allocations served by the host
//!   [`host_heap::HostHeapBump`] (see [`runtime::run_smoke_demo`]).
//!
//! Full Go compilation still uses `vm::compiler`; this module is the seam for
//! experimenting with `wasm-encoder` / Wasmtime the same way the UDF branch does.

pub mod emit;
pub mod host_heap;
pub mod layout;
pub mod runtime;

pub use emit::build_smoke_module;
pub use layout::{HEAP_BASE, MEMORY_MIN_PAGES, WASM_PAGE_SIZE};
pub use runtime::run_smoke_demo;
