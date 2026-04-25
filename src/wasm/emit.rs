//! WASM module emission using [`crate::wasm::module_build::WasmModuleBuilder`] and
//! [`crate::wasm::func_context::WasmFuncContext`] (8bit-tag–style stacked bodies).
//!
//! Instruction patterns match the UDF branch: `Instruction::*` via `wasm-encoder`.

use crate::wasm::module_build::WasmModuleBuilder;
use wasm_encoder::ValType;

/// Build the smoke module: memory (2 pages), import `env.rt_alloc`, export `memory` + `demo`.
pub fn build_smoke_module() -> Vec<u8> {
    let mut b = WasmModuleBuilder::new();
    let alloc_idx = b.add_rt_alloc_import();
    debug_assert_eq!(alloc_idx, 0);

    let demo_ty = b.add_func_type(vec![], vec![ValType::I32]);
    b.add_default_memory();
    b.export_memory("memory", 0);

    let demo_idx = b.define_function(demo_ty);
    b.begin_func_body(demo_idx, vec![]);
    {
        let f = b.active();
        f.i32_const(16);
        f.call(alloc_idx);
    }
    b.end_func_body();

    b.export_func("demo", demo_idx);
    b.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_module_validates() {
        let bytes = build_smoke_module();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("wasm must validate");
    }

    #[test]
    fn smoke_module_roundtrip_print() {
        let bytes = build_smoke_module();
        let wat = wasmprinter::print_bytes(&bytes).expect("print");
        assert!(wat.contains("rt_alloc"));
        assert!(wat.contains("(export \"demo\""));
    }

    #[test]
    fn module_builder_two_functions_preserves_code_order() {
        let mut b = WasmModuleBuilder::new();
        let rt = b.add_rt_alloc_import();
        let t_i32 = b.add_func_type(vec![], vec![ValType::I32]);
        b.add_default_memory();

        let inner_idx = b.define_function(t_i32);
        b.begin_func_body(inner_idx, vec![]);
        {
            let f = b.active();
            f.i32_const(8);
            f.call(rt);
        }
        b.end_func_body();

        let outer_idx = b.define_function(t_i32);
        b.begin_func_body(outer_idx, vec![]);
        b.active().call(inner_idx);
        b.end_func_body();

        let bytes = b.finish();
        wasmparser::Validator::new()
            .validate_all(&bytes)
            .expect("two-function module validates");
        assert!(outer_idx > inner_idx);
    }
}
