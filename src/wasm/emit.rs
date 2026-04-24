//! Minimal WASM module emission (MVP stack + `call` to imports).
//!
//! Instruction patterns mirror the UDF `WasmCompiler` / `wasm-encoder` usage:
//! `Function::new`, `Instruction::*`, sections in link order.

use crate::wasm::layout::MEMORY_MIN_PAGES;
use wasm_encoder::{
    CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection, ImportSection,
    Instruction, MemorySection, MemoryType, Module, TypeSection, ValType,
};

/// Build a tiny module: memory (2 pages), import `env.rt_alloc(i32)->i32`, export `memory`
/// and `demo` (calls `rt_alloc(16)`, returns pointer).
pub fn build_smoke_module() -> Vec<u8> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    // type index 0: rt_alloc
    types
        .ty()
        .function(vec![ValType::I32], vec![ValType::I32]);
    // type index 1: demo () -> i32
    types.ty().function(vec![], vec![ValType::I32]);

    let mut imports = ImportSection::new();
    imports.import("env", "rt_alloc", EntityType::Function(0));

    let mut functions = FunctionSection::new();
    functions.function(1);

    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: MEMORY_MIN_PAGES,
        maximum: Some(256),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });

    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    // func index 0 = import rt_alloc, 1 = demo
    exports.export("demo", ExportKind::Func, 1);

    let mut demo = Function::new(vec![]);
    demo.instruction(&Instruction::I32Const(16));
    demo.instruction(&Instruction::Call(0));
    demo.instruction(&Instruction::End);

    let mut code = CodeSection::new();
    code.function(&demo);

    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    module.section(&memory);
    module.section(&exports);
    module.section(&code);

    module.finish()
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
}
