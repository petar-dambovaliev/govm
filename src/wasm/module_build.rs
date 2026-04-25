//! Incremental module construction with a **stack of [`WasmFuncContext`]** (8bit-tag style).
//!
//! UDF’s `WasmCompiler` keeps section writers and `code_buffer` on one struct; this type splits
//! out the same responsibilities for the experimental `src/wasm` path.

use crate::wasm::func_context::WasmFuncContext;
use crate::wasm::layout::{MEMORY_MIN_PAGES, STACK_TOP};
use std::borrow::Cow;
use wasm_encoder::{
    CodeSection, ConstExpr, ElementSection, Elements, EntityType, ExportKind, ExportSection,
    FunctionSection, GlobalSection, GlobalType, ImportSection, MemorySection, MemoryType, Module,
    RefType, TableSection, TableType, TypeSection, ValType,
};

#[derive(Debug)]
pub struct WasmModuleBuilder {
    types: TypeSection,
    imports: ImportSection,
    functions: FunctionSection,
    memory: MemorySection,
    globals: GlobalSection,
    exports: ExportSection,
    completed_bodies: Vec<(u32, wasm_encoder::Function)>,
    next_type_idx: u32,
    next_func_idx: u32,
    num_imports: u32,
    /// Active nested function bodies (innermost at end), matching nested **Go** func compilation.
    func_stack: Vec<(u32, WasmFuncContext)>,
    rt_alloc_func_idx: Option<u32>,
    print_string_func_idx: Option<u32>,
    println_string_func_idx: Option<u32>,
    sp_global_idx: Option<u32>,
    next_global_idx: u32,
    /// Funcref table entries: (table_offset, func_idx) for interface vtables.
    vtable_entries: Vec<(u32, u32)>,
    vtable_size: u32,
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
            globals: GlobalSection::new(),
            exports: ExportSection::new(),
            completed_bodies: Vec::new(),
            next_type_idx: 0,
            next_func_idx: 0,
            num_imports: 0,
            func_stack: Vec::new(),
            rt_alloc_func_idx: None,
            print_string_func_idx: None,
            println_string_func_idx: None,
            sp_global_idx: None,
            next_global_idx: 0,
            vtable_entries: Vec::new(),
            vtable_size: 0,
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
        self.num_imports += 1;
        self.rt_alloc_func_idx = Some(func_idx);
        func_idx
    }

    pub fn rt_alloc_func_idx(&self) -> Option<u32> {
        self.rt_alloc_func_idx
    }

    /// `(i32, i32) -> ()` import `env.print_string` — writes bytes from linear memory to stdout.
    pub fn add_print_string_import(&mut self) -> u32 {
        let ty = self.next_type_idx;
        self.types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.imports
            .import("env", "print_string", EntityType::Function(ty));
        self.next_func_idx += 1;
        self.num_imports += 1;
        self.print_string_func_idx = Some(func_idx);
        func_idx
    }

    pub fn print_string_func_idx(&self) -> Option<u32> {
        self.print_string_func_idx
    }

    /// `(i32, i32) -> ()` import `env.println_string` — writes bytes + newline to stdout.
    pub fn add_println_string_import(&mut self) -> u32 {
        let ty = self.next_type_idx;
        self.types
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.imports
            .import("env", "println_string", EntityType::Function(ty));
        self.next_func_idx += 1;
        self.num_imports += 1;
        self.println_string_func_idx = Some(func_idx);
        func_idx
    }

    pub fn println_string_func_idx(&self) -> Option<u32> {
        self.println_string_func_idx
    }

    /// Mutable i32 global `$sp` initialized to [`STACK_TOP`], used for stack-allocated arrays.
    pub fn add_stack_pointer_global(&mut self) -> u32 {
        let idx = self.next_global_idx;
        self.globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(STACK_TOP as i32),
        );
        self.next_global_idx += 1;
        self.sp_global_idx = Some(idx);
        idx
    }

    pub fn sp_global_idx(&self) -> Option<u32> {
        self.sp_global_idx
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

    /// Start emitting a body for the function at `func_idx` that was just [`Self::define_function`].
    pub fn begin_func_body(&mut self, func_idx: u32, locals: Vec<(u32, ValType)>) {
        self.func_stack.push((func_idx, WasmFuncContext::new(locals)));
    }

    pub fn active(&mut self) -> &mut WasmFuncContext {
        self.func_stack
            .last_mut()
            .map(|(_, ctx)| ctx)
            .expect("WasmModuleBuilder: no active function (call begin_func_body first)")
    }

    /// Finish innermost body and buffer it for ordered output.
    pub fn end_func_body(&mut self) {
        let (func_idx, ctx) = self.func_stack.pop().expect("WasmModuleBuilder: end_func_body without begin");
        let func = ctx.finish();
        self.completed_bodies.push((func_idx, func));
    }

    /// Reserve `size` slots in the funcref table for interface vtables.
    pub fn set_vtable_size(&mut self, size: u32) {
        self.vtable_size = size;
    }

    /// Record a vtable entry: function at `table_offset` in table 0.
    pub fn add_vtable_entry(&mut self, table_offset: u32, func_idx: u32) {
        self.vtable_entries.push((table_offset, func_idx));
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.completed_bodies.sort_by_key(|(idx, _)| *idx);
        let mut code = CodeSection::new();
        for (_, func) in &self.completed_bodies {
            code.function(func);
        }

        let has_table = self.vtable_size > 0;

        let mut module = Module::new();
        module.section(&self.types);
        module.section(&self.imports);
        module.section(&self.functions);

        if has_table {
            let mut tables = TableSection::new();
            tables.table(TableType {
                element_type: RefType::FUNCREF,
                minimum: self.vtable_size as u64,
                maximum: Some(self.vtable_size as u64),
                table64: false,
                shared: false,
            });
            module.section(&tables);
        }

        module.section(&self.memory);
        module.section(&self.globals);
        module.section(&self.exports);

        if has_table && !self.vtable_entries.is_empty() {
            let mut elements = ElementSection::new();
            // Group contiguous runs to minimize element segments, but for simplicity
            // emit one segment per entry.
            for (offset, func_idx) in &self.vtable_entries {
                let funcs = vec![*func_idx];
                elements.active(
                    Some(0),
                    &ConstExpr::i32_const(*offset as i32),
                    Elements::Functions(Cow::Owned(funcs)),
                );
            }
            module.section(&elements);
        }

        module.section(&code);
        module.finish()
    }
}
