use crate::parser::ast;
use crate::symbols::{DefineType, Error};
use crate::wasm::types::WasmType;
use wasm_encoder::{
    BlockType, ExportKind, Function, HeapType, Instruction, MemArg, ValType,
};

use super::WasmCompiler;

const TYPE_TAG_I32: i32 = 0;
const TYPE_TAG_I64: i32 = 1;
const TYPE_TAG_F32: i32 = 2;
const TYPE_TAG_F64: i32 = 3;
const TYPE_TAG_STRING: i32 = 4;
const TYPE_TAG_BOOL: i32 = 5;
const TYPE_TAG_TIMESTAMP: i32 = 6;

const COLUMN_ENTRY_SIZE: i32 = 16;

impl WasmCompiler {
    pub(crate) fn emit_udf_wrappers(&mut self, file: &ast::File) -> Result<(), Error> {
        let func_descs: Vec<_> = self.manifest.functions.clone();
        for func_desc in &func_descs {
            if !self.has_struct_slice_param(func_desc, file) {
                continue;
            }
            self.emit_single_udf_wrapper(func_desc, file)?;
        }
        Ok(())
    }

    fn has_struct_slice_param(
        &self,
        func_desc: &crate::wasm::udf::FunctionDescriptor,
        file: &ast::File,
    ) -> bool {
        for decl in &file.decl {
            if let ast::Declaration::Function(func_decl) = decl {
                if func_decl.name.name != func_desc.name {
                    continue;
                }
                for field in &func_decl.typ.params.list {
                    let type_name = self.expr_type_name(&field.typ);
                    if let Some(struct_name) = type_name.strip_prefix("[]") {
                        if self.struct_defs.contains_key(struct_name) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn get_func_struct_types(
        &self,
        func_decl: &ast::FuncDecl,
    ) -> (Option<String>, Option<String>) {
        let mut input_struct = None;
        let mut output_struct = None;

        for field in &func_decl.typ.params.list {
            let type_name = self.expr_type_name(&field.typ);
            if let Some(struct_name) = type_name.strip_prefix("[]") {
                if self.struct_defs.contains_key(struct_name) {
                    input_struct = Some(struct_name.to_string());
                }
            }
        }

        for field in &func_decl.typ.result.list {
            let type_name = self.expr_type_name(&field.typ);
            if let Some(struct_name) = type_name.strip_prefix("[]") {
                if self.struct_defs.contains_key(struct_name) {
                    output_struct = Some(struct_name.to_string());
                }
            }
        }

        (input_struct, output_struct)
    }

    fn emit_single_udf_wrapper(
        &mut self,
        func_desc: &crate::wasm::udf::FunctionDescriptor,
        file: &ast::File,
    ) -> Result<(), Error> {
        let func_decl = file
            .decl
            .iter()
            .find_map(|d| {
                if let ast::Declaration::Function(fd) = d {
                    if fd.name.name == func_desc.name {
                        Some(fd)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                Error::InternalError(format!(
                    "function '{}' not found in AST for UDF wrapper",
                    func_desc.name
                ))
            })?;

        let (input_struct_name, output_struct_name) = self.get_func_struct_types(func_decl);
        let input_struct_name = match input_struct_name {
            Some(n) => n,
            None => return Ok(()),
        };

        let input_sd = self.struct_defs.get(&input_struct_name).cloned().ok_or_else(|| {
            Error::InternalError(format!("struct def for '{}' not found", input_struct_name))
        })?;

        let input_gc_type_idx = match input_sd.gc_type_idx {
            Some(idx) => idx,
            None => {
                if let Some(&idx) = self.gc_struct_types.get(&input_struct_name) {
                    idx
                } else {
                    return Err(Error::InternalError(format!(
                        "GC type for struct '{}' not found", input_struct_name
                    )));
                }
            }
        };

        let input_gc_vt = Self::gc_ref_val_type(input_gc_type_idx);

        let input_elem_vt = input_gc_vt;
        let (input_slice_gc_idx, input_array_gc_idx) =
            self.get_or_create_gc_slice_type(input_elem_vt);

        let original_func_idx = self
            .functions
            .iter()
            .find(|f| f.name == func_desc.name)
            .map(|f| f.wasm_func_idx)
            .ok_or_else(|| {
                Error::InternalError(format!(
                    "compiled function '{}' not found for UDF wrapper",
                    func_desc.name
                ))
            })?;

        let alloc_idx = self.alloc_func_idx()?;

        let gc_string_bridge_idx = self
            .functions
            .iter()
            .find(|f| f.name == "__make_gc_string")
            .map(|f| f.wasm_func_idx);

        let wrapper_name = format!("__udf_{}", func_desc.name);

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(vec![ValType::I32], vec![ValType::I32]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        // Locals layout: 0=header_ptr (param)
        // 1=row_count, 2=column_count, 3=i (loop var), 4=input_slice_gc,
        // 5=input_array_gc, 6=struct_ref, 7=col_data_ptr, 8=output_header_ptr
        let mut locals_spec: Vec<(u32, ValType)> = vec![
            (1, ValType::I32),                                      // 1: row_count
            (1, ValType::I32),                                      // 2: column_count
            (1, ValType::I32),                                      // 3: i
            (1, Self::gc_ref_val_type(input_slice_gc_idx)),         // 4: input_slice_gc
            (1, Self::gc_ref_val_type(input_array_gc_idx)),         // 5: input_array_gc
            (1, input_gc_vt),                                       // 6: struct_ref
            (1, ValType::I32),                                      // 7: col_data_ptr
            (1, ValType::I32),                                      // 8: output_header_ptr
        ];

        let local_row_count = 1u32;
        let local_column_count = 2u32;
        let local_i = 3u32;
        let local_input_slice = 4u32;
        let local_input_array = 5u32;
        let local_struct_ref = 6u32;
        let local_col_data_ptr = 7u32;
        let local_output_header = 8u32;

        let mut next_local = 9u32;

        // Extra locals for output struct fields if we have output
        let output_sd = output_struct_name
            .as_ref()
            .and_then(|n| self.struct_defs.get(n).cloned());

        let output_gc_type_idx = output_struct_name
            .as_ref()
            .and_then(|n| self.gc_struct_types.get(n.as_str()).copied());

        let output_slice_info = if let (Some(out_sd), Some(gc_idx)) =
            (&output_sd, output_gc_type_idx)
        {
            let out_gc_vt = Self::gc_ref_val_type(gc_idx);
            let (slice_gc, array_gc) = self.get_or_create_gc_slice_type(out_gc_vt);
            Some((out_sd.clone(), gc_idx, slice_gc, array_gc))
        } else {
            None
        };

        let local_output_slice = next_local;
        if output_slice_info.is_some() {
            let (_, _, slice_gc, _) = output_slice_info.as_ref().unwrap();
            locals_spec.push((1, Self::gc_ref_val_type(*slice_gc)));
            next_local += 1;
        }

        let local_output_array = next_local;
        if output_slice_info.is_some() {
            let (_, _, _, array_gc) = output_slice_info.as_ref().unwrap();
            locals_spec.push((1, Self::gc_ref_val_type(*array_gc)));
            next_local += 1;
        }

        let local_output_row_count = next_local;
        if output_slice_info.is_some() {
            locals_spec.push((1, ValType::I32));
            next_local += 1;
        }

        let local_output_struct = next_local;
        if let Some((_, gc_idx, _, _)) = &output_slice_info {
            locals_spec.push((1, Self::gc_ref_val_type(*gc_idx)));
            next_local += 1;
        }

        // Extra temp locals for string handling
        let local_str_total_bytes = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let local_temp_i32 = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let local_j = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let byte_array_gc_idx = self.gc_builtin_types.byte_array;
        let local_byte_array_ref = next_local;
        if byte_array_gc_idx.is_some() {
            locals_spec.push((1, Self::gc_ref_val_type(byte_array_gc_idx.unwrap())));
            next_local += 1;
        }

        let time_gc_idx = self.gc_struct_types.get("time.Time").copied();
        let (local_temp_i64, local_temp_i64_2, local_time_ref) = if time_gc_idx.is_some() {
            let t1 = next_local;
            locals_spec.push((1, ValType::I64));
            next_local += 1;

            let t2 = next_local;
            locals_spec.push((1, ValType::I64));
            next_local += 1;

            let tr = next_local;
            locals_spec.push((1, Self::gc_ref_val_type(time_gc_idx.unwrap())));
            next_local += 1;

            (t1, t2, tr)
        } else {
            (0, 0, 0)
        };

        // Locals for GC ↔ linear memory conversion
        let local_linear_data_ptr = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let local_linear_slice_ptr = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let local_row_base = next_local;
        locals_spec.push((1, ValType::I32));
        next_local += 1;

        let _ = next_local;

        let mut func = Function::new(locals_spec);

        // === Step 1: Read header ===
        // row_count = *(header_ptr)
        func.instruction(&Instruction::LocalGet(0)); // header_ptr
        func.instruction(&Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        func.instruction(&Instruction::LocalSet(local_row_count));

        // column_count = *(header_ptr + 4)
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        func.instruction(&Instruction::LocalSet(local_column_count));

        // === Step 2: Create input array and slice ===
        // input_array = ArrayNew(input_array_gc_idx, null, row_count)
        func.instruction(&Instruction::RefNull(HeapType::Concrete(input_gc_type_idx)));
        func.instruction(&Instruction::LocalGet(local_row_count));
        func.instruction(&Instruction::ArrayNew(input_array_gc_idx));
        func.instruction(&Instruction::LocalSet(local_input_array));

        // input_slice = StructNew(input_slice_gc_idx, input_array, 0, row_count, row_count)
        func.instruction(&Instruction::LocalGet(local_input_array));
        func.instruction(&Instruction::I32Const(0)); // offset
        func.instruction(&Instruction::LocalGet(local_row_count)); // len
        func.instruction(&Instruction::LocalGet(local_row_count)); // cap
        func.instruction(&Instruction::StructNew(input_slice_gc_idx));
        func.instruction(&Instruction::LocalSet(local_input_slice));

        // === Step 3: Unmarshal columns into structs ===
        // i = 0
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(local_i));

        // outer loop: for i = 0; i < row_count; i++
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));

        // break if i >= row_count
        func.instruction(&Instruction::LocalGet(local_i));
        func.instruction(&Instruction::LocalGet(local_row_count));
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));

        // struct_ref = StructNewDefault(input_gc_type_idx)
        func.instruction(&Instruction::StructNewDefault(input_gc_type_idx));
        func.instruction(&Instruction::LocalSet(local_struct_ref));

        // For each field, read from column buffer and set on struct
        for (field_idx, sf) in input_sd.fields.iter().enumerate() {
            let col_entry_offset = 8 + (field_idx as i32) * COLUMN_ENTRY_SIZE;

            // col_data_ptr = *(header_ptr + col_entry_offset + 4)
            func.instruction(&Instruction::LocalGet(0)); // header_ptr
            func.instruction(&Instruction::I32Load(MemArg {
                offset: (col_entry_offset + 4) as u64,
                align: 2,
                memory_index: 0,
            }));
            func.instruction(&Instruction::LocalSet(local_col_data_ptr));

            // Load value from column buffer at data_ptr + i * elem_size
            func.instruction(&Instruction::LocalGet(local_struct_ref));

            match sf.wasm_type {
                WasmType::I32 => {
                    let is_bool = matches!(&sf.field_type, Some(DefineType::Bool));
                    if is_bool {
                        // Bool: one byte per value
                        func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                        func.instruction(&Instruction::LocalGet(local_i));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Load8U(MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        }));
                    } else {
                        func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                        func.instruction(&Instruction::LocalGet(local_i));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                    }
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: input_gc_type_idx,
                        field_index: sf.field_index,
                    });
                }
                WasmType::I64 => {
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(8));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::I64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: input_gc_type_idx,
                        field_index: sf.field_index,
                    });
                }
                WasmType::F32 => {
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(4));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::F32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: input_gc_type_idx,
                        field_index: sf.field_index,
                    });
                }
                WasmType::F64 => {
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(8));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::F64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: input_gc_type_idx,
                        field_index: sf.field_index,
                    });
                }
                WasmType::Ref(ref_idx)
                    if Some(ref_idx) == self.gc_builtin_types.go_string =>
                {
                    // String: read offsets[i] and offsets[i+1], then call __make_gc_string
                    if let Some(bridge_idx) = gc_string_bridge_idx {
                        let offsets_base = local_col_data_ptr;
                        // offsets[i]
                        func.instruction(&Instruction::LocalGet(offsets_base));
                        func.instruction(&Instruction::LocalGet(local_i));
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        }));
                        func.instruction(&Instruction::LocalSet(local_temp_i32)); // start

                        // offsets[i+1]
                        func.instruction(&Instruction::LocalGet(offsets_base));
                        func.instruction(&Instruction::LocalGet(local_i));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Load(MemArg {
                            offset: 0,
                            align: 2,
                            memory_index: 0,
                        })); // end on stack

                        // len = end - start
                        func.instruction(&Instruction::LocalGet(local_temp_i32));
                        func.instruction(&Instruction::I32Sub); // len

                        func.instruction(&Instruction::LocalSet(local_str_total_bytes)); // len

                        // string data base = data_ptr + (row_count + 1) * 4
                        func.instruction(&Instruction::LocalGet(offsets_base));
                        func.instruction(&Instruction::LocalGet(local_row_count));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        // + offsets[i]
                        func.instruction(&Instruction::LocalGet(local_temp_i32));
                        func.instruction(&Instruction::I32Add);

                        // ptr on stack, now push len
                        func.instruction(&Instruction::LocalGet(local_str_total_bytes));

                        // call __make_gc_string(ptr, len)
                        func.instruction(&Instruction::Call(bridge_idx));
                        func.instruction(&Instruction::StructSet {
                            struct_type_index: input_gc_type_idx,
                            field_index: sf.field_index,
                        });
                    } else {
                        // No string bridge, pop the struct ref we already pushed
                        func.instruction(&Instruction::Drop);
                    }
                }
                WasmType::Ref(ref_idx) if self.is_time_struct(ref_idx) => {
                    // Timestamp: read i64 micros, construct time.Time GC struct directly
                    // Matches Go's time.Unix() normalization for negative values.
                    const UNIX_TO_INTERNAL: i64 = 62135596800;

                    // Load micros from column buffer
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(8));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::I64Load(MemArg {
                        offset: 0,
                        align: 3,
                        memory_index: 0,
                    }));
                    func.instruction(&Instruction::LocalSet(local_temp_i64)); // usec

                    // nsec = (usec % 1_000_000) * 1_000
                    func.instruction(&Instruction::LocalGet(local_temp_i64));
                    func.instruction(&Instruction::I64Const(1_000_000));
                    func.instruction(&Instruction::I64RemS);
                    func.instruction(&Instruction::I64Const(1_000));
                    func.instruction(&Instruction::I64Mul);
                    func.instruction(&Instruction::LocalSet(local_temp_i64_2)); // nsec

                    // sec = usec / 1_000_000
                    func.instruction(&Instruction::LocalGet(local_temp_i64));
                    func.instruction(&Instruction::I64Const(1_000_000));
                    func.instruction(&Instruction::I64DivS);
                    func.instruction(&Instruction::LocalSet(local_temp_i64)); // sec (reuse local)

                    // Normalize: if nsec < 0 { nsec += 1_000_000_000; sec -= 1 }
                    func.instruction(&Instruction::LocalGet(local_temp_i64_2));
                    func.instruction(&Instruction::I64Const(0));
                    func.instruction(&Instruction::I64LtS);
                    func.instruction(&Instruction::If(BlockType::Empty));
                    {
                        func.instruction(&Instruction::LocalGet(local_temp_i64_2));
                        func.instruction(&Instruction::I64Const(1_000_000_000));
                        func.instruction(&Instruction::I64Add);
                        func.instruction(&Instruction::LocalSet(local_temp_i64_2));

                        func.instruction(&Instruction::LocalGet(local_temp_i64));
                        func.instruction(&Instruction::I64Const(1));
                        func.instruction(&Instruction::I64Sub);
                        func.instruction(&Instruction::LocalSet(local_temp_i64));
                    }
                    func.instruction(&Instruction::End); // end if

                    // Create time.Time GC struct
                    func.instruction(&Instruction::StructNewDefault(ref_idx));
                    func.instruction(&Instruction::LocalSet(local_time_ref));

                    // field 0 (wall) = nsec
                    func.instruction(&Instruction::LocalGet(local_time_ref));
                    func.instruction(&Instruction::LocalGet(local_temp_i64_2));
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: ref_idx,
                        field_index: 0,
                    });

                    // field 1 (ext) = sec + unixToInternal
                    func.instruction(&Instruction::LocalGet(local_time_ref));
                    func.instruction(&Instruction::LocalGet(local_temp_i64));
                    func.instruction(&Instruction::I64Const(UNIX_TO_INTERNAL));
                    func.instruction(&Instruction::I64Add);
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: ref_idx,
                        field_index: 1,
                    });

                    // field 2 (loc) stays null (= UTC) from StructNewDefault

                    // Set time.Time ref on the user's input struct
                    // struct_ref is already on the stack from earlier push
                    func.instruction(&Instruction::LocalGet(local_time_ref));
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: input_gc_type_idx,
                        field_index: sf.field_index,
                    });
                }
                _ => {
                    func.instruction(&Instruction::Drop);
                }
            }
        }

        // Store struct in array: input_array[i] = struct_ref
        func.instruction(&Instruction::LocalGet(local_input_array));
        func.instruction(&Instruction::LocalGet(local_i));
        func.instruction(&Instruction::LocalGet(local_struct_ref));
        func.instruction(&Instruction::ArraySet(input_array_gc_idx));

        // i++
        func.instruction(&Instruction::LocalGet(local_i));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(local_i));

        func.instruction(&Instruction::Br(0)); // continue loop
        func.instruction(&Instruction::End); // end loop
        func.instruction(&Instruction::End); // end block

        // === Step 3.5: Flatten GC slice → linear memory slice ===
        {
            let total_size = input_sd.total_size as i32;

            // Allocate struct data array: total_size * row_count
            func.instruction(&Instruction::LocalGet(local_row_count));
            func.instruction(&Instruction::I32Const(total_size));
            func.instruction(&Instruction::I32Mul);
            func.instruction(&Instruction::Call(alloc_idx));
            func.instruction(&Instruction::LocalSet(local_linear_data_ptr));

            // Allocate 12-byte slice header
            func.instruction(&Instruction::I32Const(12));
            func.instruction(&Instruction::Call(alloc_idx));
            func.instruction(&Instruction::LocalSet(local_linear_slice_ptr));

            // Write slice header: [data_ptr, len, cap]
            func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
            func.instruction(&Instruction::LocalGet(local_linear_data_ptr));
            func.instruction(&Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
            func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
            func.instruction(&Instruction::LocalGet(local_row_count));
            func.instruction(&Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
            func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
            func.instruction(&Instruction::LocalGet(local_row_count));
            func.instruction(&Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

            // Loop: copy each GC struct's fields into linear memory
            func.instruction(&Instruction::I32Const(0));
            func.instruction(&Instruction::LocalSet(local_i));

            func.instruction(&Instruction::Block(BlockType::Empty));
            func.instruction(&Instruction::Loop(BlockType::Empty));

            func.instruction(&Instruction::LocalGet(local_i));
            func.instruction(&Instruction::LocalGet(local_row_count));
            func.instruction(&Instruction::I32GeU);
            func.instruction(&Instruction::BrIf(1));

            // struct_ref = input_array[i]
            func.instruction(&Instruction::LocalGet(local_input_array));
            func.instruction(&Instruction::LocalGet(local_i));
            func.instruction(&Instruction::ArrayGet(input_array_gc_idx));
            func.instruction(&Instruction::LocalSet(local_struct_ref));

            // row_base = data_ptr + i * total_size
            func.instruction(&Instruction::LocalGet(local_linear_data_ptr));
            func.instruction(&Instruction::LocalGet(local_i));
            func.instruction(&Instruction::I32Const(total_size));
            func.instruction(&Instruction::I32Mul);
            func.instruction(&Instruction::I32Add);
            func.instruction(&Instruction::LocalSet(local_row_base));

            for f in &input_sd.fields {
                func.instruction(&Instruction::LocalGet(local_row_base));
                func.instruction(&Instruction::LocalGet(local_struct_ref));
                func.instruction(&Instruction::StructGet {
                    struct_type_index: input_gc_type_idx,
                    field_index: f.field_index,
                });
                let mem_arg = |align: u32| MemArg { offset: f.offset as u64, align, memory_index: 0 };
                match f.wasm_type {
                    WasmType::I32 => func.instruction(&Instruction::I32Store(mem_arg(2))),
                    WasmType::I64 => func.instruction(&Instruction::I64Store(mem_arg(3))),
                    WasmType::F32 => func.instruction(&Instruction::F32Store(mem_arg(2))),
                    WasmType::F64 => func.instruction(&Instruction::F64Store(mem_arg(3))),
                    WasmType::Ref(_) => {
                        func.instruction(&Instruction::Drop); // drop the ref value
                        func.instruction(&Instruction::Drop); // drop the address
                        func.instruction(&Instruction::LocalGet(local_row_base));
                        func.instruction(&Instruction::I32Const(0));
                        func.instruction(&Instruction::I32Store(mem_arg(2)));
                        &mut func
                    }
                };
            }

            func.instruction(&Instruction::LocalGet(local_i));
            func.instruction(&Instruction::I32Const(1));
            func.instruction(&Instruction::I32Add);
            func.instruction(&Instruction::LocalSet(local_i));

            func.instruction(&Instruction::Br(0));
            func.instruction(&Instruction::End); // end loop
            func.instruction(&Instruction::End); // end block
        }

        // === Step 4: Call original function (with linear memory pointer) ===
        func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
        func.instruction(&Instruction::Call(original_func_idx));
        // Result: i32 (linear memory slice pointer) on stack

        // === Step 5: Marshal output ===
        if let Some((ref out_sd, out_gc_idx, out_slice_gc, out_array_gc)) = output_slice_info {
            // Store the linear memory result pointer
            func.instruction(&Instruction::LocalSet(local_linear_slice_ptr));

            // Read output slice header: [data_ptr, len, cap]
            func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
            func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            func.instruction(&Instruction::LocalSet(local_linear_data_ptr));

            func.instruction(&Instruction::LocalGet(local_linear_slice_ptr));
            func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            func.instruction(&Instruction::LocalSet(local_output_row_count));

            // Rebuild GC output array from linear memory
            func.instruction(&Instruction::RefNull(HeapType::Concrete(out_gc_idx)));
            func.instruction(&Instruction::LocalGet(local_output_row_count));
            func.instruction(&Instruction::ArrayNew(out_array_gc));
            func.instruction(&Instruction::LocalSet(local_output_array));

            {
                let out_total_size = out_sd.total_size as i32;

                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::LocalSet(local_i));

                func.instruction(&Instruction::Block(BlockType::Empty));
                func.instruction(&Instruction::Loop(BlockType::Empty));

                func.instruction(&Instruction::LocalGet(local_i));
                func.instruction(&Instruction::LocalGet(local_output_row_count));
                func.instruction(&Instruction::I32GeU);
                func.instruction(&Instruction::BrIf(1));

                // Create new GC struct
                func.instruction(&Instruction::StructNewDefault(out_gc_idx));
                func.instruction(&Instruction::LocalSet(local_output_struct));

                // row_base = data_ptr + i * total_size
                func.instruction(&Instruction::LocalGet(local_linear_data_ptr));
                func.instruction(&Instruction::LocalGet(local_i));
                func.instruction(&Instruction::I32Const(out_total_size));
                func.instruction(&Instruction::I32Mul);
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::LocalSet(local_row_base));

                for f in &out_sd.fields {
                    func.instruction(&Instruction::LocalGet(local_output_struct));
                    func.instruction(&Instruction::LocalGet(local_row_base));
                    let mem_arg = |align: u32| MemArg { offset: f.offset as u64, align, memory_index: 0 };
                    match f.wasm_type {
                        WasmType::I32 => func.instruction(&Instruction::I32Load(mem_arg(2))),
                        WasmType::I64 => func.instruction(&Instruction::I64Load(mem_arg(3))),
                        WasmType::F32 => func.instruction(&Instruction::F32Load(mem_arg(2))),
                        WasmType::F64 => func.instruction(&Instruction::F64Load(mem_arg(3))),
                        WasmType::Ref(_) => {
                            func.instruction(&Instruction::Drop); // drop address
                            func.instruction(&Instruction::Drop); // drop struct ref
                            continue;
                        }
                    };
                    func.instruction(&Instruction::StructSet {
                        struct_type_index: out_gc_idx,
                        field_index: f.field_index,
                    });
                }

                // Store in GC array
                func.instruction(&Instruction::LocalGet(local_output_array));
                func.instruction(&Instruction::LocalGet(local_i));
                func.instruction(&Instruction::LocalGet(local_output_struct));
                func.instruction(&Instruction::ArraySet(out_array_gc));

                func.instruction(&Instruction::LocalGet(local_i));
                func.instruction(&Instruction::I32Const(1));
                func.instruction(&Instruction::I32Add);
                func.instruction(&Instruction::LocalSet(local_i));

                func.instruction(&Instruction::Br(0));
                func.instruction(&Instruction::End); // end loop
                func.instruction(&Instruction::End); // end block
            }

            // Build GC slice from the rebuilt array
            func.instruction(&Instruction::LocalGet(local_output_array));
            func.instruction(&Instruction::I32Const(0));
            func.instruction(&Instruction::LocalGet(local_output_row_count));
            func.instruction(&Instruction::LocalGet(local_output_row_count));
            func.instruction(&Instruction::StructNew(out_slice_gc));
            func.instruction(&Instruction::LocalSet(local_output_slice));

            let num_out_fields = out_sd.fields.len();
            let header_size = 8 + (num_out_fields as i32) * COLUMN_ENTRY_SIZE;

            // Allocate output column buffers and header
            // For simplicity: allocate header first, then fill in column buffers
            func.instruction(&Instruction::I32Const(header_size));
            func.instruction(&Instruction::Call(alloc_idx));
            func.instruction(&Instruction::LocalSet(local_output_header));

            // Write row_count and column_count
            func.instruction(&Instruction::LocalGet(local_output_header));
            func.instruction(&Instruction::LocalGet(local_output_row_count));
            func.instruction(&Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));

            func.instruction(&Instruction::LocalGet(local_output_header));
            func.instruction(&Instruction::I32Const(num_out_fields as i32));
            func.instruction(&Instruction::I32Store(MemArg {
                offset: 4,
                align: 2,
                memory_index: 0,
            }));

            // For each output field, allocate a buffer, loop to write values
            for (field_idx, sf) in out_sd.fields.iter().enumerate() {
                let col_entry_offset = (8 + (field_idx as i32) * COLUMN_ENTRY_SIZE) as u64;

                let is_string = matches!(sf.wasm_type, WasmType::Ref(idx) if Some(idx) == self.gc_builtin_types.go_string);
                let is_timestamp = matches!(sf.wasm_type, WasmType::Ref(idx) if self.is_time_struct(idx));
                let is_bool = matches!(&sf.field_type, Some(DefineType::Bool)) && sf.wasm_type == WasmType::I32;

                let (type_tag, elem_size) = if is_string {
                    (TYPE_TAG_STRING, 0) // strings handled specially
                } else if is_timestamp {
                    (TYPE_TAG_TIMESTAMP, 8) // i64 microseconds
                } else if is_bool {
                    (TYPE_TAG_BOOL, 1)
                } else {
                    match sf.wasm_type {
                        WasmType::I32 => (TYPE_TAG_I32, 4),
                        WasmType::I64 => (TYPE_TAG_I64, 8),
                        WasmType::F32 => (TYPE_TAG_F32, 4),
                        WasmType::F64 => (TYPE_TAG_F64, 8),
                        _ => (TYPE_TAG_I32, 4),
                    }
                };

                // Write type_tag
                func.instruction(&Instruction::LocalGet(local_output_header));
                func.instruction(&Instruction::I32Const(type_tag));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: col_entry_offset,
                    align: 2,
                    memory_index: 0,
                }));

                if is_string {
                    // For strings: we need two passes.
                    // Pass 1: compute total bytes needed
                    // Pass 2: write offsets and string data
                    // For simplicity in codegen, allocate a generous buffer.
                    // Allocate offsets + placeholder for data
                    let offsets_size_expr = |row_count_local: u32| {
                        // (row_count + 1) * 4
                        vec![
                            Instruction::LocalGet(row_count_local),
                            Instruction::I32Const(1),
                            Instruction::I32Add,
                            Instruction::I32Const(4),
                            Instruction::I32Mul,
                        ]
                    };

                    // First pass: compute total string bytes
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::LocalSet(local_str_total_bytes));

                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::LocalSet(local_i));

                    func.instruction(&Instruction::Block(BlockType::Empty));
                    func.instruction(&Instruction::Loop(BlockType::Empty));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::LocalGet(local_output_row_count));
                    func.instruction(&Instruction::I32GeU);
                    func.instruction(&Instruction::BrIf(1));

                    // Get string from output struct
                    func.instruction(&Instruction::LocalGet(local_output_array));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::ArrayGet(out_array_gc));
                    func.instruction(&Instruction::LocalSet(local_output_struct));
                    func.instruction(&Instruction::LocalGet(local_output_struct));
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: out_gc_idx,
                        field_index: sf.field_index,
                    });
                    // This is a GC string ref. Get its length (field 1 of GoString)
                    let go_string_gc = self.gc_builtin_types.go_string.unwrap();
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: go_string_gc,
                        field_index: 1, // len
                    });
                    func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalSet(local_str_total_bytes));

                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalSet(local_i));
                    func.instruction(&Instruction::Br(0));
                    func.instruction(&Instruction::End);
                    func.instruction(&Instruction::End);

                    // Allocate offsets + string data buffer
                    for instr in offsets_size_expr(local_output_row_count) {
                        func.instruction(&instr);
                    }
                    func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::Call(alloc_idx));
                    func.instruction(&Instruction::LocalSet(local_col_data_ptr));

                    // Write data_ptr to header
                    func.instruction(&Instruction::LocalGet(local_output_header));
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: col_entry_offset + 4,
                        align: 2,
                        memory_index: 0,
                    }));

                    // null_bitmap = 0
                    func.instruction(&Instruction::LocalGet(local_output_header));
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: col_entry_offset + 8,
                        align: 2,
                        memory_index: 0,
                    }));

                    // Second pass: write offsets and string bytes
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::LocalSet(local_str_total_bytes)); // byte offset
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::LocalSet(local_i));

                    func.instruction(&Instruction::Block(BlockType::Empty));
                    func.instruction(&Instruction::Loop(BlockType::Empty));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::LocalGet(local_output_row_count));
                    func.instruction(&Instruction::I32GeU);
                    func.instruction(&Instruction::BrIf(1));

                    // Write offsets[i] = byte_offset
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(4));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));

                    // Get string from output struct, extract byte_array and len
                    func.instruction(&Instruction::LocalGet(local_output_array));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::ArrayGet(out_array_gc));
                    func.instruction(&Instruction::LocalSet(local_output_struct));

                    // Get string length
                    func.instruction(&Instruction::LocalGet(local_output_struct));
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: out_gc_idx,
                        field_index: sf.field_index,
                    });
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: go_string_gc,
                        field_index: 1, // len
                    });
                    func.instruction(&Instruction::LocalSet(local_temp_i32)); // str_len

                    // Get byte array ref
                    func.instruction(&Instruction::LocalGet(local_output_struct));
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: out_gc_idx,
                        field_index: sf.field_index,
                    });
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: go_string_gc,
                        field_index: 0, // byte_array ref
                    });
                    func.instruction(&Instruction::LocalSet(local_byte_array_ref));

                    // Copy bytes: for j = 0; j < str_len; j++
                    //   mem[col_data_ptr + (row_count+1)*4 + byte_offset + j] = byte_array[j]
                    func.instruction(&Instruction::LocalGet(local_temp_i32));
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::I32GtU);
                    func.instruction(&Instruction::If(BlockType::Empty));
                    {
                        let ba_gc = self.gc_builtin_types.byte_array.unwrap();

                        func.instruction(&Instruction::I32Const(0));
                        func.instruction(&Instruction::LocalSet(local_j));

                        func.instruction(&Instruction::Block(BlockType::Empty));
                        func.instruction(&Instruction::Loop(BlockType::Empty));

                        // break if j >= str_len
                        func.instruction(&Instruction::LocalGet(local_j));
                        func.instruction(&Instruction::LocalGet(local_temp_i32));
                        func.instruction(&Instruction::I32GeU);
                        func.instruction(&Instruction::BrIf(1));

                        // dest addr = col_data_ptr + (row_count+1)*4 + byte_offset + j
                        func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                        func.instruction(&Instruction::LocalGet(local_output_row_count));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::I32Const(4));
                        func.instruction(&Instruction::I32Mul);
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalGet(local_j));
                        func.instruction(&Instruction::I32Add);

                        // byte value = byte_array[j]
                        func.instruction(&Instruction::LocalGet(local_byte_array_ref));
                        func.instruction(&Instruction::LocalGet(local_j));
                        func.instruction(&Instruction::ArrayGetU(ba_gc));

                        // store byte
                        func.instruction(&Instruction::I32Store8(MemArg {
                            offset: 0,
                            align: 0,
                            memory_index: 0,
                        }));

                        // j++
                        func.instruction(&Instruction::LocalGet(local_j));
                        func.instruction(&Instruction::I32Const(1));
                        func.instruction(&Instruction::I32Add);
                        func.instruction(&Instruction::LocalSet(local_j));

                        func.instruction(&Instruction::Br(0));
                        func.instruction(&Instruction::End); // loop
                        func.instruction(&Instruction::End); // block
                    }
                    func.instruction(&Instruction::End); // end if

                    // Update byte_offset
                    func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                    func.instruction(&Instruction::LocalGet(local_temp_i32)); // str_len
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalSet(local_str_total_bytes));

                    // i++
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalSet(local_i));
                    func.instruction(&Instruction::Br(0));
                    func.instruction(&Instruction::End); // loop
                    func.instruction(&Instruction::End); // block

                    // Write final offset: offsets[row_count] = total_bytes
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_output_row_count));
                    func.instruction(&Instruction::I32Const(4));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalGet(local_str_total_bytes));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                } else {
                    // Non-string fields: allocate buffer and loop to write values
                    let buf_size_multiplier = elem_size;
                    func.instruction(&Instruction::LocalGet(local_output_row_count));
                    func.instruction(&Instruction::I32Const(buf_size_multiplier));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::Call(alloc_idx));
                    func.instruction(&Instruction::LocalSet(local_col_data_ptr));

                    // Write data_ptr to header
                    func.instruction(&Instruction::LocalGet(local_output_header));
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: col_entry_offset + 4,
                        align: 2,
                        memory_index: 0,
                    }));

                    // Write null_bitmap_ptr = 0
                    func.instruction(&Instruction::LocalGet(local_output_header));
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::I32Store(MemArg {
                        offset: col_entry_offset + 8,
                        align: 2,
                        memory_index: 0,
                    }));

                    // Loop: write each value
                    func.instruction(&Instruction::I32Const(0));
                    func.instruction(&Instruction::LocalSet(local_i));

                    func.instruction(&Instruction::Block(BlockType::Empty));
                    func.instruction(&Instruction::Loop(BlockType::Empty));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::LocalGet(local_output_row_count));
                    func.instruction(&Instruction::I32GeU);
                    func.instruction(&Instruction::BrIf(1));

                    // Get struct from output array
                    func.instruction(&Instruction::LocalGet(local_output_array));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::ArrayGet(out_array_gc));
                    func.instruction(&Instruction::LocalSet(local_output_struct));

                    // dest address = col_data_ptr + i * elem_size
                    func.instruction(&Instruction::LocalGet(local_col_data_ptr));
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(buf_size_multiplier));
                    func.instruction(&Instruction::I32Mul);
                    func.instruction(&Instruction::I32Add);

                    // Get field value from struct
                    func.instruction(&Instruction::LocalGet(local_output_struct));
                    func.instruction(&Instruction::StructGet {
                        struct_type_index: out_gc_idx,
                        field_index: sf.field_index,
                    });

                    // Store
                    match sf.wasm_type {
                        WasmType::I32 if is_bool => {
                            func.instruction(&Instruction::I32Store8(MemArg {
                                offset: 0,
                                align: 0,
                                memory_index: 0,
                            }));
                        }
                        WasmType::I32 => {
                            func.instruction(&Instruction::I32Store(MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        WasmType::I64 => {
                            func.instruction(&Instruction::I64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        WasmType::F32 => {
                            func.instruction(&Instruction::F32Store(MemArg {
                                offset: 0,
                                align: 2,
                                memory_index: 0,
                            }));
                        }
                        WasmType::F64 => {
                            func.instruction(&Instruction::F64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        WasmType::Ref(ref_idx) if is_timestamp => {
                            // Stack: [dest_addr, time.Time GC ref]
                            // Extract microseconds: (ext - unixToInternal) * 1e6 + (wall & nsecMask) / 1e3
                            const UNIX_TO_INTERNAL: i64 = 62135596800;
                            const NSEC_MASK: i64 = (1 << 30) - 1;

                            func.instruction(&Instruction::LocalSet(local_time_ref));
                            // Stack: [dest_addr]

                            // sec_usec = (ext - unixToInternal) * 1_000_000
                            func.instruction(&Instruction::LocalGet(local_time_ref));
                            func.instruction(&Instruction::StructGet {
                                struct_type_index: ref_idx,
                                field_index: 1, // ext
                            });
                            func.instruction(&Instruction::I64Const(UNIX_TO_INTERNAL));
                            func.instruction(&Instruction::I64Sub);
                            func.instruction(&Instruction::I64Const(1_000_000));
                            func.instruction(&Instruction::I64Mul);

                            // nsec_usec = (wall & nsecMask) / 1_000
                            func.instruction(&Instruction::LocalGet(local_time_ref));
                            func.instruction(&Instruction::StructGet {
                                struct_type_index: ref_idx,
                                field_index: 0, // wall
                            });
                            func.instruction(&Instruction::I64Const(NSEC_MASK));
                            func.instruction(&Instruction::I64And);
                            func.instruction(&Instruction::I64Const(1_000));
                            func.instruction(&Instruction::I64DivS);

                            func.instruction(&Instruction::I64Add);
                            // Stack: [dest_addr, i64 usec]

                            func.instruction(&Instruction::I64Store(MemArg {
                                offset: 0,
                                align: 3,
                                memory_index: 0,
                            }));
                        }
                        _ => {
                            func.instruction(&Instruction::Drop);
                            func.instruction(&Instruction::Drop);
                        }
                    }

                    // i++
                    func.instruction(&Instruction::LocalGet(local_i));
                    func.instruction(&Instruction::I32Const(1));
                    func.instruction(&Instruction::I32Add);
                    func.instruction(&Instruction::LocalSet(local_i));

                    func.instruction(&Instruction::Br(0)); // continue
                    func.instruction(&Instruction::End); // loop
                    func.instruction(&Instruction::End); // block
                }

                // Write reserved = 0
                func.instruction(&Instruction::LocalGet(local_output_header));
                func.instruction(&Instruction::I32Const(0));
                func.instruction(&Instruction::I32Store(MemArg {
                    offset: col_entry_offset + 12,
                    align: 2,
                    memory_index: 0,
                }));
            }

            // Return output header pointer
            func.instruction(&Instruction::LocalGet(local_output_header));
        } else {
            // No output struct, return 0
            func.instruction(&Instruction::Drop); // drop whatever the function returned
            func.instruction(&Instruction::I32Const(0));
        }

        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.export_section
            .export(&wrapper_name, ExportKind::Func, func_idx);

        self.functions.push(super::FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: wrapper_name,
            params: vec![("header_ptr".to_string(), WasmType::I32)],
            results: vec![WasmType::I32],
            result_define_types: vec![DefineType::Int32],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });

        Ok(())
    }

    fn is_time_struct(&self, gc_idx: u32) -> bool {
        self.gc_struct_types.get("time.Time") == Some(&gc_idx)
    }

    #[allow(dead_code)]
    fn find_func_idx(&self, name: &str) -> Option<u32> {
        self.functions
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.wasm_func_idx)
    }
}
