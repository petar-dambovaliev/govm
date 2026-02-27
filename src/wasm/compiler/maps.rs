use super::*;

impl WasmCompiler {
    pub(crate) fn compile_range_map(
        &mut self,
        range: &ast::RangeStmt,
        map_name: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        let entry_size = Self::map_entry_size(mti.key_size, mti.val_size) as i32;

        let idx_local = locals.add_local("__rm_idx", ValType::I32);
        let count_local = locals.add_local("__rm_count", ValType::I32);
        let cap_local = locals.add_local("__rm_cap", ValType::I32);
        let data_local = locals.add_local("__rm_data", ValType::I32);
        let entry_local = locals.add_local("__rm_entry", ValType::I32);
        let slot_local = locals.add_local("__rm_slot", ValType::I32);

        // Nil map guard: if map is nil, skip the entire loop (0 iterations)
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        // Load capacity and data_ptr from map header
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        // Randomize starting index: start = counter % cap (avoid div-by-zero)
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32GtU);
        out.push(Instruction::If(BlockType::Result(ValType::I32)));
        {
            out.push(Instruction::GlobalGet(self.map_iter_counter_global));
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32RemU);
        }
        out.push(Instruction::Else);
        out.push(Instruction::I32Const(0));
        out.push(Instruction::End);
        out.push(Instruction::LocalSet(idx_local));

        // Bump the global counter for next map iteration
        out.push(Instruction::GlobalGet(self.map_iter_counter_global));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::GlobalSet(self.map_iter_counter_global));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(count_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, false, true));

        // Check count < cap
        out.push(Instruction::LocalGet(count_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // slot = idx % cap (wrap around)
        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32RemU);
        out.push(Instruction::LocalSet(slot_local));

        // entry = data + slot * entry_size
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(slot_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        // Skip non-occupied entries (tag != 1)
        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Increment idx and count, continue
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
            out.push(Instruction::LocalGet(count_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(count_local));
            out.push(Instruction::Br(1)); // back to loop
        }
        out.push(Instruction::End);

        // Extract key
        if let Some(key) = &range.key {
            if let ast::Expression::Ident(ident) = key {
                if ident.name != "_" {
                    if mti.is_string_key {
                        let key_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, ValType::I32)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, ValType::I32))
                        };
                        let key_len_local = locals.add_local(&format!("{}__str_len", ident.name), ValType::I32);
                        locals.set_var_struct_type(&ident.name, "__string");
                        locals.string_locals.insert(ident.name.clone(), (key_local, key_len_local));
                        out.push(Instruction::LocalGet(entry_local));
                        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalSet(key_local));
                        out.push(Instruction::LocalGet(entry_local));
                        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalSet(key_len_local));
                    } else {
                        let key_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, mti.key_vt)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, mti.key_vt))
                        };
                        out.push(Instruction::LocalGet(entry_local));
                        let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
                        Self::emit_typed_load(mti.key_vt, 4, key_align, out);
                        out.push(Instruction::LocalSet(key_local));
                    }
                }
            }
        }

        // Extract value
        if let Some(value) = &range.value {
            if let ast::Expression::Ident(ident) = value {
                if ident.name != "_" {
                    let val_offset = 4u64 + mti.key_size as u64;
                    if mti.is_string_val {
                        let val_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, ValType::I32)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, ValType::I32))
                        };
                        let val_len_local = locals.add_local(&format!("{}__str_len", ident.name), ValType::I32);
                        locals.set_var_struct_type(&ident.name, "__string");
                        locals.string_locals.insert(ident.name.clone(), (val_local, val_len_local));
                        out.push(Instruction::LocalGet(entry_local));
                        out.push(Instruction::I32Load(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalSet(val_local));
                        out.push(Instruction::LocalGet(entry_local));
                        out.push(Instruction::I32Load(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
                        out.push(Instruction::LocalSet(val_len_local));
                    } else {
                        let val_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, mti.val_vt)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, mti.val_vt))
                        };
                        let (_, val_align) = Self::elem_size_and_align(mti.val_vt);
                        out.push(Instruction::LocalGet(entry_local));
                        Self::emit_typed_load(mti.val_vt, val_offset, val_align, out);
                        out.push(Instruction::LocalSet(val_local));
                    }
                }
            }
        }

        // Increment idx and count before body
        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(idx_local));
        out.push(Instruction::LocalGet(count_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(count_local));

        self.compile_block(&range.body, out, locals, result_types)?;

        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        self.loop_depth.pop();

        out.push(Instruction::End); // end nil map guard

        Ok(())
    }

    pub(crate) fn emit_map_key_eq(
        &self,
        mti: &MapTypeInfo,
        entry_local: u32,
        key_local: u32,
        key_len_local: Option<u32>,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
        if mti.is_string_key {
            let kl = key_len_local.expect("key_len_local must be set for string-keyed maps");

            if let Some(streq_idx) = self.rt_streq_func_idx {
                // Use __rt_streq(stored_ptr, stored_len, search_ptr, search_len)
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(key_local));
                out.push(Instruction::LocalGet(kl));
                out.push(Instruction::Call(streq_idx));
            } else {
                let eq_result = locals.add_local(&format!("__mkeq_{}", locals.locals.len()), ValType::I32);
                let cmp_idx = locals.add_local(&format!("__mkcidx_{}", locals.locals.len()), ValType::I32);

                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(kl));
                out.push(Instruction::I32Ne);
                out.push(Instruction::If(BlockType::Result(ValType::I32)));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::Else);
                {
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(eq_result));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(cmp_idx));
                    out.push(Instruction::Block(BlockType::Empty));
                    out.push(Instruction::Loop(BlockType::Empty));
                    out.push(Instruction::LocalGet(cmp_idx));
                    out.push(Instruction::LocalGet(kl));
                    out.push(Instruction::I32GeU);
                    out.push(Instruction::BrIf(1));
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalGet(cmp_idx));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    out.push(Instruction::LocalGet(key_local));
                    out.push(Instruction::LocalGet(cmp_idx));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    out.push(Instruction::I32Ne);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::I32Const(0));
                    out.push(Instruction::LocalSet(eq_result));
                    out.push(Instruction::Br(2));
                    out.push(Instruction::End);
                    out.push(Instruction::LocalGet(cmp_idx));
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalSet(cmp_idx));
                    out.push(Instruction::Br(0));
                    out.push(Instruction::End); // loop
                    out.push(Instruction::End); // block
                    out.push(Instruction::LocalGet(eq_result));
                }
                out.push(Instruction::End); // if/else
            }
        } else {
            out.push(Instruction::LocalGet(entry_local));
            let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
            Self::emit_typed_load(mti.key_vt, 4, key_align, out);
            out.push(Instruction::LocalGet(key_local));
            match mti.key_vt {
                ValType::I64 => out.push(Instruction::I64Eq),
                ValType::F64 => out.push(Instruction::F64Eq),
                ValType::F32 => out.push(Instruction::F32Eq),
                _ => out.push(Instruction::I32Eq),
            }
        }
    }

    pub(crate) fn emit_map_key_store(
        mti: &MapTypeInfo,
        entry_local: u32,
        key_local: u32,
        key_len_local: Option<u32>,
        out: &mut Vec<Instruction<'static>>,
    ) {
        if mti.is_string_key {
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_local));
            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_len_local.expect("key_len_local must be set for string-keyed maps")));
            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));
        } else {
            let (_, key_align) = Self::elem_size_and_align(mti.key_vt);
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(key_local));
            Self::emit_typed_store(mti.key_vt, 4, key_align, out);
        }
    }

    pub(crate) fn emit_map_val_store(
        mti: &MapTypeInfo,
        entry_local: u32,
        val_local: u32,
        val_len_local: Option<u32>,
        val_vt_actual: ValType,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        let val_offset = 4u64 + mti.key_size as u64;
        if mti.is_string_val {
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(val_local));
            out.push(Instruction::I32Store(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
            if let Some(vl) = val_len_local {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::LocalGet(vl));
                out.push(Instruction::I32Store(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
            }
        } else {
            let (_, val_align) = Self::elem_size_and_align(mti.val_vt);
            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::LocalGet(val_local));
            Self::emit_typed_coerce(val_vt_actual, mti.val_vt, out)?;
            Self::emit_typed_store(mti.val_vt, val_offset, val_align, out);
        }
        Ok(())
    }

    pub(crate) fn compile_map_get(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_vt = mti.key_vt;
        let val_vt = mti.val_vt;
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        // Compile key
        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
            }
            let kl = locals.add_local(&format!("__mg_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__mg_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_actual = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_actual, key_vt, out)?;
            key_local = locals.add_local(&format!("__mg_key_{}", locals.locals.len()), key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        let cap_local = locals.add_local(&format!("__mg_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__mg_data_{}", locals.locals.len()), ValType::I32);
        let i_local = locals.add_local(&format!("__mg_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__mg_e_{}", locals.locals.len()), ValType::I32);
        let found_local = locals.add_local(&format!("__mg_found_{}", locals.locals.len()), ValType::I32);
        let result_local = if mti.is_string_val {
            locals.add_local(&format!("__mg_res_{}", locals.locals.len()), ValType::I32)
        } else {
            locals.add_local(&format!("__mg_res_{}", locals.locals.len()), val_vt)
        };
        let result_len_local = if mti.is_string_val {
            Some(locals.add_local(&format!("__mg_reslen_{}", locals.locals.len()), ValType::I32))
        } else {
            None
        };

        // Nil map guard: skip lookup, result_local stays zero-initialized
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        // Load map header
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(found_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Compare key
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                let val_offset = 4u64 + key_size as u64;
                if mti.is_string_val {
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(result_local));
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(result_len_local.unwrap()));
                } else {
                    out.push(Instruction::LocalGet(entry_local));
                    let (_, val_align) = Self::elem_size_and_align(val_vt);
                    Self::emit_typed_load(val_vt, val_offset, val_align, out);
                    out.push(Instruction::LocalSet(result_local));
                }
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(found_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        out.push(Instruction::End); // end nil map guard

        out.push(Instruction::LocalGet(result_local));
        if mti.is_string_val {
            out.push(Instruction::LocalGet(result_len_local.unwrap()));
        }
        Ok(())
    }

    pub(crate) fn compile_map_get_ok(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        val_var: &str,
        ok_var: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_vt = mti.key_vt;
        let val_vt = mti.val_vt;
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
            }
            let kl = locals.add_local(&format!("__mgok_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__mgok_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_actual = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_actual, key_vt, out)?;
            key_local = locals.add_local(&format!("__mgok_key_{}", locals.locals.len()), key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        let cap_local = locals.add_local(&format!("__mgok_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__mgok_data_{}", locals.locals.len()), ValType::I32);
        let i_local = locals.add_local(&format!("__mgok_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__mgok_e_{}", locals.locals.len()), ValType::I32);

        let val_local = if mti.is_string_val {
            locals.add_local(val_var, ValType::I32)
        } else {
            locals.add_local(val_var, val_vt)
        };
        let val_len_local = if mti.is_string_val {
            let vl = locals.add_local(&format!("{}__str_len", val_var), ValType::I32);
            locals.set_var_struct_type(val_var, "__string");
            locals.string_locals.insert(val_var.to_string(), (val_local, vl));
            Some(vl)
        } else {
            None
        };
        let ok_local = locals.add_local(ok_var, ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(ok_local));

        // Nil map guard: skip lookup, val_local stays zero, ok stays 0
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                let val_offset = 4u64 + key_size as u64;
                if mti.is_string_val {
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(val_local));
                    out.push(Instruction::LocalGet(entry_local));
                    out.push(Instruction::I32Load(MemArg { offset: val_offset + 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalSet(val_len_local.unwrap()));
                } else {
                    out.push(Instruction::LocalGet(entry_local));
                    let (_, val_align) = Self::elem_size_and_align(val_vt);
                    Self::emit_typed_load(val_vt, val_offset, val_align, out);
                    out.push(Instruction::LocalSet(val_local));
                }
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        out.push(Instruction::End); // end nil map guard

        Ok(())
    }

    pub(crate) fn compile_map_set(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        val_already_in_tmp: u32,
        val_len_tmp: Option<u32>,
        val_vt_actual: ValType,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let key_size = mti.key_size;
        let entry_size = Self::map_entry_size(key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        // Nil map check: writing to a nil map panics
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            let msg = b"assignment to entry in nil map";
            let msg_len = msg.len() as i32;
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let msg_ptr = locals.add_local(
                &format!("__nil_map_ptr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(msg_ptr));
            for (i, &byte) in msg.iter().enumerate() {
                out.push(Instruction::LocalGet(msg_ptr));
                out.push(Instruction::I32Const(byte as i32));
                out.push(Instruction::I32Store8(MemArg {
                    offset: i as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            out.push(Instruction::LocalGet(msg_ptr));
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(0)); // ctx_log
            out.push(Instruction::Unreachable);
        }
        out.push(Instruction::End);

        // Compile key
        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
            }
            let kl = locals.add_local(&format!("__ms_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__ms_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_e = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_e, mti.key_vt, out)?;
            key_local = locals.add_local(&format!("__ms_key_{}", locals.locals.len()), mti.key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        // Load map header
        let cap_local = locals.add_local(&format!("__ms_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__ms_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__ms_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__ms_e_{}", locals.locals.len()), ValType::I32);
        let done_local = locals.add_local(&format!("__ms_done_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(done_local));

        // Pass 1: look for existing key
        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                Self::emit_map_val_store(&mti, entry_local, val_already_in_tmp, val_len_tmp, val_vt_actual, out)?;
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(done_local));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        // If not found, insert into first empty slot
        out.push(Instruction::LocalGet(done_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Resize if load factor >= 75%
            out.push(Instruction::LocalGet(map_local));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::I32Const(4));
            out.push(Instruction::I32Mul);
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32Const(3));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32GeU);
            out.push(Instruction::If(BlockType::Empty));
            {
                let new_cap_l = locals.add_local(&format!("__ms_ncap_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalGet(cap_local));
                out.push(Instruction::I32Const(2));
                out.push(Instruction::I32Mul);
                out.push(Instruction::LocalSet(new_cap_l));

                let new_dsz = locals.add_local(&format!("__ms_ndsz_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::I32Const(entry_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::LocalTee(new_dsz));

                out.push(Instruction::Call(self.alloc_func_idx()?));
                let new_data_l = locals.add_local(&format!("__ms_nd_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalTee(new_data_l));

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(new_dsz));
                out.push(Instruction::MemoryFill(0));

                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalGet(cap_local));
                out.push(Instruction::I32Const(entry_size));
                out.push(Instruction::I32Mul);
                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                out.push(Instruction::LocalGet(new_cap_l));
                out.push(Instruction::LocalSet(cap_local));
                out.push(Instruction::LocalGet(new_data_l));
                out.push(Instruction::LocalSet(data_local));
            }
            out.push(Instruction::End);

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(i_local));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::LocalGet(cap_local));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            out.push(Instruction::LocalGet(data_local));
            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::I32Const(entry_size));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(entry_local));

            out.push(Instruction::LocalGet(entry_local));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Ne);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                Self::emit_map_key_store(&mti, entry_local, key_local, key_len_local, out);
                Self::emit_map_val_store(&mti, entry_local, val_already_in_tmp, val_len_tmp, val_vt_actual, out)?;
                // Increment count
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);

            out.push(Instruction::LocalGet(i_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(i_local));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        Ok(())
    }

    pub(crate) fn compile_map_delete(
        &mut self,
        map_name: &str,
        key_expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let mti = locals.map_types.get(map_name).ok_or_else(|| {
            Error::InternalError(format!("map type info not found for '{}'", map_name))
        })?.clone();
        let entry_size = Self::map_entry_size(mti.key_size, mti.val_size) as i32;

        let map_local = locals.find(map_name).ok_or_else(|| {
            Error::InternalError(format!("map variable '{}' not found", map_name))
        })?;

        self.compile_expression(key_expr, out, locals)?;
        let key_local;
        let key_len_local;
        if mti.is_string_key {
            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
            }
            let kl = locals.add_local(&format!("__md_klen_{}", locals.locals.len()), ValType::I32);
            key_len_local = Some(kl);
            out.push(Instruction::LocalSet(kl));
            key_local = locals.add_local(&format!("__md_kptr_{}", locals.locals.len()), ValType::I32);
            out.push(Instruction::LocalSet(key_local));
        } else {
            let key_vt_e = self.infer_val_type(key_expr, locals);
            Self::emit_typed_coerce(key_vt_e, mti.key_vt, out)?;
            key_local = locals.add_local(&format!("__md_key_{}", locals.locals.len()), mti.key_vt);
            out.push(Instruction::LocalSet(key_local));
            key_len_local = None;
        }

        // Nil map guard: delete on nil map is a no-op per Go spec
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Else);

        let cap_local = locals.add_local(&format!("__md_cap_{}", locals.locals.len()), ValType::I32);
        let data_local = locals.add_local(&format!("__md_data_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(cap_local));
        out.push(Instruction::LocalGet(map_local));
        out.push(Instruction::I32Load(MemArg { offset: 8, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalSet(data_local));

        let i_local = locals.add_local(&format!("__md_i_{}", locals.locals.len()), ValType::I32);
        let entry_local = locals.add_local(&format!("__md_e_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(i_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::LocalGet(cap_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(entry_size));
        out.push(Instruction::I32Mul);
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(entry_local));

        out.push(Instruction::LocalGet(entry_local));
        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            self.emit_map_key_eq(&mti, entry_local, key_local, key_len_local, out, locals);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(entry_local));
                out.push(Instruction::I32Const(2));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::LocalGet(map_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::Br(3));
            }
            out.push(Instruction::End);
        }
        out.push(Instruction::End);

        out.push(Instruction::LocalGet(i_local));
        out.push(Instruction::I32Const(1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(i_local));
        out.push(Instruction::Br(0));
        out.push(Instruction::End); // loop
        out.push(Instruction::End); // block

        out.push(Instruction::End); // end nil map guard

        Ok(())
    }
}
