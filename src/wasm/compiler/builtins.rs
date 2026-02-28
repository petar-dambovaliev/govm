use super::*;

impl WasmCompiler {
    pub(crate) fn emit_gc_default_value(elem_vt: ValType, out: &mut Vec<Instruction<'static>>) {
        match elem_vt {
            ValType::I32 => out.push(Instruction::I32Const(0)),
            ValType::I64 => out.push(Instruction::I64Const(0)),
            ValType::F32 => out.push(Instruction::F32Const(0.0_f32.into())),
            ValType::F64 => out.push(Instruction::F64Const(0.0_f64.into())),
            ValType::Ref(rt) => out.push(Instruction::RefNull(rt.heap_type)),
            _ => out.push(Instruction::I32Const(0)),
        }
    }

    pub(crate) fn compile_builtin_len(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let arg = match call.args.first() {
            Some(a) => a,
            None => {
                return Err(Error::InternalError(
                    "len() requires 1 argument".to_string(),
                ));
            }
        };

        let mut done = false;
        if let ast::Expression::Ident(ident) = arg {
            if let Some(&gc_ref_idx) = locals.gc_string_locals.get(&ident.name) {
                let go_string_idx = self.gc_builtin_types.go_string.unwrap();
                out.push(Instruction::LocalGet(gc_ref_idx));
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                done = true;
            }
            if !done {
                if let Some(&(_, len_local)) = locals.string_locals.get(&ident.name) {
                    out.push(Instruction::LocalGet(len_local));
                    done = true;
                }
            }
            if !done {
                if let Some(gc_info) = self.resolve_gc_slice_info_for_ident(&ident.name, locals) {
                    let slice_vt = Self::gc_ref_val_type(gc_info.slice_gc_idx);
                    let tmp = locals.add_local(&format!("__len_gc_{}", locals.locals.len()), slice_vt);
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::LocalTee(tmp));
                    out.push(Instruction::RefIsNull);
                    out.push(Instruction::If(BlockType::Result(ValType::I32)));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::Else);
                    out.push(Instruction::LocalGet(tmp));
                    out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 2 });
                    out.push(Instruction::End);
                    done = true;
                }
            }
            if !done {
                let is_slice = locals.is_var_type_slice(&ident.name)
                    || matches!(self.global_var_struct_types.get(&self.resolve_global_var_name(&ident.name)), Some(DefineType::Slice(_)));
                if is_slice {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 4,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                }
            }
            if !done && locals.is_var_type_map(&ident.name) {
                self.compile_expression(arg, out, locals)?;
                let map_ptr = locals.add_local(
                    &format!("__len_map_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalTee(map_ptr));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::Else);
                out.push(Instruction::LocalGet(map_ptr));
                out.push(Instruction::I32Load(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                }));
                out.push(Instruction::End);
                done = true;
            }
            if !done {
                if let Some(&(_, arr_len, ..)) = locals.array_info.get(&ident.name) {
                    out.push(Instruction::I32Const(arr_len as i32));
                    done = true;
                }
            }
        }

        if !done {
            if let ast::Expression::Selector(sel) = arg {
                if self.is_selector_slice_field(sel, locals) {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 4,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                } else if self.is_selector_map_field(sel, locals) {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 0,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                }
            }
        }

        // Fallback: use infer_val_type to detect GC slice refs from any expression
        if !done {
            if let Some(gc_info) = self.lookup_gc_slice_info(self.infer_val_type(arg, locals)) {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 2 });
                done = true;
            }
        }

        if !done {
            let is_gc_string = self.gc_builtin_types.go_string.is_some() && self.is_string_expr(arg, locals);
            // Non-string Slice expressions produce a header pointer; load len from header[4]
            let is_non_string_slice = matches!(arg, ast::Expression::Slice(_))
                && !self.is_string_expr(arg, locals);

            self.compile_expression(arg, out, locals)?;

            if is_gc_string {
                let go_string_idx = self.gc_builtin_types.go_string.unwrap();
                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            } else if is_non_string_slice {
                out.push(Instruction::I32Load(MemArg {
                    offset: 4,
                    align: 2,
                    memory_index: 0,
                }));
            } else {
                let result_count = self.expression_result_count(arg, Some(locals));
                if result_count >= 3 {
                    out.push(Instruction::Drop); // cap
                    let len_tmp = locals.add_local(
                        &format!("__len_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(len_tmp));
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(len_tmp));
                } else if result_count == 2 {
                    let len_tmp = locals.add_local(
                        &format!("__len_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(len_tmp));
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(len_tmp));
                }
            }
        }

        // Per Go spec, len() returns int (I64)
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    pub(crate) fn compile_builtin_cap(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let arg = match call.args.first() {
            Some(a) => a,
            None => {
                return Err(Error::InternalError(
                    "cap() requires 1 argument".to_string(),
                ));
            }
        };

        let mut done = false;
        if let ast::Expression::Ident(ident) = arg {
            if let Some(&(_, arr_len, ..)) = locals.array_info.get(&ident.name) {
                out.push(Instruction::I32Const(arr_len as i32));
                done = true;
            }
            if !done {
                if let Some(gc_info) = self.resolve_gc_slice_info_for_ident(&ident.name, locals) {
                    let slice_vt = Self::gc_ref_val_type(gc_info.slice_gc_idx);
                    let tmp = locals.add_local(&format!("__cap_gc_{}", locals.locals.len()), slice_vt);
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::LocalTee(tmp));
                    out.push(Instruction::RefIsNull);
                    out.push(Instruction::If(BlockType::Result(ValType::I32)));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::Else);
                    out.push(Instruction::LocalGet(tmp));
                    out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 3 });
                    out.push(Instruction::End);
                    done = true;
                }
            }
            if !done {
                let is_slice = locals.is_var_type_slice(&ident.name)
                    || matches!(self.global_var_struct_types.get(&self.resolve_global_var_name(&ident.name)), Some(DefineType::Slice(_)));
                if is_slice {
                    self.compile_expression(arg, out, locals)?;
                    out.push(Instruction::I32Load(MemArg {
                        offset: 8,
                        align: 2,
                        memory_index: 0,
                    }));
                    done = true;
                }
            }
        }

        if !done {
            if let Some(gc_info) = self.lookup_gc_slice_info(self.infer_val_type(arg, locals)) {
                self.compile_expression(arg, out, locals)?;
                out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 3 });
                done = true;
            }
        }

        if !done {
            let is_non_string_slice = matches!(arg, ast::Expression::Slice(_))
                && !self.is_string_expr(arg, locals);

            self.compile_expression(arg, out, locals)?;

            if is_non_string_slice {
                out.push(Instruction::I32Load(MemArg {
                    offset: 8,
                    align: 2,
                    memory_index: 0,
                }));
            } else {
                let result_count = self.expression_result_count(arg, Some(locals));
                if result_count >= 3 {
                    let cap_tmp = locals.add_local(
                        &format!("__cap_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(cap_tmp));
                    out.push(Instruction::Drop); // len
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(cap_tmp));
                } else if result_count == 2 {
                    let cap_tmp = locals.add_local(
                        &format!("__cap_tmp_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(cap_tmp));
                    out.push(Instruction::Drop); // ptr
                    out.push(Instruction::LocalGet(cap_tmp));
                }
            }
        }

        // Per Go spec, cap() returns int (I64)
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    pub(crate) fn compile_builtin_copy(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if call.args.len() < 2 {
            return Err(Error::InternalError(
                "copy() requires 2 arguments".to_string(),
            ));
        }

        let src_is_string = self.is_string_expr(&call.args[1], locals);

        if src_is_string {
            return self.compile_builtin_copy_from_string(call, out, locals);
        }

        let dst_gc = if let ast::Expression::Ident(id) = &call.args[0] {
            self.resolve_gc_slice_info_for_ident(&id.name, locals)
        } else { None };

        if let Some(gc_info) = dst_gc {
            return self.compile_gc_copy(call, &gc_info, out, locals);
        }

        let elem_vt = if let ast::Expression::Ident(ident) = &call.args[0] {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let (elem_size, _) = Self::elem_size_and_align(elem_vt);

        // Compile dst slice header
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local(
            &format!("__copy_dst_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(dst_hdr));

        // Compile src slice header
        self.compile_expression(&call.args[1], out, locals)?;
        let src_hdr = locals.add_local(
            &format!("__copy_src_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(src_hdr));

        let n_local = locals.add_local(
            &format!("__copy_n_{}", locals.locals.len()),
            ValType::I32,
        );

        // Nil slice guard: if either header is nil, copy returns 0
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::I32Or);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        // Load dst len
        let dst_len = locals.add_local(
            &format!("__copy_dlen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(dst_len));

        // Load src len
        let src_len = locals.add_local(
            &format!("__copy_slen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(src_len));

        // n = min(dst_len, src_len)
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32LeU);
        out.push(Instruction::Select);
        out.push(Instruction::LocalSet(n_local));

        // memory.copy(dst_data_ptr, src_data_ptr, n * elem_size)
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        out.push(Instruction::End); // end nil slice guard

        // Push n as i64 (Go copy returns int); n_local is 0 if nil branch was taken
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    fn compile_gc_copy(
        &mut self,
        call: &ast::Call,
        gc_info: &super::GcSliceInfo,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let slice_vt = Self::gc_ref_val_type(gc_info.slice_gc_idx);
        let arr_vt = Self::gc_ref_val_type(gc_info.array_gc_idx);

        // Compile dst
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_ref = locals.add_local(&format!("__gccopy_dr_{}", locals.locals.len()), slice_vt);
        out.push(Instruction::LocalSet(dst_ref));

        // Compile src
        self.compile_expression(&call.args[1], out, locals)?;
        let src_ref = locals.add_local(&format!("__gccopy_sr_{}", locals.locals.len()), slice_vt);
        out.push(Instruction::LocalSet(src_ref));

        let n_local = locals.add_local(&format!("__gccopy_n_{}", locals.locals.len()), ValType::I32);

        // Nil guard: if dst or src is null, return 0
        out.push(Instruction::LocalGet(dst_ref));
        out.push(Instruction::RefIsNull);
        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::RefIsNull);
        out.push(Instruction::I32Or);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        // Extract dst array, offset, len
        let dst_arr = locals.add_local(&format!("__gccopy_da_{}", locals.locals.len()), arr_vt);
        out.push(Instruction::LocalGet(dst_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 0 });
        out.push(Instruction::LocalSet(dst_arr));
        let dst_off = locals.add_local(&format!("__gccopy_do_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(dst_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 1 });
        out.push(Instruction::LocalSet(dst_off));
        let dst_len = locals.add_local(&format!("__gccopy_dl_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(dst_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 2 });
        out.push(Instruction::LocalSet(dst_len));

        // Extract src array, offset, len
        let src_arr = locals.add_local(&format!("__gccopy_sa_{}", locals.locals.len()), arr_vt);
        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 0 });
        out.push(Instruction::LocalSet(src_arr));
        let src_off = locals.add_local(&format!("__gccopy_so_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 1 });
        out.push(Instruction::LocalSet(src_off));
        let src_len = locals.add_local(&format!("__gccopy_sl_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(src_ref));
        out.push(Instruction::StructGet { struct_type_index: gc_info.slice_gc_idx, field_index: 2 });
        out.push(Instruction::LocalSet(src_len));

        // n = min(dst_len, src_len)
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32LeU);
        out.push(Instruction::Select);
        out.push(Instruction::LocalSet(n_local));

        // array.copy(dst_arr, dst_off, src_arr, src_off, n)
        out.push(Instruction::LocalGet(dst_arr));
        out.push(Instruction::LocalGet(dst_off));
        out.push(Instruction::LocalGet(src_arr));
        out.push(Instruction::LocalGet(src_off));
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::ArrayCopy {
            array_type_index_dst: gc_info.array_gc_idx,
            array_type_index_src: gc_info.array_gc_idx,
        });

        out.push(Instruction::End);

        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    pub(crate) fn compile_builtin_copy_from_string(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // copy(dst []byte, src string): copy bytes from string into byte slice
        // Strings are (ptr, len) on the stack; byte slices store each byte as an I32 in 4-byte slots.

        // Compile dst slice header
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__cpys_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Compile src string (pushes ptr, len — or GC ref)
        self.compile_expression(&call.args[1], out, locals)?;
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
        }
        let src_len = locals.add_local("__cpys_slen", ValType::I32);
        let src_ptr = locals.add_local("__cpys_sptr", ValType::I32);
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        // Load dst len
        let dst_len = locals.add_local("__cpys_dlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));

        // n = min(dst_len, src_len)
        let n_local = locals.add_local("__cpys_n", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32LeU);
        out.push(Instruction::Select);
        out.push(Instruction::LocalSet(n_local));

        // Load dst data pointer
        let dst_data = locals.add_local("__cpys_ddata", ValType::I32);
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        // Loop: copy each byte from string into 4-byte I32 slots
        let idx = locals.add_local("__cpys_idx", ValType::I32);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        // if idx >= n, break
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // dst_data[idx * 4] = (i32) src_ptr[idx]
        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(4));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

        // idx++
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0));

        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        // Push n as i64 (Go copy returns int)
        out.push(Instruction::LocalGet(n_local));
        out.push(Instruction::I64ExtendI32S);
        Ok(())
    }

    pub(crate) fn map_key_val_types(&self, map_type: &ast::MapType) -> (ValType, u32, ValType, u32, bool, bool, Option<String>) {
        let key_vt = self.expr_to_val_type(&map_type.key);
        let val_vt = self.expr_to_val_type(&map_type.val);
        let is_string_key = matches!(&*map_type.key, ast::Expression::Ident(id) if id.name == "string");
        let is_string_val = matches!(&*map_type.val, ast::Expression::Ident(id) if id.name == "string");
        let key_size = if is_string_key { 8u32 } else { match key_vt { ValType::I64 | ValType::F64 => 8u32, _ => 4u32 } };
        let val_size = if is_string_val { 8u32 } else { match val_vt { ValType::I64 | ValType::F64 => 8u32, _ => 4u32 } };
        let val_struct = if let ast::Expression::Ident(id) = map_type.val.as_ref() {
            if self.struct_defs.contains_key(&id.name) {
                Some(id.name.clone())
            } else {
                None
            }
        } else {
            None
        };
        (key_vt, key_size, val_vt, val_size, is_string_key, is_string_val, val_struct)
    }

    pub(crate) fn build_nested_map_type_info(&self, map_type: &ast::MapType) -> Option<Box<MapTypeInfo>> {
        if let ast::Expression::TypeMap(inner_map) = map_type.val.as_ref() {
            let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(inner_map);
            let nested = self.build_nested_map_type_info(inner_map);
            Some(Box::new(MapTypeInfo {
                key_vt: kv, val_vt: vv, key_size: ks, val_size: vs,
                is_string_key: sk, is_string_val: sv, val_struct_type: vst,
                nested_map_val_type: nested,
            }))
        } else {
            None
        }
    }

    pub(crate) fn map_entry_size(key_size: u32, val_size: u32) -> u32 {
        4 + key_size + val_size // tag(4) + key + val
    }

    pub(crate) fn compile_builtin_make(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Check if this is a map make
        if let Some(ast::Expression::TypeMap(map_type)) = call.args.first() {
            return self.compile_make_map(map_type, call, out, locals);
        }

        // Resolve named composite types: make(MySlice, n) or make(MyMap, n)
        if let Some(ast::Expression::Ident(type_ident)) = call.args.first() {
            if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                if let ast::Expression::TypeMap(map_type) = &underlying {
                    return self.compile_make_map(map_type, call, out, locals);
                }
            }
        }

        let use_linear = if let Some(ast::Expression::TypeSlice(sl)) = call.args.first() {
            !self.should_gc_slice_elem(&sl.typ)
        } else { false };

        if use_linear {
            return self.compile_make_slice_linear(call, out, locals);
        }

        let elem_vt = self.infer_slice_elem_type(call.args.first());
        let (slice_gc_idx, array_gc_idx) = self.get_or_create_gc_slice_type(elem_vt);

        let len_local = locals.add_local(
            &format!("__make_len_{}", locals.locals.len()),
            ValType::I32,
        );

        if let Some(len_arg) = call.args.get(1) {
            self.compile_expression(len_arg, out, locals)?;
            let vt = self.infer_val_type(len_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::I32Const(0));
        }
        out.push(Instruction::LocalSet(len_local));

        let cap_local = locals.add_local(
            &format!("__make_cap_{}", locals.locals.len()),
            ValType::I32,
        );
        if let Some(cap_arg) = call.args.get(2) {
            self.compile_expression(cap_arg, out, locals)?;
            let vt = self.infer_val_type(cap_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
        } else {
            out.push(Instruction::LocalGet(len_local));
        }
        out.push(Instruction::LocalSet(cap_local));

        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Unreachable);
        out.push(Instruction::End);

        Self::emit_gc_default_value(elem_vt, out);
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::ArrayNew(array_gc_idx));
        // stack: array_ref
        out.push(Instruction::I32Const(0)); // offset
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::StructNew(slice_gc_idx));

        Ok(())
    }

    fn compile_make_slice_linear(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let elem_vt = self.infer_slice_elem_type(call.args.first());
        let (elem_size, _align) = Self::elem_size_and_align(elem_vt);
        const HEADER_SIZE: i32 = 12;

        let len_local = locals.add_local(&format!("__mksl_len_{}", locals.locals.len()), ValType::I32);
        let cap_local = locals.add_local(&format!("__mksl_cap_{}", locals.locals.len()), ValType::I32);

        if let Some(len_arg) = call.args.get(1) {
            self.compile_expression(len_arg, out, locals)?;
            let vt = self.infer_val_type(len_arg, locals);
            if vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
        } else {
            out.push(Instruction::I32Const(0));
        }
        out.push(Instruction::LocalSet(len_local));

        if let Some(cap_arg) = call.args.get(2) {
            self.compile_expression(cap_arg, out, locals)?;
            let vt = self.infer_val_type(cap_arg, locals);
            if vt == ValType::I64 { out.push(Instruction::I32WrapI64); }
        } else {
            out.push(Instruction::LocalGet(len_local));
        }
        out.push(Instruction::LocalSet(cap_local));

        out.push(Instruction::I32Const(HEADER_SIZE));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let hdr = locals.add_local(&format!("__mksl_hdr_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(hdr));

        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data = locals.add_local(&format!("__mksl_dat_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(data));

        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(data));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr));
        Ok(())
    }

    pub(crate) fn compile_make_map(
        &mut self,
        map_type: &ast::MapType,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let (key_vt, key_size, val_vt, val_size, _is_string_key, _is_string_val, _val_struct) = self.map_key_val_types(map_type);
        let entry_size = Self::map_entry_size(key_size, val_size);

        let init_cap = if let Some(cap_arg) = call.args.get(1) {
            // Compile the capacity hint; use it as initial capacity
            self.compile_expression(cap_arg, out, locals)?;
            let vt = self.infer_val_type(cap_arg, locals);
            if vt == ValType::I64 {
                out.push(Instruction::I32WrapI64);
            }
            let cap_l = locals.add_local(&format!("__mmap_cap_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(cap_l));
            Some(cap_l)
        } else {
            None
        };

        // Map header: 20 bytes — count(4), cap(4), data_ptr(4), key_size(4), val_size(4)
        out.push(Instruction::I32Const(20));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let hdr = locals.add_local(&format!("__mmap_hdr_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(hdr));

        let actual_cap = if let Some(cl) = init_cap {
            cl
        } else {
            let default_cap = locals.add_local(&format!("__mmap_dcap_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::I32Const(8)); // default capacity 8
            out.push(Instruction::LocalSet(default_cap));
            default_cap
        };

        // Allocate data: cap * entry_size
        out.push(Instruction::LocalGet(actual_cap));
        out.push(Instruction::I32Const(entry_size as i32));
        out.push(Instruction::I32Mul);
        let data_sz = locals.add_local(&format!("__mmap_dsz_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalTee(data_sz));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_ptr = locals.add_local(&format!("__mmap_dp_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalTee(data_ptr));

        // Zero-fill data
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalGet(data_sz));
        out.push(Instruction::MemoryFill(0));

        // Write header fields
        // count = 0
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        // capacity
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(actual_cap));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        // data_ptr
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        // key_size
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(key_size as i32));
        out.push(Instruction::I32Store(MemArg { offset: 12, align: 2, memory_index: 0 }));
        // val_size
        out.push(Instruction::LocalGet(hdr));
        out.push(Instruction::I32Const(val_size as i32));
        out.push(Instruction::I32Store(MemArg { offset: 16, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(hdr));

        // Stash info so that index/assign can find key/val types
        let _ = (key_vt, val_vt);
        Ok(())
    }

    pub(crate) fn compile_builtin_append(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if call.args.len() < 2 {
            return Err(Error::InternalError(
                "append() requires at least 2 arguments".to_string(),
            ));
        }

        let gc_info_opt = if let ast::Expression::Ident(ident) = &call.args[0] {
            self.resolve_gc_slice_info_for_ident(&ident.name, locals)
        } else {
            self.lookup_gc_slice_info(self.infer_val_type(&call.args[0], locals))
        };
        if let Some(gc_info) = gc_info_opt {
            if call.dots.is_some() && call.args.len() == 2 {
                return self.compile_gc_append_spread(call, out, locals, &gc_info);
            }
            return self.compile_gc_append(call, out, locals, &gc_info);
        }

        let elem_vt_from_slice = if let ast::Expression::Ident(ident) = &call.args[0] {
            locals
                .slice_elem_types
                .get(&ident.name)
                .copied()
                .unwrap_or(ValType::I64)
        } else {
            ValType::I64
        };
        let (elem_size, _align) = Self::elem_size_and_align(elem_vt_from_slice);
        let elem_vt = elem_vt_from_slice;

        // Handle append(s1, s2...) — spread a source slice into the destination
        if call.dots.is_some() && call.args.len() == 2 {
            if self.is_string_expr(&call.args[1], locals) {
                return self.compile_builtin_append_spread_string(call, out, locals);
            }
            return self.compile_builtin_append_spread(call, out, locals, elem_vt, elem_size);
        }

        let num_new_elems = (call.args.len() - 1) as i32;

        // Go spec: evaluate arguments left-to-right. Compile slice first, then elements.
        self.compile_expression(&call.args[0], out, locals)?;
        let hdr_local = locals.add_local(
            &format!("__app_hdr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(hdr_local));

        let mut elem_locals = Vec::with_capacity(num_new_elems as usize);
        for i in 1..call.args.len() {
            self.compile_expression(&call.args[i], out, locals)?;
            let expr_vt = self.infer_val_type(&call.args[i], locals);
            if expr_vt != elem_vt_from_slice {
                match (expr_vt, elem_vt_from_slice) {
                    (ValType::I64, ValType::I32) => out.push(Instruction::I32WrapI64),
                    (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32S),
                    (ValType::F64, ValType::F32) => out.push(Instruction::F32DemoteF64),
                    (ValType::F32, ValType::F64) => out.push(Instruction::F64PromoteF32),
                    _ => {
                        return Err(Error::InternalError(format!(
                            "type mismatch in append: element type {:?} cannot be coerced to slice element type {:?}",
                            expr_vt, elem_vt_from_slice
                        )));
                    }
                }
            }
            let el = locals.add_local(
                &format!("__app_elem_{}_{}", i, locals.locals.len()),
                elem_vt,
            );
            out.push(Instruction::LocalSet(el));
            elem_locals.push(el);
        }

        // Nil-slice guard: if hdr_local == 0, allocate a fresh 12-byte header
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::I32Const(12));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(hdr_local));
        }
        out.push(Instruction::End);

        // Allocate a NEW result header so the original slice is not mutated
        let result_hdr = locals.add_local(
            &format!("__app_res_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(12));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(result_hdr));
        // Copy original header into result: data_ptr, len, cap
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(hdr_local));
        out.push(Instruction::I32Const(12));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Load current len
        let old_len = locals.add_local(
            &format!("__app_len_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(old_len));

        // new_len = old_len + num_new_elems
        let new_len = locals.add_local(
            &format!("__app_nlen_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::I32Const(num_new_elems));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // Load current cap
        let cap_local = locals.add_local(
            &format!("__app_cap_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 8,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(cap_local));

        // If new_len > cap, grow
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            // new_cap = max((cap + 1) * 2, new_len)
            let new_cap = locals.add_local(
                &format!("__app_ncap_{}", locals.locals.len()),
                ValType::I32,
            );
            // doubled = (cap + 1) * 2 with overflow guard
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local(
                &format!("__app_cp1_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalTee(cap_plus1));
            // overflow if cap_plus1 < cap (wrapped)
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local(
                &format!("__app_dbl_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalTee(doubled));
            // overflow if doubled < cap_plus1 (wrapped around on multiply)
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            // new_cap = select(doubled, new_len, doubled >= new_len)
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            // Allocate new data: new_cap * elem_size (with overflow check)
            let new_data = locals.add_local(
                &format!("__app_ndata_{}", locals.locals.len()),
                ValType::I32,
            );
            let alloc_bytes = locals.add_local(
                &format!("__app_abytes_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            // overflow if alloc_bytes / elem_size != new_cap (and elem_size > 1)
            if elem_size > 1 {
                out.push(Instruction::I32Const(elem_size));
                out.push(Instruction::I32DivU);
                out.push(Instruction::LocalGet(new_cap));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Call(self.oom_func_idx));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
            }
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data: memory.copy(new_data, old_data_ptr, old_len * elem_size)
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy {
                dst_mem: 0,
                src_mem: 0,
            });

            // Update result header: data_ptr = new_data
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));

            // Update result header: cap = new_cap
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg {
                offset: 8,
                align: 2,
                memory_index: 0,
            }));
        }
        out.push(Instruction::End);

        // Load data_ptr from result header
        let data_ptr = locals.add_local(
            &format!("__app_dptr_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        out.push(Instruction::LocalSet(data_ptr));

        // Store each element at data_ptr + (old_len + i) * elem_size
        for (i, &el) in elem_locals.iter().enumerate() {
            out.push(Instruction::LocalGet(data_ptr));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::I32Const(i as i32));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(el));
            match elem_vt {
                ValType::I64 => out.push(Instruction::I64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::F64 => out.push(Instruction::F64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
                ValType::I32 => out.push(Instruction::I32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                })),
                ValType::F32 => out.push(Instruction::F32Store(MemArg {
                    offset: 0,
                    align: 2,
                    memory_index: 0,
                })),
                _ => out.push(Instruction::I64Store(MemArg {
                    offset: 0,
                    align: 3,
                    memory_index: 0,
                })),
            }
        }

        // Update result header: len = new_len
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        }));

        // Push result header pointer
        out.push(Instruction::LocalGet(result_hdr));
        Ok(())
    }

    /// Handle `append(dst, src...)` where src is a slice spread into dst.

    pub(crate) fn compile_gc_append(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        gc_info: &GcSliceInfo,
    ) -> Result<(), Error> {
        let slice_gc_idx = gc_info.slice_gc_idx;
        let array_gc_idx = gc_info.array_gc_idx;
        let elem_vt = gc_info.elem_vt;
        let slice_vt = Self::gc_ref_val_type(slice_gc_idx);
        let arr_vt = Self::gc_ref_val_type(array_gc_idx);
        let num_new = (call.args.len() - 1) as i32;

        // Evaluate the source slice
        self.compile_expression(&call.args[0], out, locals)?;
        let src_slice = locals.add_local(&format!("__gca_src_{}", locals.locals.len()), slice_vt);
        out.push(Instruction::LocalSet(src_slice));

        // Evaluate new elements
        let mut elem_locals = Vec::new();
        for i in 1..call.args.len() {
            self.compile_expression(&call.args[i], out, locals)?;
            let el = locals.add_local(&format!("__gca_el_{}_{}", i, locals.locals.len()), elem_vt);
            out.push(Instruction::LocalSet(el));
            elem_locals.push(el);
        }

        // Extract old array, offset, len, cap
        let old_arr = locals.add_local(&format!("__gca_oa_{}", locals.locals.len()), arr_vt);
        let old_off = locals.add_local(&format!("__gca_oo_{}", locals.locals.len()), ValType::I32);
        let old_len = locals.add_local(&format!("__gca_ol_{}", locals.locals.len()), ValType::I32);
        let old_cap = locals.add_local(&format!("__gca_oc_{}", locals.locals.len()), ValType::I32);

        // Handle nil slice: create empty slice if null
        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::RefIsNull);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Create empty array and slice
            Self::emit_gc_default_value(elem_vt, out);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::ArrayNew(array_gc_idx));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::StructNew(slice_gc_idx));
            out.push(Instruction::LocalSet(src_slice));
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 0 });
        out.push(Instruction::LocalSet(old_arr));
        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 1 });
        out.push(Instruction::LocalSet(old_off));
        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 2 });
        out.push(Instruction::LocalSet(old_len));
        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 3 });
        out.push(Instruction::LocalSet(old_cap));

        let new_len = locals.add_local(&format!("__gca_nl_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(old_len));
        out.push(Instruction::I32Const(num_new));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        let result_arr = locals.add_local(&format!("__gca_ra_{}", locals.locals.len()), arr_vt);
        let result_off = locals.add_local(&format!("__gca_ro_{}", locals.locals.len()), ValType::I32);
        let result_cap = locals.add_local(&format!("__gca_rc_{}", locals.locals.len()), ValType::I32);

        // If new_len > cap, grow: create a new array and copy
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(old_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local(&format!("__gca_nc_{}", locals.locals.len()), ValType::I32);
            // new_cap = max((cap+1)*2, new_len)
            out.push(Instruction::LocalGet(old_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local(&format!("__gca_dbl_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            // Create new array
            Self::emit_gc_default_value(elem_vt, out);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::ArrayNew(array_gc_idx));
            out.push(Instruction::LocalSet(result_arr));

            // Copy old elements: array.copy(dst, 0, src, old_off, old_len)
            out.push(Instruction::LocalGet(result_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(old_arr));
            out.push(Instruction::LocalGet(old_off));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::ArrayCopy { array_type_index_dst: array_gc_idx, array_type_index_src: array_gc_idx });

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result_off));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::LocalSet(result_cap));
        }
        out.push(Instruction::Else);
        {
            // Reuse existing array
            out.push(Instruction::LocalGet(old_arr));
            out.push(Instruction::LocalSet(result_arr));
            out.push(Instruction::LocalGet(old_off));
            out.push(Instruction::LocalSet(result_off));
            out.push(Instruction::LocalGet(old_cap));
            out.push(Instruction::LocalSet(result_cap));
        }
        out.push(Instruction::End);

        // ArraySet new elements at result_off + old_len + i
        for (i, &el) in elem_locals.iter().enumerate() {
            out.push(Instruction::LocalGet(result_arr));
            out.push(Instruction::LocalGet(result_off));
            out.push(Instruction::LocalGet(old_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(i as i32));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(el));
            out.push(Instruction::ArraySet(array_gc_idx));
        }

        // Construct result slice struct
        out.push(Instruction::LocalGet(result_arr));
        out.push(Instruction::LocalGet(result_off));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(result_cap));
        out.push(Instruction::StructNew(slice_gc_idx));

        Ok(())
    }

    fn compile_gc_append_spread(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        gc_info: &GcSliceInfo,
    ) -> Result<(), Error> {
        let slice_gc_idx = gc_info.slice_gc_idx;
        let array_gc_idx = gc_info.array_gc_idx;
        let elem_vt = gc_info.elem_vt;
        let slice_vt = Self::gc_ref_val_type(slice_gc_idx);
        let arr_vt = Self::gc_ref_val_type(array_gc_idx);

        // Compile dst slice
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_slice = locals.add_local(&format!("__gcas_dst_{}", locals.locals.len()), slice_vt);
        out.push(Instruction::LocalSet(dst_slice));

        // Compile src slice (the spread argument)
        self.compile_expression(&call.args[1], out, locals)?;
        let src_slice = locals.add_local(&format!("__gcas_src_{}", locals.locals.len()), slice_vt);
        out.push(Instruction::LocalSet(src_slice));

        // Handle nil dst: create empty
        out.push(Instruction::LocalGet(dst_slice));
        out.push(Instruction::RefIsNull);
        out.push(Instruction::If(BlockType::Empty));
        {
            Self::emit_gc_default_value(elem_vt, out);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::ArrayNew(array_gc_idx));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::StructNew(slice_gc_idx));
            out.push(Instruction::LocalSet(dst_slice));
        }
        out.push(Instruction::End);

        // Extract dst fields
        let dst_arr = locals.add_local(&format!("__gcas_da_{}", locals.locals.len()), arr_vt);
        let dst_off = locals.add_local(&format!("__gcas_do_{}", locals.locals.len()), ValType::I32);
        let dst_len = locals.add_local(&format!("__gcas_dl_{}", locals.locals.len()), ValType::I32);
        let dst_cap = locals.add_local(&format!("__gcas_dc_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(dst_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 0 });
        out.push(Instruction::LocalSet(dst_arr));
        out.push(Instruction::LocalGet(dst_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 1 });
        out.push(Instruction::LocalSet(dst_off));
        out.push(Instruction::LocalGet(dst_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 2 });
        out.push(Instruction::LocalSet(dst_len));
        out.push(Instruction::LocalGet(dst_slice));
        out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 3 });
        out.push(Instruction::LocalSet(dst_cap));

        // Extract src len (handle nil src)
        let src_arr = locals.add_local(&format!("__gcas_sa_{}", locals.locals.len()), arr_vt);
        let src_off = locals.add_local(&format!("__gcas_so_{}", locals.locals.len()), ValType::I32);
        let src_len = locals.add_local(&format!("__gcas_sl_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalGet(src_slice));
        out.push(Instruction::RefIsNull);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);
        {
            out.push(Instruction::LocalGet(src_slice));
            out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 0 });
            out.push(Instruction::LocalSet(src_arr));
            out.push(Instruction::LocalGet(src_slice));
            out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 1 });
            out.push(Instruction::LocalSet(src_off));
            out.push(Instruction::LocalGet(src_slice));
            out.push(Instruction::StructGet { struct_type_index: slice_gc_idx, field_index: 2 });
            out.push(Instruction::LocalSet(src_len));
        }
        out.push(Instruction::End);

        // new_len = dst_len + src_len
        let new_len = locals.add_local(&format!("__gcas_nl_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        let result_arr = locals.add_local(&format!("__gcas_ra_{}", locals.locals.len()), arr_vt);
        let result_off = locals.add_local(&format!("__gcas_ro_{}", locals.locals.len()), ValType::I32);
        let result_cap = locals.add_local(&format!("__gcas_rc_{}", locals.locals.len()), ValType::I32);

        // If new_len > dst_cap, grow
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(dst_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local(&format!("__gcas_nc_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local(&format!("__gcas_dbl_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            Self::emit_gc_default_value(elem_vt, out);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::ArrayNew(array_gc_idx));
            out.push(Instruction::LocalSet(result_arr));

            // Copy dst elements
            out.push(Instruction::LocalGet(result_arr));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalGet(dst_arr));
            out.push(Instruction::LocalGet(dst_off));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::ArrayCopy { array_type_index_dst: array_gc_idx, array_type_index_src: array_gc_idx });

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(result_off));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::LocalSet(result_cap));
        }
        out.push(Instruction::Else);
        {
            out.push(Instruction::LocalGet(dst_arr));
            out.push(Instruction::LocalSet(result_arr));
            out.push(Instruction::LocalGet(dst_off));
            out.push(Instruction::LocalSet(result_off));
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::LocalSet(result_cap));
        }
        out.push(Instruction::End);

        // Copy src elements at result_off + dst_len
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::LocalGet(result_arr));
            out.push(Instruction::LocalGet(result_off));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(src_arr));
            out.push(Instruction::LocalGet(src_off));
            out.push(Instruction::LocalGet(src_len));
            out.push(Instruction::ArrayCopy { array_type_index_dst: array_gc_idx, array_type_index_src: array_gc_idx });
        }
        out.push(Instruction::End);

        // Construct result
        out.push(Instruction::LocalGet(result_arr));
        out.push(Instruction::LocalGet(result_off));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(result_cap));
        out.push(Instruction::StructNew(slice_gc_idx));

        Ok(())
    }

    pub(crate) fn compile_builtin_append_spread(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        _elem_vt: ValType,
        elem_size: i32,
    ) -> Result<(), Error> {
        // Compile source slice (second arg) — get its header pointer
        self.compile_expression(&call.args[1], out, locals)?;
        let src_hdr = locals.add_local("__appsprd_shdr", ValType::I32);
        out.push(Instruction::LocalSet(src_hdr));

        // Nil-guard for src_hdr: if nil, src has zero elements
        let src_len = locals.add_local("__appsprd_slen", ValType::I32);
        let src_data = locals.add_local("__appsprd_sdata", ValType::I32);
        out.push(Instruction::LocalGet(src_hdr));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(src_len));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(src_data));
        }
        out.push(Instruction::Else);
        {
            out.push(Instruction::LocalGet(src_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(src_len));
            out.push(Instruction::LocalGet(src_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(src_data));
        }
        out.push(Instruction::End);

        // Compile destination slice (first arg)
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__appsprd_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Nil-guard for dst_hdr: allocate fresh 12-byte header if nil
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::I32Const(12));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(dst_hdr));
        }
        out.push(Instruction::End);

        // Allocate a NEW result header so the original slice is not mutated
        let result_hdr = locals.add_local("__appsprd_res", ValType::I32);
        out.push(Instruction::I32Const(12));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(result_hdr));
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Const(12));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Load dst len and cap
        let dst_len = locals.add_local("__appsprd_dlen", ValType::I32);
        let dst_cap = locals.add_local("__appsprd_dcap", ValType::I32);
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_cap));

        // new_len = dst_len + src_len
        let new_len = locals.add_local("__appsprd_nlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // If new_len > cap, grow
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(dst_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local("__appsprd_ncap", ValType::I32);
            // doubled = (cap + 1) * 2
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local("__appsprd_cp1", ValType::I32);
            out.push(Instruction::LocalTee(cap_plus1));
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local("__appsprd_dbl", ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            // new_cap = max(doubled, new_len)
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            // Allocate new_cap * elem_size
            let alloc_bytes = locals.add_local("__appsprd_abytes", ValType::I32);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            if elem_size > 1 {
                out.push(Instruction::I32Const(elem_size));
                out.push(Instruction::I32DivU);
                out.push(Instruction::LocalGet(new_cap));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::Call(self.oom_func_idx));
                out.push(Instruction::Unreachable);
                out.push(Instruction::End);
            }
            let new_data = locals.add_local("__appsprd_ndata", ValType::I32);
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

            // Update result header
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Bulk copy: memory.copy(dst_data + dst_len*elem_size, src_data, src_len*elem_size)
        let dst_data = locals.add_local("__appsprd_ddptr", ValType::I32);
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_data));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Update result header: len = new_len
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(result_hdr));
        Ok(())
    }

    /// Handle `append(dst []byte, src string...)`: append bytes from string into byte slice.

    pub(crate) fn compile_builtin_append_spread_string(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let elem_size: i32 = 4; // []byte stores each byte in a 4-byte I32 slot

        // Compile source string (pushes ptr, len — or GC ref)
        self.compile_expression(&call.args[1], out, locals)?;
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
        }
        let src_len = locals.add_local("__appstr_slen", ValType::I32);
        let src_ptr = locals.add_local("__appstr_sptr", ValType::I32);
        out.push(Instruction::LocalSet(src_len));
        out.push(Instruction::LocalSet(src_ptr));

        // Compile destination slice (first arg)
        self.compile_expression(&call.args[0], out, locals)?;
        let dst_hdr = locals.add_local("__appstr_dhdr", ValType::I32);
        out.push(Instruction::LocalSet(dst_hdr));

        // Nil-guard for dst_hdr: allocate fresh 12-byte header if nil
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::I32Const(12));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(dst_hdr));
        }
        out.push(Instruction::End);

        // Allocate a NEW result header so the original slice is not mutated
        let result_hdr = locals.add_local("__appstr_res", ValType::I32);
        out.push(Instruction::I32Const(12));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(result_hdr));
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(dst_hdr));
        out.push(Instruction::I32Const(12));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Load dst len and cap
        let dst_len = locals.add_local("__appstr_dlen", ValType::I32);
        let dst_cap = locals.add_local("__appstr_dcap", ValType::I32);
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_len));
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_cap));

        // new_len = dst_len + src_len
        let new_len = locals.add_local("__appstr_nlen", ValType::I32);
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(new_len));

        // If new_len > cap, grow (same growth logic as append_spread)
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::LocalGet(dst_cap));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Empty));
        {
            let new_cap = locals.add_local("__appstr_ncap", ValType::I32);
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            let cap_plus1 = locals.add_local("__appstr_cp1", ValType::I32);
            out.push(Instruction::LocalTee(cap_plus1));
            out.push(Instruction::LocalGet(dst_cap));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32Const(2));
            out.push(Instruction::I32Mul);
            let doubled = locals.add_local("__appstr_dbl", ValType::I32);
            out.push(Instruction::LocalTee(doubled));
            out.push(Instruction::LocalGet(cap_plus1));
            out.push(Instruction::I32LtU);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::LocalGet(doubled));
            out.push(Instruction::LocalGet(new_len));
            out.push(Instruction::I32GeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(new_cap));

            let alloc_bytes = locals.add_local("__appstr_abytes", ValType::I32);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalTee(alloc_bytes));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32DivU);
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Call(self.oom_func_idx));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);
            let new_data = locals.add_local("__appstr_ndata", ValType::I32);
            out.push(Instruction::LocalGet(alloc_bytes));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(new_data));

            // Copy old data
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(dst_len));
            out.push(Instruction::I32Const(elem_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

            // Update result header
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_data));
            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(result_hdr));
            out.push(Instruction::LocalGet(new_cap));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Copy bytes from string into I32 slots: loop over each byte
        let dst_data = locals.add_local("__appstr_ddptr", ValType::I32);
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(dst_data));

        let idx = locals.add_local("__appstr_idx", ValType::I32);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        // if idx >= src_len, break
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // dst_data[(dst_len + idx) * 4] = (i32) src_ptr[idx]
        out.push(Instruction::LocalGet(dst_data));
        out.push(Instruction::LocalGet(dst_len));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Const(elem_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

        // idx++
        out.push(Instruction::LocalGet(idx));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx));
        out.push(Instruction::Br(0));

        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        // Update result header: len = new_len
        out.push(Instruction::LocalGet(result_hdr));
        out.push(Instruction::LocalGet(new_len));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

        out.push(Instruction::LocalGet(result_hdr));
        Ok(())
    }
}
