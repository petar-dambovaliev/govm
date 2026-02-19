use wasm_encoder::{
    CodeSection, ConstExpr, ExportKind, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, Instruction, MemorySection, MemoryType, Module, TypeSection, ValType,
};

const HEAP_BASE: i32 = 1024;

pub fn emit_allocator_into(module: &mut ModuleAllocator) {
    module.add_memory();
    module.add_heap_pointer_global();
    module.add_alloc_function();
    module.add_reset_function();
}

pub struct ModuleAllocator {
    pub type_section: TypeSection,
    pub function_section: FunctionSection,
    pub memory_section: MemorySection,
    pub export_section: ExportSection,
    pub global_section: GlobalSection,
    pub code_section: CodeSection,
    pub next_type_idx: u32,
    pub next_func_idx: u32,
    pub alloc_func_idx: u32,
    pub reset_func_idx: u32,
    pub heap_ptr_global_idx: u32,
}

impl ModuleAllocator {
    pub fn new() -> Self {
        Self {
            type_section: TypeSection::new(),
            function_section: FunctionSection::new(),
            memory_section: MemorySection::new(),
            export_section: ExportSection::new(),
            global_section: GlobalSection::new(),
            code_section: CodeSection::new(),
            next_type_idx: 0,
            next_func_idx: 0,
            alloc_func_idx: 0,
            reset_func_idx: 0,
            heap_ptr_global_idx: 0,
        }
    }

    fn add_memory(&mut self) {
        self.memory_section.memory(MemoryType {
            minimum: 1,
            maximum: Some(256),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        self.export_section.export("memory", ExportKind::Memory, 0);
    }

    fn add_heap_pointer_global(&mut self) {
        self.heap_ptr_global_idx = 0;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(HEAP_BASE),
        );
    }

    /// alloc(size: i32) -> i32 (pointer)
    fn add_alloc_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section.ty().function(
            vec![ValType::I32],
            vec![ValType::I32],
        );
        self.next_type_idx += 1;

        self.alloc_func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![(1, ValType::I32)]);

        // local 0 = size parameter
        // local 1 = current pointer (temp)

        // Align size to 8 bytes: size = (size + 7) & ~7
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Const(7));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Const(!7));
        func.instruction(&Instruction::I32And);
        func.instruction(&Instruction::LocalSet(0));

        // Save current heap pointer to local 1
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global_idx));
        func.instruction(&Instruction::LocalSet(1));

        // Advance heap pointer: heap_ptr += size
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global_idx));
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global_idx));

        // Return old pointer
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.export_section
            .export("alloc", ExportKind::Func, self.alloc_func_idx);
    }

    /// reset() -- resets heap pointer to base
    fn add_reset_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        self.reset_func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        func.instruction(&Instruction::I32Const(HEAP_BASE));
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global_idx));
        func.instruction(&Instruction::End);

        self.code_section.function(&func);
        self.export_section
            .export("reset", ExportKind::Func, self.reset_func_idx);
    }

    pub fn build(self) -> Vec<u8> {
        let mut module = Module::new();
        module.section(&self.type_section);
        module.section(&self.function_section);
        module.section(&self.memory_section);
        module.section(&self.global_section);
        module.section(&self.export_section);
        module.section(&self.code_section);
        module.finish()
    }
}
