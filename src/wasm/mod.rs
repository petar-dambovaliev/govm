//! WASM support on the `wasm-host-heap-8bit` line: MVP emission + host heap in upper linear memory.
//!
//! - **No** WebAssembly GC proposal types — only MVP `i32` / memory / imports.
//! - **Layout**: initial memory is [`layout::MEMORY_MIN_PAGES`] pages; the upper half
//!   (from [`layout::HEAP_BASE`]) is reserved for allocations served by the host
//!   [`host_heap::HostHeapBump`] (see [`runtime::run_smoke_demo`]).
//!
//! Full Go compilation still uses `vm::compiler`; this module is the seam for
//! experimenting with `wasm-encoder` / Wasmtime the same way the UDF branch does.
//!
//! ## Structure (8bit-tag–like)
//!
//! - [`WasmModuleBuilder`] — section writers + `next_type_idx` / `next_func_idx` (UDF `WasmCompiler` subset).
//! - [`WasmFuncContext`] — one `wasm_encoder::Function` body; stack `func_stack` on the builder
//!   for nested emission (same role as 8bit-tag `func_contexts` during nested Go func compile).
//! - [`instructions`] — re-exports `Instruction`, `MemArg`, `BlockType`, `ValType` for copy-paste from UDF.

pub mod emit;
pub mod func_context;
pub mod host_heap;
pub mod instructions;
pub mod layout;
pub mod module_build;
pub mod runtime;

pub use emit::build_smoke_module;
pub use func_context::WasmFuncContext;
pub use module_build::WasmModuleBuilder;
pub use layout::{HEAP_BASE, MEMORY_MIN_PAGES, WASM_PAGE_SIZE};
pub use runtime::run_smoke_demo;
