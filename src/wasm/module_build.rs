//! Incremental module construction with a **stack of [`WasmFuncContext`]** (8bit-tag style).
//!
//! UDF’s `WasmCompiler` keeps section writers and `code_buffer` on one struct; this type splits
//! out the same responsibilities for the experimental `src/wasm` path.

use crate::wasm::func_context::WasmFuncContext;
use crate::wasm::layout::MEMORY_MIN_PAGES;
use wasm_encoder::{
    CodeSection, EntityType, ExportKind, ExportSection, FunctionSection, ImportSection,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

#[derive(Debug)]
pub struct WasmModuleBuilder {
    types: TypeSection,
    imports: ImportSection,
    functions: FunctionSection,
    memory: MemorySection,
    exports: ExportSection,
    code: CodeSection,
    next_type_idx: u32,
    next_func_idx: u32,
    /// Active nested function bodies (innermost at end), matching nested **Go** func compilation.
    func_stack: Vec<WasmFuncContext>,
    rt_alloc_func_idx: Option<u32>,
}

impl Default for WasmModuleBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl WasmModuleBuilder {
    pub fn new() -> Self {
        Self {
            types: TypeSection::new(),
            imports: ImportSection::new(),
            functions: FunctionSection::new(),
            memory: MemorySection::new(),
            exports: ExportSection::new(),
            code: CodeSection::new(),
            next_type_idx: 0,
            next_func_idx: 0,
            func_stack: Vec::new(),
            rt_alloc_func_idx: None,
        }
    }

    /// `(i32) -> i32` import `env.rt_alloc` — same ABI as [`crate::wasm::runtime::host_rt_alloc`].
    pub fn add_rt_alloc_import(&mut self) -> u32 {
        let ty = self.next_type_idx;
        self.types
            .ty()
            .function(vec![ValType::I32], vec![ValType::I32]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.imports
            .import("env", "rt_alloc", EntityType::Function(ty));
        self.next_func_idx += 1;
        self.rt_alloc_func_idx = Some(func_idx);
        func_idx
    }

    pub fn rt_alloc_func_idx(&self) -> Option<u32> {
        self.rt_alloc_func_idx
    }

    /// Register a function type; returns its type index.
    pub fn add_func_type(&mut self, params: Vec<ValType>, results: Vec<ValType>) -> u32 {
        let idx = self.next_type_idx;
        self.types.ty().function(params, results);
        self.next_type_idx += 1;
        idx
    }

    /// One memory, exported under `memory` (index 0), same limits as smoke module.
    pub fn add_default_memory(&mut self) {
        self.memory.memory(MemoryType {
            minimum: MEMORY_MIN_PAGES,
            maximum: Some(256),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
    }

    pub fn export_memory(&mut self, name: &str, mem_idx: u32) {
        self.exports.export(name, ExportKind::Memory, mem_idx);
    }

    pub fn export_func(&mut self, name: &str, func_idx: u32) {
        self.exports.export(name, ExportKind::Func, func_idx);
    }

    /// Append an entry to the **function** section; returns the **function index** for `call` / exports.
    pub fn define_function(&mut self, type_idx: u32) -> u32 {
        self.functions.function(type_idx);
        let idx = self.next_func_idx;
        self.next_func_idx += 1;
        idx
    }

    /// Start emitting a body for the function that was just [`Self::define_function`].
    pub fn begin_func_body(&mut self, locals: Vec<(u32, ValType)>) {
        self.func_stack.push(WasmFuncContext::new(locals));
    }

    pub fn active(&mut self) -> &mut WasmFuncContext {
        self.func_stack
            .last_mut()
            .expect("WasmModuleBuilder: no active function (call begin_func_body first)")
    }

    /// Finish innermost body and append it to the **code** section in order.
    pub fn end_func_body(&mut self) {
        let ctx = self.func_stack.pop().expect("WasmModuleBuilder: end_func_body without begin");
        let func = ctx.finish();
        self.code.function(&func);
    }

    pub fn finish(self) -> Vec<u8> {
        let mut module = Module::new();
        module.section(&self.types);
        module.section(&self.imports);
        module.section(&self.functions);
        module.section(&self.memory);
        module.section(&self.exports);
        module.section(&self.code);
        module.finish()
    }
}
