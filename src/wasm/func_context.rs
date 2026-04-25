//! Per-function WASM body builder, analogous to **8bit-tag** [`crate::vm::compiler::FuncContext`]:
//! tracks one function’s emission while the compiler may stack nested contexts.
//!
//! Instruction helpers mirror the UDF compiler style (`Instruction::LocalGet`, `Instruction::Call`,
//! `MemArg { offset, align, memory_index }`, …) but target [`wasm_encoder::Function`] instead of
//! `Vec<Instruction>` + later copy.

use wasm_encoder::{Function, Instruction, MemArg, ValType};

/// One function body under construction (locals + instruction stream).
#[derive(Debug)]
pub struct WasmFuncContext {
    func: Function,
}

impl WasmFuncContext {
    pub fn new(local_groups: Vec<(u32, ValType)>) -> Self {
        Self {
            func: Function::new(local_groups),
        }
    }

    #[inline]
    pub fn emit(&mut self, ins: &Instruction) {
        self.func.instruction(ins);
    }

    #[inline]
    pub fn i32_const(&mut self, v: i32) {
        self.emit(&Instruction::I32Const(v));
    }

    #[inline]
    pub fn i64_const(&mut self, v: i64) {
        self.emit(&Instruction::I64Const(v));
    }

    #[inline]
    pub fn call(&mut self, func_idx: u32) {
        self.emit(&Instruction::Call(func_idx));
    }

    #[inline]
    pub fn local_get(&mut self, idx: u32) {
        self.emit(&Instruction::LocalGet(idx));
    }

    #[inline]
    pub fn local_set(&mut self, idx: u32) {
        self.emit(&Instruction::LocalSet(idx));
    }

    #[inline]
    pub fn local_tee(&mut self, idx: u32) {
        self.emit(&Instruction::LocalTee(idx));
    }

    #[inline]
    pub fn global_get(&mut self, idx: u32) {
        self.emit(&Instruction::GlobalGet(idx));
    }

    #[inline]
    pub fn global_set(&mut self, idx: u32) {
        self.emit(&Instruction::GlobalSet(idx));
    }

    #[inline]
    pub fn drop(&mut self) {
        self.emit(&Instruction::Drop);
    }

    #[inline]
    pub fn i32_store8(&mut self, offset: u64) {
        self.emit(&Instruction::I32Store8(MemArg {
            offset,
            align: 0,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn i32_store(&mut self, offset: u64) {
        self.emit(&Instruction::I32Store(MemArg {
            offset,
            align: 2,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn i32_load(&mut self, offset: u64) {
        self.emit(&Instruction::I32Load(MemArg {
            offset,
            align: 2,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn i64_store(&mut self, offset: u64) {
        self.emit(&Instruction::I64Store(MemArg {
            offset,
            align: 3,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn i64_load(&mut self, offset: u64) {
        self.emit(&Instruction::I64Load(MemArg {
            offset,
            align: 3,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn f32_store(&mut self, offset: u64) {
        self.emit(&Instruction::F32Store(MemArg {
            offset,
            align: 2,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn f32_load(&mut self, offset: u64) {
        self.emit(&Instruction::F32Load(MemArg {
            offset,
            align: 2,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn f64_store(&mut self, offset: u64) {
        self.emit(&Instruction::F64Store(MemArg {
            offset,
            align: 3,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn f64_load(&mut self, offset: u64) {
        self.emit(&Instruction::F64Load(MemArg {
            offset,
            align: 3,
            memory_index: 0,
        }));
    }

    #[inline]
    pub fn memory_copy(&mut self) {
        self.emit(&Instruction::MemoryCopy {
            src_mem: 0,
            dst_mem: 0,
        });
    }

    #[inline]
    pub fn memory_fill(&mut self) {
        self.emit(&Instruction::MemoryFill(0));
    }

    #[inline]
    pub fn ret(&mut self) {
        self.emit(&Instruction::Return);
    }

    /// Append `end`, return the finished [`Function`] for [`wasm_encoder::CodeSection`].
    pub fn finish(mut self) -> Function {
        self.emit(&Instruction::End);
        self.func
    }
}
