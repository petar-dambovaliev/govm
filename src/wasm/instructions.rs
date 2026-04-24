//! Re-exports for opcode types used throughout the UDF WASM compiler (`udf` branch:
//! `src/wasm/compiler/*.rs`). Prefer the same names when porting snippets:
//!
//! - `Instruction`, `MemArg`, `BlockType`, `ValType` from `wasm-encoder`
//!
//! UDF often accumulates `Vec<Instruction>` then copies into `wasm_encoder::Function`; this branch
//! uses [`crate::wasm::func_context::WasmFuncContext`] instead.

pub use wasm_encoder::{BlockType, Instruction, MemArg, ValType};
