use super::*;

impl WasmCompiler {
    pub(crate) fn emit_string_eq_from_locals(
        &self,
        tag_ptr: u32,
        tag_len: u32,
        case_ptr: u32,
        case_len: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            // GC mode: tag_ptr and case_ptr hold GoString ref locals
            let tag_arr = locals.add_local(
                &format!("__gc_seq_ta_{}", locals.locals.len()),
                Self::gc_ref_val_type(byte_array_idx),
            );
            let case_arr = locals.add_local(
                &format!("__gc_seq_ca_{}", locals.locals.len()),
                Self::gc_ref_val_type(byte_array_idx),
            );
            let tag_len_l = locals.add_local(
                &format!("__gc_seq_tl_{}", locals.locals.len()),
                ValType::I32,
            );
            let case_len_l = locals.add_local(
                &format!("__gc_seq_cl_{}", locals.locals.len()),
                ValType::I32,
            );
            let result = locals.add_local(
                &format!("__gc_seq_r_{}", locals.locals.len()),
                ValType::I32,
            );
            let idx = locals.add_local(
                &format!("__gc_seq_i_{}", locals.locals.len()),
                ValType::I32,
            );

            out.push(Instruction::LocalGet(tag_ptr));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(tag_len_l));
            out.push(Instruction::LocalGet(tag_ptr));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(tag_arr));

            out.push(Instruction::LocalGet(case_ptr));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(case_len_l));
            out.push(Instruction::LocalGet(case_ptr));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(case_arr));

            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::LocalGet(tag_len_l));
            out.push(Instruction::LocalGet(case_len_l));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Else);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::LocalGet(tag_len_l));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));
            out.push(Instruction::LocalGet(tag_arr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::ArrayGetU(byte_array_idx));
            out.push(Instruction::LocalGet(case_arr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::ArrayGetU(byte_array_idx));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block
            out.push(Instruction::End); // else
            out.push(Instruction::LocalGet(result));
        } else {
            let result = locals.add_local(
                &format!("__sw_seq_r_{}", locals.locals.len()),
                ValType::I32,
            );
            let idx = locals.add_local(
                &format!("__sw_seq_i_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::LocalGet(tag_len));
            out.push(Instruction::LocalGet(case_len));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Else);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::LocalGet(tag_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));
            out.push(Instruction::LocalGet(tag_ptr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(case_ptr));
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result));
            out.push(Instruction::Br(2));
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(idx));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block
            out.push(Instruction::End); // else
            out.push(Instruction::LocalGet(result));
        }
    }

    pub(crate) fn emit_string_concat(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(lhs, out, locals)?;
        self.compile_expression(rhs, out, locals)?;

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            let s2_ref = locals.add_local("__gc_scat_s2", go_str_vt);
            let s1_ref = locals.add_local("__gc_scat_s1", go_str_vt);
            out.push(Instruction::LocalSet(s2_ref));
            out.push(Instruction::LocalSet(s1_ref));

            let arr1 = locals.add_local("__gc_scat_a1", arr_vt);
            let len1 = locals.add_local("__gc_scat_l1", ValType::I32);
            let arr2 = locals.add_local("__gc_scat_a2", arr_vt);
            let len2 = locals.add_local("__gc_scat_l2", ValType::I32);
            let total_len = locals.add_local("__gc_scat_tl", ValType::I32);
            let new_arr = locals.add_local("__gc_scat_na", arr_vt);

            out.push(Instruction::LocalGet(s1_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(len1));
            out.push(Instruction::LocalGet(s1_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(arr1));

            out.push(Instruction::LocalGet(s2_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(len2));
            out.push(Instruction::LocalGet(s2_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(arr2));

            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalTee(total_len));

            // Overflow check
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            // Create new ByteArray
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::ArrayNew(byte_array_idx));
            out.push(Instruction::LocalSet(new_arr));

            // Copy first string: array.copy dst dst_offset src src_offset length
            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(arr1));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::ArrayCopy {
                array_type_index_dst: byte_array_idx,
                array_type_index_src: byte_array_idx,
            });

            // Copy second string
            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(arr2));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::ArrayCopy {
                array_type_index_dst: byte_array_idx,
                array_type_index_src: byte_array_idx,
            });

            // Create GoString struct
            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::StructNew(go_string_idx));
        } else {
            let len2 = locals.add_local("__scat_len2", ValType::I32);
            let ptr2 = locals.add_local("__scat_ptr2", ValType::I32);
            let len1 = locals.add_local("__scat_len1", ValType::I32);
            let ptr1 = locals.add_local("__scat_ptr1", ValType::I32);

            out.push(Instruction::LocalSet(len2));
            out.push(Instruction::LocalSet(ptr2));
            out.push(Instruction::LocalSet(len1));
            out.push(Instruction::LocalSet(ptr1));

            let total_len = locals.add_local("__scat_total", ValType::I32);
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalTee(total_len));

            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let new_ptr = locals.add_local("__scat_new", ValType::I32);
            out.push(Instruction::LocalSet(new_ptr));

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(ptr1));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::MemoryCopy {
                dst_mem: 0,
                src_mem: 0,
            });

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(ptr2));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::MemoryCopy {
                dst_mem: 0,
                src_mem: 0,
            });

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(total_len));
        }

        Ok(())
    }

    pub(crate) fn emit_expr_to_string_on_stack(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if self.is_string_expr(expr, locals) {
            self.compile_expression(expr, out, locals)?;
            return Ok(());
        }

        if let ast::Expression::Ident(id) = expr {
            if id.name == "true" || id.name == "false" {
                self.emit_bool_to_string(id.name == "true", out, locals)?;
                return Ok(());
            }
        }

        let vt = self.infer_val_type(expr, locals);
        self.compile_expression(expr, out, locals)?;
        match vt {
            ValType::I64 | ValType::I32 => {
                if vt == ValType::I32 {
                    out.push(Instruction::I64ExtendI32S);
                }
                self.emit_i64_to_string(out, locals)?;
            }
            ValType::F64 => {
                self.emit_f64_to_string(out, locals)?;
            }
            ValType::F32 => {
                out.push(Instruction::F64PromoteF32);
                self.emit_f64_to_string(out, locals)?;
            }
            _ => {
                out.push(Instruction::Drop);
                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                    let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::ArrayNew(byte_array_idx));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::StructNew(go_string_idx));
                } else {
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::I32Const(0));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn emit_bool_to_string(
        &mut self,
        val: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let s = if val { b"true" as &[u8] } else { b"false" as &[u8] };

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            for &byte in s.iter() {
                out.push(Instruction::I32Const(byte as i32));
            }
            out.push(Instruction::ArrayNewFixed {
                array_type_index: byte_array_idx,
                array_size: s.len() as u32,
            });
            out.push(Instruction::I32Const(s.len() as i32));
            out.push(Instruction::StructNew(go_string_idx));
        } else {
            let ptr_local = locals.add_local("__bts_ptr", ValType::I32);
            out.push(Instruction::I32Const(s.len() as i32));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(ptr_local));
            for (i, &byte) in s.iter().enumerate() {
                out.push(Instruction::LocalGet(ptr_local));
                out.push(Instruction::I32Const(byte as i32));
                out.push(Instruction::I32Store8(MemArg { offset: i as u64, align: 0, memory_index: 0 }));
            }
            out.push(Instruction::LocalGet(ptr_local));
            out.push(Instruction::I32Const(s.len() as i32));
        }
        Ok(())
    }

    pub(crate) fn emit_append_newline(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            let src_ref = locals.add_local("__gc_nl_sr", go_str_vt);
            let src_arr = locals.add_local("__gc_nl_sa", arr_vt);
            let src_len = locals.add_local("__gc_nl_sl", ValType::I32);
            let new_len = locals.add_local("__gc_nl_nl", ValType::I32);
            let new_arr = locals.add_local("__gc_nl_na", arr_vt);

            out.push(Instruction::LocalSet(src_ref));

            out.push(Instruction::LocalGet(src_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(src_arr));
            out.push(Instruction::LocalGet(src_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(src_len));

            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(new_len));

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::ArrayNew(byte_array_idx));
            out.push(Instruction::LocalSet(new_arr));

            // Copy source bytes
            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(src_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::ArrayCopy {
                array_type_index_dst: byte_array_idx,
                array_type_index_src: byte_array_idx,
            });

            // Set newline at index src_len
            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::I32Const(b'\n' as i32));
            out.push(Instruction::ArraySet(byte_array_idx));

            out.push(Instruction::LocalGet(new_arr));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::StructNew(go_string_idx));
        } else {
            let src_len = locals.add_local("__nl_src_len", ValType::I32);
            let src_ptr = locals.add_local("__nl_src_ptr", ValType::I32);
            out.push(Instruction::LocalSet(src_len));
            out.push(Instruction::LocalSet(src_ptr));

            let new_len = locals.add_local("__nl_new_len", ValType::I32);
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(new_len));

            let new_ptr = locals.add_local("__nl_new_ptr", ValType::I32);
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_ptr));

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(src_ptr));
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(b'\n' as i32));
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            out.push(Instruction::LocalGet(new_ptr));
            out.push(Instruction::LocalGet(new_len));
        }
        Ok(())
    }

    pub(crate) fn emit_string_compare(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        op: Operator,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(lhs, out, locals)?;
        self.compile_expression(rhs, out, locals)?;

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            let s2_ref = locals.add_local(&format!("__gc_scmp_s2_{}", locals.locals.len()), go_str_vt);
            let s1_ref = locals.add_local(&format!("__gc_scmp_s1_{}", locals.locals.len()), go_str_vt);
            out.push(Instruction::LocalSet(s2_ref));
            out.push(Instruction::LocalSet(s1_ref));

            let arr1 = locals.add_local(&format!("__gc_scmp_a1_{}", locals.locals.len()), arr_vt);
            let len1 = locals.add_local(&format!("__gc_scmp_l1_{}", locals.locals.len()), ValType::I32);
            let arr2 = locals.add_local(&format!("__gc_scmp_a2_{}", locals.locals.len()), arr_vt);
            let len2 = locals.add_local(&format!("__gc_scmp_l2_{}", locals.locals.len()), ValType::I32);

            out.push(Instruction::LocalGet(s1_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(len1));
            out.push(Instruction::LocalGet(s1_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(arr1));

            out.push(Instruction::LocalGet(s2_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(len2));
            out.push(Instruction::LocalGet(s2_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(arr2));

            if op == Operator::Equal || op == Operator::NotEqual {
                let result = locals.add_local(&format!("__gc_scmp_res_{}", locals.locals.len()), ValType::I32);
                let idx = locals.add_local(&format!("__gc_scmp_idx_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(result));

                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(result));
                out.push(Instruction::Else);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));
                out.push(Instruction::LocalGet(arr1));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::LocalGet(arr2));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(result));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block
                out.push(Instruction::End); // else

                out.push(Instruction::LocalGet(result));
                if op == Operator::NotEqual {
                    out.push(Instruction::I32Eqz);
                }
            } else {
                let cmp = locals.add_local(&format!("__gc_scmp_cmp_{}", locals.locals.len()), ValType::I32);
                let idx = locals.add_local(&format!("__gc_scmp_idx_{}", locals.locals.len()), ValType::I32);
                let min_len = locals.add_local(&format!("__gc_scmp_min_{}", locals.locals.len()), ValType::I32);
                let b1 = locals.add_local(&format!("__gc_scmp_b1_{}", locals.locals.len()), ValType::I32);
                let b2 = locals.add_local(&format!("__gc_scmp_b2_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::I32LeU);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(min_len));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(idx));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::LocalGet(min_len));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(arr1));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::LocalSet(b1));
                out.push(Instruction::LocalGet(arr2));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::LocalSet(b2));

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                {
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(-1i32));
                    out.push(Instruction::LocalSet(cmp));
                    out.push(Instruction::Else);
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::I32GtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(cmp));
                    out.push(Instruction::End);
                    out.push(Instruction::End);
                }
                out.push(Instruction::End);

                match op {
                    Operator::Less => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(-1i32));
                        out.push(Instruction::I32Eq);
                    }
                    Operator::Greater => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::I32Eq);
                    }
                    Operator::LessEqual => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::I32Ne);
                    }
                    Operator::GreaterEqual => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(-1i32));
                        out.push(Instruction::I32Ne);
                    }
                    _ => {
                        return Err(Error::InternalError(format!(
                            "unsupported operator {:?} for string comparison",
                            op
                        )));
                    }
                }
            }
        } else {
            let len2 = locals.add_local(&format!("__scmp_len2_{}", locals.locals.len()), ValType::I32);
            let ptr2 = locals.add_local(&format!("__scmp_ptr2_{}", locals.locals.len()), ValType::I32);
            let len1 = locals.add_local(&format!("__scmp_len1_{}", locals.locals.len()), ValType::I32);
            let ptr1 = locals.add_local(&format!("__scmp_ptr1_{}", locals.locals.len()), ValType::I32);

            out.push(Instruction::LocalSet(len2));
            out.push(Instruction::LocalSet(ptr2));
            out.push(Instruction::LocalSet(len1));
            out.push(Instruction::LocalSet(ptr1));

            if op == Operator::Equal || op == Operator::NotEqual {
                if let Some(streq_idx) = self.rt_streq_func_idx {
                    out.push(Instruction::LocalGet(ptr1));
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(ptr2));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::Call(streq_idx));
                    if op == Operator::NotEqual {
                        out.push(Instruction::I32Eqz);
                    }
                } else {
                    let result = locals.add_local(&format!("__scmp_res_{}", locals.locals.len()), ValType::I32);
                    let idx = locals.add_local(&format!("__scmp_idx_{}", locals.locals.len()), ValType::I32);

                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(result));

                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(result));
                    out.push(Instruction::Else);
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(idx));
                    out.push(Instruction::Block(BlockType::Empty));
                    out.push(Instruction::Loop(BlockType::Empty));
                    out.push(Instruction::LocalGet(idx));
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::I32GeU);
                    out.push(Instruction::BrIf(1));
                    out.push(Instruction::LocalGet(ptr1));
                    out.push(Instruction::LocalGet(idx));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    out.push(Instruction::LocalGet(ptr2));
                    out.push(Instruction::LocalGet(idx));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(result));
                    out.push(Instruction::Br(2));
                    out.push(Instruction::End);
                    out.push(Instruction::LocalGet(idx));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalSet(idx));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                    out.push(Instruction::End); // else

                    out.push(Instruction::LocalGet(result));
                    if op == Operator::NotEqual {
                        out.push(Instruction::I32Eqz);
                    }
                }
            } else {
                if let Some(strcmp_idx) = self.rt_strcmp_func_idx {
                    out.push(Instruction::LocalGet(ptr1));
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(ptr2));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::Call(strcmp_idx));

                    match op {
                        Operator::Less => {
                            out.push(Instruction::I32Const(-1i32));
                            out.push(Instruction::I32Eq);
                        }
                        Operator::Greater => {
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Eq);
                        }
                        Operator::LessEqual => {
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Ne);
                        }
                        Operator::GreaterEqual => {
                            out.push(Instruction::I32Const(-1i32));
                            out.push(Instruction::I32Ne);
                        }
                        _ => {
                            return Err(Error::InternalError(format!(
                                "unsupported operator {:?} for string comparison",
                                op
                            )));
                        }
                    }
                } else {
                let cmp = locals.add_local(&format!("__scmp_cmp_{}", locals.locals.len()), ValType::I32);
                let idx = locals.add_local(&format!("__scmp_idx_{}", locals.locals.len()), ValType::I32);
                let min_len = locals.add_local(&format!("__scmp_min_{}", locals.locals.len()), ValType::I32);
                let b1 = locals.add_local(&format!("__scmp_b1_{}", locals.locals.len()), ValType::I32);
                let b2 = locals.add_local(&format!("__scmp_b2_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::LocalGet(len1));
                out.push(Instruction::LocalGet(len2));
                out.push(Instruction::I32LeU);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(min_len));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(idx));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::LocalGet(min_len));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(ptr1));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalSet(b1));
                out.push(Instruction::LocalGet(ptr2));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalSet(b2));

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                {
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(-1i32));
                    out.push(Instruction::LocalSet(cmp));
                    out.push(Instruction::Else);
                    out.push(Instruction::LocalGet(len1));
                    out.push(Instruction::LocalGet(len2));
                    out.push(Instruction::I32GtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(cmp));
                    out.push(Instruction::End);
                    out.push(Instruction::End);
                }
                out.push(Instruction::End);

                match op {
                    Operator::Less => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(-1i32));
                        out.push(Instruction::I32Eq);
                    }
                    Operator::Greater => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::I32Eq);
                    }
                    Operator::LessEqual => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::I32Ne);
                    }
                    Operator::GreaterEqual => {
                        out.push(Instruction::LocalGet(cmp));
                        out.push(Instruction::I32Const(-1i32));
                        out.push(Instruction::I32Ne);
                    }
                    _ => {
                        return Err(Error::InternalError(format!(
                            "unsupported operator {:?} for string comparison",
                            op
                        )));
                    }
                }
                } // else (fallback inline strcmp)
            }
        }

        Ok(())
    }

    pub(crate) fn emit_string_min_max(
        &mut self,
        args: &[ast::Expression],
        is_min: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            let best_ref = locals.add_local(&format!("__gc_smm_br_{}", locals.locals.len()), go_str_vt);
            self.compile_expression(&args[0], out, locals)?;
            out.push(Instruction::LocalSet(best_ref));

            for arg in &args[1..] {
                let cur_ref = locals.add_local(&format!("__gc_smm_cr_{}", locals.locals.len()), go_str_vt);
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::LocalSet(cur_ref));

                let best_arr = locals.add_local(&format!("__gc_smm_ba_{}", locals.locals.len()), arr_vt);
                let best_len = locals.add_local(&format!("__gc_smm_bl_{}", locals.locals.len()), ValType::I32);
                let cur_arr = locals.add_local(&format!("__gc_smm_ca_{}", locals.locals.len()), arr_vt);
                let cur_len = locals.add_local(&format!("__gc_smm_cl_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::LocalGet(best_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                out.push(Instruction::LocalSet(best_len));
                out.push(Instruction::LocalGet(best_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                out.push(Instruction::LocalSet(best_arr));

                out.push(Instruction::LocalGet(cur_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                out.push(Instruction::LocalSet(cur_len));
                out.push(Instruction::LocalGet(cur_ref));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                out.push(Instruction::LocalSet(cur_arr));

                let cmp = locals.add_local(&format!("__gc_smm_cmp_{}", locals.locals.len()), ValType::I32);
                let idx = locals.add_local(&format!("__gc_smm_i_{}", locals.locals.len()), ValType::I32);
                let min_len_l = locals.add_local(&format!("__gc_smm_ml_{}", locals.locals.len()), ValType::I32);
                let b1 = locals.add_local(&format!("__gc_smm_b1_{}", locals.locals.len()), ValType::I32);
                let b2 = locals.add_local(&format!("__gc_smm_b2_{}", locals.locals.len()), ValType::I32);

                // min_len = min(best_len, cur_len)
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32LeU);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(min_len_l));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(idx));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::LocalGet(min_len_l));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(best_arr));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::LocalSet(b1));
                out.push(Instruction::LocalGet(cur_arr));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::LocalSet(b2));

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                // If bytes equal, compare lengths
                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Else);
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::End);
                out.push(Instruction::End);
                out.push(Instruction::End);

                let should_replace = if is_min {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(0));
                    Instruction::I32GtS
                } else {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(0));
                    Instruction::I32LtS
                };
                out.push(should_replace);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(cur_ref));
                out.push(Instruction::LocalSet(best_ref));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(best_ref));
        } else {
            let best_ptr = locals.add_local(&format!("__smm_bp_{}", locals.locals.len()), ValType::I32);
            let best_len = locals.add_local(&format!("__smm_bl_{}", locals.locals.len()), ValType::I32);
            self.compile_expression(&args[0], out, locals)?;
            out.push(Instruction::LocalSet(best_len));
            out.push(Instruction::LocalSet(best_ptr));

            for arg in &args[1..] {
                let cur_ptr = locals.add_local(&format!("__smm_cp_{}", locals.locals.len()), ValType::I32);
                let cur_len = locals.add_local(&format!("__smm_cl_{}", locals.locals.len()), ValType::I32);
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::LocalSet(cur_len));
                out.push(Instruction::LocalSet(cur_ptr));

                let cmp = locals.add_local(&format!("__smm_cmp_{}", locals.locals.len()), ValType::I32);
                let idx = locals.add_local(&format!("__smm_i_{}", locals.locals.len()), ValType::I32);
                let min_len_l = locals.add_local(&format!("__smm_ml_{}", locals.locals.len()), ValType::I32);
                let b1 = locals.add_local(&format!("__smm_b1_{}", locals.locals.len()), ValType::I32);
                let b2 = locals.add_local(&format!("__smm_b2_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32LeU);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(min_len_l));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(idx));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::LocalGet(min_len_l));
                out.push(Instruction::I32GeU);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(best_ptr));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalSet(b1));
                out.push(Instruction::LocalGet(cur_ptr));
                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalSet(b2));

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(b1));
                out.push(Instruction::LocalGet(b2));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Br(2));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(idx));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(idx));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32LtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(-1i32));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::Else);
                out.push(Instruction::LocalGet(best_len));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::I32GtU);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(cmp));
                out.push(Instruction::End);
                out.push(Instruction::End);
                out.push(Instruction::End);

                let should_replace = if is_min {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(0));
                    Instruction::I32GtS
                } else {
                    out.push(Instruction::LocalGet(cmp));
                    out.push(Instruction::I32Const(0));
                    Instruction::I32LtS
                };
                out.push(should_replace);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(cur_ptr));
                out.push(Instruction::LocalSet(best_ptr));
                out.push(Instruction::LocalGet(cur_len));
                out.push(Instruction::LocalSet(best_len));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(best_ptr));
            out.push(Instruction::LocalGet(best_len));
        }
        Ok(())
    }

    /// Convert a GoString ref on the stack to (ptr, len) in linear memory.
    pub(crate) fn emit_gc_string_to_linear(
        &mut self,
        go_string_idx: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
        let go_str_vt = Self::gc_ref_val_type(go_string_idx);
        let arr_vt = Self::gc_ref_val_type(byte_array_idx);

        let src_ref = locals.add_local(&format!("__g2l_sr_{}", locals.locals.len()), go_str_vt);
        let src_arr = locals.add_local(&format!("__g2l_sa_{}", locals.locals.len()), arr_vt);
        let src_len = locals.add_local(&format!("__g2l_sl_{}", locals.locals.len()), ValType::I32);
        let dst_ptr = locals.add_local(&format!("__g2l_dp_{}", locals.locals.len()), ValType::I32);
        let i_local = locals.add_local(&format!("__g2l_i_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(src_ref));

        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
        out.push(Instruction::LocalSet(src_arr));

        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(dst_ptr));

        // Copy bytes from GC array to linear memory
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(dst_ptr));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_arr));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::ArrayGetU(byte_array_idx));
        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        out.push(Instruction::LocalGet(dst_ptr));
        out.push(Instruction::LocalGet(src_len));
        Ok(())
    }

    /// Convert (ptr, len) on the stack to a GoString GC ref.
    pub(crate) fn emit_linear_to_gc_string(
        &mut self,
        go_string_idx: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
        let arr_vt = Self::gc_ref_val_type(byte_array_idx);

        let src_len = locals.add_local(&format!("__l2g_sl_{}", locals.locals.len()), ValType::I32);
        let src_ptr = locals.add_local(&format!("__l2g_sp_{}", locals.locals.len()), ValType::I32);
        let gc_arr = locals.add_local(&format!("__l2g_ga_{}", locals.locals.len()), arr_vt);
        let i_local = locals.add_local(&format!("__l2g_i_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::ArrayNew(byte_array_idx));
        out.push(Instruction::LocalSet(gc_arr));

        // Copy bytes from linear memory to GC array
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(gc_arr));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::ArraySet(byte_array_idx));

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        out.push(Instruction::LocalGet(gc_arr));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::StructNew(go_string_idx));
        Ok(())
    }

    pub(crate) fn emit_i64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let func_idx = self.rt_i64_to_str_func_idx.ok_or_else(|| {
            Error::InternalError("rt_i64_to_str host import not registered".to_string())
        })?;
        out.push(Instruction::Call(func_idx));

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
        }

        Ok(())
    }

    pub(crate) fn emit_f64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let func_idx = self.rt_f64_to_str_func_idx.ok_or_else(|| {
            Error::InternalError("rt_f64_to_str host import not registered".to_string())
        })?;
        out.push(Instruction::Call(func_idx));

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
        }

        Ok(())
    }

    // End of string conversion functions - old WASM codegen removed in favor of host imports
    // _end_of_impl_marker
    #[cfg(any())]
    fn _placeholder_do_not_call() {
        let _fval = 0u32;
        let _frac_int = 0u32;
        let _int_ptr = 0u32;
        let _int_len = 0u32;
        let frac_buf = locals.add_local(&format!("__ftos_fb_{}", locals.locals.len()), ValType::I32);
        let frac_pos = locals.add_local(&format!("__ftos_fp_{}", locals.locals.len()), ValType::I32);
        let final_ptr = locals.add_local(&format!("__ftos_rp_{}", locals.locals.len()), ValType::I32);
        let total_len = locals.add_local(&format!("__ftos_tl_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(fval));

        // Check sign
        out.push(Instruction::LocalGet(fval));
        out.push(Instruction::F64Const(0.0_f64.into()));
        out.push(Instruction::F64Lt);
        out.push(Instruction::LocalSet(is_neg));

        out.push(Instruction::LocalGet(fval));
        out.push(Instruction::F64Abs);
        out.push(Instruction::LocalSet(abs_val));

        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::F64Floor);
        out.push(Instruction::I64TruncF64S);
        out.push(Instruction::LocalSet(int_part));

        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::LocalGet(int_part));
        out.push(Instruction::F64ConvertI64S);
        out.push(Instruction::F64Sub);
        out.push(Instruction::LocalSet(frac_val));

        // Convert int_part to string using existing helper
        out.push(Instruction::LocalGet(int_part));
        self.emit_i64_to_string(out, locals)?;

        // emit_i64_to_string now produces either (ptr, len) or GoString ref
        // We need to handle both cases for extracting int_ptr/int_len
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            let go_str_vt = Self::gc_ref_val_type(go_string_idx);
            let arr_vt = Self::gc_ref_val_type(byte_array_idx);

            // emit_i64_to_string pushed a GoString ref
            let int_str_ref = locals.add_local(&format!("__ftos_isr_{}", locals.locals.len()), go_str_vt);
            out.push(Instruction::LocalSet(int_str_ref));

            // Extract int_ptr and int_len from the GoString for the linear memory part
            // Actually, in GC mode the int part is already in a GoString.
            // We still need the linear memory copies for building the final string,
            // then wrap the result in a GoString at the end.
            // Instead, extract the int part's array ref and length.
            let int_arr = locals.add_local(&format!("__ftos_ia_{}", locals.locals.len()), arr_vt);
            out.push(Instruction::LocalGet(int_str_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(int_arr));
            out.push(Instruction::LocalGet(int_str_ref));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(int_len));

            let gc_result_ref = locals.add_local(&format!("__ftos_gcr_{}", locals.locals.len()), go_str_vt);

            // Check if frac is zero
            out.push(Instruction::LocalGet(frac_val));
            out.push(Instruction::F64Const(1e-9_f64.into()));
            out.push(Instruction::F64Lt);
            out.push(Instruction::If(BlockType::Empty));
            {
                // No fractional part: result = sign + int digits
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(total_len));

                let result_arr = locals.add_local(&format!("__ftos_ra_{}", locals.locals.len()), arr_vt);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::ArrayNew(byte_array_idx));
                out.push(Instruction::LocalSet(result_arr));

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32Const(45)); // '-'
                out.push(Instruction::ArraySet(byte_array_idx));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalGet(int_arr));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::ArrayCopy {
                    array_type_index_dst: byte_array_idx,
                    array_type_index_src: byte_array_idx,
                });

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::StructNew(go_string_idx));
                out.push(Instruction::LocalSet(gc_result_ref));
            }
            out.push(Instruction::Else);
            {
                out.push(Instruction::LocalGet(frac_val));
                out.push(Instruction::F64Const(1e6_f64.into()));
                out.push(Instruction::F64Mul);
                out.push(Instruction::F64Const(0.5_f64.into()));
                out.push(Instruction::F64Add);
                out.push(Instruction::F64Floor);
                out.push(Instruction::I64TruncF64S);
                out.push(Instruction::LocalSet(frac_int));

                let frac_arr = locals.add_local(&format!("__ftos_fa_{}", locals.locals.len()), arr_vt);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32Const(6));
                out.push(Instruction::ArrayNew(byte_array_idx));
                out.push(Instruction::LocalSet(frac_arr));

                {
                    let digit_idx = locals.add_local(&format!("__ftos_di_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::I32Const(5));
                    out.push(Instruction::LocalSet(digit_idx));

                    out.push(Instruction::Block(BlockType::Empty));
                    out.push(Instruction::Loop(BlockType::Empty));
                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::I32LtS);
                    out.push(Instruction::BrIf(1));

                    out.push(Instruction::LocalGet(frac_arr));
                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::LocalGet(frac_int));
                    out.push(Instruction::I64Const(10));
                    out.push(Instruction::I64RemU);
                    out.push(Instruction::I32WrapI64);
                    out.push(Instruction::I32Const(48));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::ArraySet(byte_array_idx));

                    out.push(Instruction::LocalGet(frac_int));
                    out.push(Instruction::I64Const(10));
                    out.push(Instruction::I64DivU);
                    out.push(Instruction::LocalSet(frac_int));

                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Sub);
                    out.push(Instruction::LocalSet(digit_idx));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                }

                out.push(Instruction::I32Const(6));
                out.push(Instruction::LocalSet(frac_pos));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32LeS);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(frac_arr));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::ArrayGetU(byte_array_idx));
                out.push(Instruction::I32Const(48)); // '0'
                out.push(Instruction::I32Ne);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::LocalSet(frac_pos));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Const(1)); // for '.'
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(total_len));

                let result_arr = locals.add_local(&format!("__ftos_ra2_{}", locals.locals.len()), arr_vt);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::ArrayNew(byte_array_idx));
                out.push(Instruction::LocalSet(result_arr));

                let cursor = locals.add_local(&format!("__ftos_cur_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32Const(45)); // '-'
                out.push(Instruction::ArraySet(byte_array_idx));
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalSet(cursor));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::LocalGet(int_arr));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::ArrayCopy {
                    array_type_index_dst: byte_array_idx,
                    array_type_index_src: byte_array_idx,
                });
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Const(46)); // '.'
                out.push(Instruction::ArraySet(byte_array_idx));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::LocalGet(frac_arr));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::ArrayCopy {
                    array_type_index_dst: byte_array_idx,
                    array_type_index_src: byte_array_idx,
                });

                out.push(Instruction::LocalGet(result_arr));
                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::StructNew(go_string_idx));
                out.push(Instruction::LocalSet(gc_result_ref));
            }
            out.push(Instruction::End); // end if frac == 0

            out.push(Instruction::LocalGet(gc_result_ref));
        } else {
            out.push(Instruction::LocalSet(int_len));
            out.push(Instruction::LocalSet(int_ptr));

            out.push(Instruction::LocalGet(frac_val));
            out.push(Instruction::F64Const(1e-9_f64.into()));
            out.push(Instruction::F64Lt);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(total_len));
                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                out.push(Instruction::LocalSet(final_ptr));

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::I32Const(45)); // '-'
                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(int_ptr));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
            }
            out.push(Instruction::Else);
            {
                out.push(Instruction::LocalGet(frac_val));
                out.push(Instruction::F64Const(1e6_f64.into()));
                out.push(Instruction::F64Mul);
                out.push(Instruction::F64Const(0.5_f64.into()));
                out.push(Instruction::F64Add);
                out.push(Instruction::F64Floor);
                out.push(Instruction::I64TruncF64S);
                out.push(Instruction::LocalSet(frac_int));

                out.push(Instruction::I32Const(8));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                out.push(Instruction::LocalSet(frac_buf));

                {
                    let digit_idx = locals.add_local(&format!("__ftos_di_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::I32Const(5));
                    out.push(Instruction::LocalSet(digit_idx));

                    out.push(Instruction::Block(BlockType::Empty));
                    out.push(Instruction::Loop(BlockType::Empty));
                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::I32LtS);
                    out.push(Instruction::BrIf(1));

                    out.push(Instruction::LocalGet(frac_buf));
                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalGet(frac_int));
                    out.push(Instruction::I64Const(10));
                    out.push(Instruction::I64RemU);
                    out.push(Instruction::I32WrapI64);
                    out.push(Instruction::I32Const(48));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

                    out.push(Instruction::LocalGet(frac_int));
                    out.push(Instruction::I64Const(10));
                    out.push(Instruction::I64DivU);
                    out.push(Instruction::LocalSet(frac_int));

                    out.push(Instruction::LocalGet(digit_idx));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Sub);
                    out.push(Instruction::LocalSet(digit_idx));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                }

                out.push(Instruction::I32Const(6));
                out.push(Instruction::LocalSet(frac_pos));

                out.push(Instruction::Block(BlockType::Empty));
                out.push(Instruction::Loop(BlockType::Empty));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32LeS);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(frac_buf));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::I32Const(48)); // '0'
                out.push(Instruction::I32Ne);
                out.push(Instruction::BrIf(1));

                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::LocalSet(frac_pos));
                out.push(Instruction::Br(0));
                out.push(Instruction::End); // loop
                out.push(Instruction::End); // block

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Const(1)); // for '.'
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(total_len));

                out.push(Instruction::LocalGet(total_len));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                out.push(Instruction::LocalSet(final_ptr));

                let cursor = locals.add_local(&format!("__ftos_cur_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::I32Const(45)); // '-'
                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalGet(is_neg));
                out.push(Instruction::LocalSet(cursor));
                out.push(Instruction::End);

                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(int_ptr));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::LocalGet(int_len));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Const(46)); // '.'
                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(cursor));

                out.push(Instruction::LocalGet(final_ptr));
                out.push(Instruction::LocalGet(cursor));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalGet(frac_buf));
                out.push(Instruction::LocalGet(frac_pos));
                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
            }
            out.push(Instruction::End); // end if frac == 0

            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(total_len));
        }

        Ok(())
    }
}
