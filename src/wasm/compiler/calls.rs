use super::*;

impl WasmCompiler {
    pub(crate) fn compile_call(
        &mut self,
        call: &ast::Call,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        match call.func.as_ref() {
            ast::Expression::Ident(ident) => {
                match ident.name.as_str() {
                    "len" => {
                        self.compile_builtin_len(call, out, locals)?;
                        return Ok(GoType::Int64);
                    }
                    "make" => {
                        self.compile_builtin_make(call, out, locals)?;
                        return Ok(GoType::Slice(Box::new(GoType::Int32)));
                    }
                    "append" => {
                        self.compile_builtin_append(call, out, locals)?;
                        return Ok(GoType::Slice(Box::new(GoType::Int32)));
                    }
                    "cap" => {
                        self.compile_builtin_cap(call, out, locals)?;
                        return Ok(GoType::Int64);
                    }
                    "copy" => {
                        self.compile_builtin_copy(call, out, locals)?;
                        return Ok(GoType::Int64);
                    }
                    "Float64frombits" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            out.push(Instruction::F64ReinterpretI64);
                        }
                        return Ok(GoType::Float64);
                    }
                    "Float64bits" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            out.push(Instruction::I64ReinterpretF64);
                        }
                        return Ok(GoType::Uint64);
                    }
                    "Float32frombits" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let arg_vt = self.infer_val_type(arg, locals);
                            if arg_vt == ValType::I64 {
                                out.push(Instruction::I32WrapI64);
                            }
                            out.push(Instruction::F32ReinterpretI32);
                        }
                        return Ok(GoType::Float32);
                    }
                    "Float32bits" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let arg_vt = self.infer_val_type(arg, locals);
                            if arg_vt == ValType::F64 {
                                out.push(Instruction::F32DemoteF64);
                            }
                            out.push(Instruction::I32ReinterpretF32);
                        }
                        return Ok(GoType::Int32);
                    }
                    "panic" => {
                        if let Some(arg) = call.args.first() {
                            let str_ptr_local = locals.add_local(
                                &format!("__panic_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let str_len_local = locals.add_local(
                                &format!("__panic_len_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            self.emit_expr_to_string_on_stack(arg, out, locals)?;
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                            }
                            out.push(Instruction::LocalSet(str_len_local));
                            out.push(Instruction::LocalSet(str_ptr_local));

                            out.push(Instruction::LocalGet(str_ptr_local));
                            out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
                            out.push(Instruction::LocalGet(str_len_local));
                            out.push(Instruction::GlobalSet(self.panic_value_len_global));

                            out.push(Instruction::LocalGet(str_ptr_local));
                            out.push(Instruction::LocalGet(str_len_local));
                            out.push(Instruction::Call(0)); // ctx_log
                        }

                        // Set panicking flag
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::GlobalSet(self.panicking_global));

                        // Run deferred calls so recover() can clear the flag
                        self.emit_deferred_calls(out);

                        // If still panicking (no recover), trap
                        out.push(Instruction::GlobalGet(self.panicking_global));
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::Unreachable);
                        out.push(Instruction::End);

                        // Recovered: push zero return values and return
                        for rt in &self.current_result_types.clone() {
                            match rt {
                                ValType::I32 => out.push(Instruction::I32Const(0)),
                                ValType::I64 => out.push(Instruction::I64Const(0)),
                                ValType::F32 => out.push(Instruction::F32Const(0.0_f32.into())),
                                ValType::F64 => out.push(Instruction::F64Const(0.0_f64.into())),
                                ValType::Ref(ref_type) => {
                                    // #region agent log
                                    {
                                        use std::io::Write;
                                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                                            let _ = writeln!(f, r#"{{"hypothesisId":"A","location":"calls.rs:panic_recovery","message":"emitting ref.null for Ref return type","data":{{"heap_type":"{:?}","pkg":"{}"}},"timestamp":{}}}"#,
                                                ref_type.heap_type, self.current_package.as_deref().unwrap_or(""),
                                                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                                        }
                                    }
                                    // #endregion
                                    out.push(Instruction::RefNull(ref_type.heap_type));
                                }
                                _ => out.push(Instruction::I32Const(0)),
                            }
                        }
                        out.push(Instruction::Return);
                        return Ok(GoType::Void);
                    }
                    "recover" => {
                        let rec_ptr = locals.add_local(
                            &format!("__recover_ptr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let rec_len = locals.add_local(
                            &format!("__recover_len_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(rec_ptr));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(rec_len));
                        out.push(Instruction::GlobalGet(self.panicking_global));
                        out.push(Instruction::If(BlockType::Empty));
                        out.push(Instruction::GlobalGet(self.panic_value_ptr_global));
                        out.push(Instruction::LocalSet(rec_ptr));
                        out.push(Instruction::GlobalGet(self.panic_value_len_global));
                        out.push(Instruction::LocalSet(rec_len));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panicking_global));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::GlobalSet(self.panic_value_len_global));
                        out.push(Instruction::End);
                        out.push(Instruction::LocalGet(rec_ptr));
                        out.push(Instruction::LocalGet(rec_len));
                        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                            self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                        }
                        return Ok(GoType::String);
                    }
                    "int" | "int64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::F64 => out.push(Instruction::I64TruncF64S),
                                ValType::F32 => out.push(Instruction::I64TruncF32S),
                                ValType::I32 => out.push(if is_unsigned { Instruction::I64ExtendI32U } else { Instruction::I64ExtendI32S }),
                                ValType::I64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Int64);
                    }
                    "float64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::I64 => out.push(if is_unsigned { Instruction::F64ConvertI64U } else { Instruction::F64ConvertI64S }),
                                ValType::I32 => out.push(if is_unsigned { Instruction::F64ConvertI32U } else { Instruction::F64ConvertI32S }),
                                ValType::F32 => out.push(Instruction::F64PromoteF32),
                                ValType::F64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for float64 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Float64);
                    }
                    "float32" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            let is_unsigned = self.is_unsigned_expr(arg, locals);
                            match vt {
                                ValType::I64 => out.push(if is_unsigned { Instruction::F32ConvertI64U } else { Instruction::F32ConvertI64S }),
                                ValType::I32 => out.push(if is_unsigned { Instruction::F32ConvertI32U } else { Instruction::F32ConvertI32S }),
                                ValType::F64 => out.push(Instruction::F32DemoteF64),
                                ValType::F32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for float32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Float32);
                    }
                    "int32" | "rune" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Int32);
                    }
                    "int8" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int8 conversion",
                                        vt
                                    )));
                                }
                            }
                            // Sign-extend from 8 bits
                            out.push(Instruction::I32Const(24));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Const(24));
                            out.push(Instruction::I32ShrS);
                        }
                        return Ok(GoType::Int8);
                    }
                    "int16" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::F64 => out.push(Instruction::I32TruncF64S),
                                ValType::F32 => out.push(Instruction::I32TruncF32S),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for int16 conversion",
                                        vt
                                    )));
                                }
                            }
                            // Sign-extend from 16 bits
                            out.push(Instruction::I32Const(16));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Const(16));
                            out.push(Instruction::I32ShrS);
                        }
                        return Ok(GoType::Int16);
                    }
                    "byte" | "uint8" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for byte/uint8 conversion",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::I32Const(0xFF));
                            out.push(Instruction::I32And);
                        }
                        return Ok(GoType::Uint8);
                    }
                    "uint16" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint16 conversion",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::I32Const(0xFFFF));
                            out.push(Instruction::I32And);
                        }
                        return Ok(GoType::Uint16);
                    }
                    "uint32" | "uintptr" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                ValType::F64 => out.push(Instruction::I32TruncF64U),
                                ValType::F32 => out.push(Instruction::I32TruncF32U),
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint32 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Uint32);
                    }
                    "uint" | "uint64" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let vt = self.infer_val_type(arg, locals);
                            match vt {
                                ValType::F64 => out.push(Instruction::I64TruncF64U),
                                ValType::F32 => out.push(Instruction::I64TruncF32U),
                                ValType::I32 => out.push(Instruction::I64ExtendI32U),
                                ValType::I64 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "unsupported source type {:?} for uint/uint64 conversion",
                                        vt
                                    )));
                                }
                            }
                        }
                        return Ok(GoType::Uint64);
                    }
                    "bool" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                        }
                        return Ok(GoType::Bool);
                    }
                    "string" => {
                        if let Some(arg) = call.args.first() {
                            let vt = self.infer_val_type(arg, locals);
                            let is_gc_string = self.gc_builtin_types.go_string
                                .map_or(false, |idx| vt == Self::gc_ref_val_type(idx));
                            if (vt == ValType::I32 || is_gc_string)
                                && self.is_string_expr(arg, locals)
                            {
                                self.compile_expression(arg, out, locals)?;
                                return Ok(GoType::String);
                            }
                            // Check if arg is a []rune slice => string([]rune)
                            if let ast::Expression::Ident(arg_ident) = arg {
                                if locals.rune_slices.contains(&arg_ident.name)
                                    && locals.get_var_struct_type(&arg_ident.name) == Some("__slice")
                                {
                                    let hdr_idx = locals.find(&arg_ident.name).ok_or_else(|| {
                                        Error::InternalError(format!(
                                            "local variable '{}' not found for []rune to string conversion", arg_ident.name
                                        ))
                                    })?;
                                    let src_ptr = locals.add_local(&format!("__sr2s_sp_{}", locals.locals.len()), ValType::I32);
                                    let s_len = locals.add_local(&format!("__sr2s_ln_{}", locals.locals.len()), ValType::I32);
                                    let dst_ptr = locals.add_local(&format!("__sr2s_dp_{}", locals.locals.len()), ValType::I32);
                                    let loop_i = locals.add_local(&format!("__sr2s_i_{}", locals.locals.len()), ValType::I32);
                                    let cur_rune = locals.add_local(&format!("__sr2s_r_{}", locals.locals.len()), ValType::I32);
                                    let write_pos = locals.add_local(&format!("__sr2s_wp_{}", locals.locals.len()), ValType::I32);

                                    // Read slice header
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(src_ptr));
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(s_len));

                                    // Allocate max possible bytes: len * 4 (worst case UTF-8)
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(dst_ptr));

                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(write_pos));

                                    out.push(Instruction::Block(BlockType::Empty));
                                    out.push(Instruction::Loop(BlockType::Empty));
                                    // if loop_i >= s_len, break
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32GeU);
                                    out.push(Instruction::BrIf(1));

                                    // cur_rune = src[loop_i * 4]
                                    out.push(Instruction::LocalGet(src_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(cur_rune));

                                    // UTF-8 encode cur_rune into dst_ptr + write_pos
                                    // 1-byte: rune < 0x80
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x80));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(1));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    // 2-byte: rune < 0x800
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x800));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xC0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(2));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    // 3-byte: rune < 0x10000
                                    out.push(Instruction::LocalGet(cur_rune));
                                    out.push(Instruction::I32Const(0x10000));
                                    out.push(Instruction::I32LtU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xE0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(12));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(3));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        // 4-byte
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0xF0));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(18));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(12));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32ShrU);
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(dst_ptr));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::I32Const(0x80));
                                        out.push(Instruction::LocalGet(cur_rune));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::I32Store8(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(write_pos));
                                        out.push(Instruction::I32Const(4));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalSet(write_pos));
                                    }
                                    out.push(Instruction::End); // 3-byte vs 4-byte
                                    out.push(Instruction::End); // 2-byte vs rest
                                    out.push(Instruction::End); // 1-byte vs rest

                                    // loop_i++
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::Br(0));
                                    out.push(Instruction::End); // loop
                                    out.push(Instruction::End); // block

                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(write_pos));
                                    if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                        self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                                    }
                                    return Ok(GoType::String);
                                }
                            }
                            // Check if arg is a []byte slice => string([]byte)
                            if let ast::Expression::Ident(arg_ident) = arg {
                                if locals.slice_elem_types.get(&arg_ident.name) == Some(&ValType::I32)
                                    && locals.get_var_struct_type(&arg_ident.name) == Some("__slice")
                                {
                                    // Pack I32 slice elements back into compact bytes
                                    let hdr_idx = locals.find(&arg_ident.name).ok_or_else(|| {
                                        Error::InternalError(format!(
                                            "local variable '{}' not found for []byte to string conversion", arg_ident.name
                                        ))
                                    })?;
                                    let src_ptr = locals.add_local(&format!("__s2b_sp_{}", locals.locals.len()), ValType::I32);
                                    let s_len = locals.add_local(&format!("__s2b_ln_{}", locals.locals.len()), ValType::I32);
                                    let dst_ptr = locals.add_local(&format!("__s2b_dp_{}", locals.locals.len()), ValType::I32);
                                    let loop_i = locals.add_local(&format!("__s2b_i_{}", locals.locals.len()), ValType::I32);

                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(src_ptr));
                                    out.push(Instruction::LocalGet(hdr_idx));
                                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(s_len));

                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(dst_ptr));

                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::LocalSet(loop_i));

                                    out.push(Instruction::Block(BlockType::Empty));
                                    out.push(Instruction::Loop(BlockType::Empty));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::LocalGet(s_len));
                                    out.push(Instruction::I32GeU);
                                    out.push(Instruction::BrIf(1));

                                    // dst[i] = (byte) src[i*4]
                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalGet(src_ptr));
                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(4));
                                    out.push(Instruction::I32Mul);
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

                                    out.push(Instruction::LocalGet(loop_i));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(loop_i));
                                    out.push(Instruction::Br(0));
                                    out.push(Instruction::End);
                                    out.push(Instruction::End);

                                    out.push(Instruction::LocalGet(dst_ptr));
                                    out.push(Instruction::LocalGet(s_len));
                                    if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                        self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                                    }
                                    return Ok(GoType::String);
                                }
                            }
                            self.compile_expression(arg, out, locals)?;
                            let rune_local = locals.add_local(
                                &format!("__rune_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            match vt {
                                ValType::I64 => out.push(Instruction::I32WrapI64),
                                ValType::I32 => {}
                                _ => {
                                    return Err(Error::InternalError(format!(
                                        "string() conversion from {:?} is not supported",
                                        vt
                                    )));
                                }
                            }
                            out.push(Instruction::LocalSet(rune_local));

                            let buf_local = locals.add_local(
                                &format!("__rune_buf_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let len_local = locals.add_local(
                                &format!("__rune_len_{}", locals.locals.len()),
                                ValType::I32,
                            );

                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(buf_local));

                            // Replace invalid runes with U+FFFD:
                            // surrogates (0xD800..0xDFFF) or values >= 0x110000
                            {
                                let is_invalid = locals.add_local(
                                    &format!("__rune_inv_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                // Check >= 0x110000
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x110000));
                                out.push(Instruction::I32GeU);
                                out.push(Instruction::LocalSet(is_invalid));
                                // Check surrogate range: rune >= 0xD800 && rune < 0xE000
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0xD800u32 as i32));
                                out.push(Instruction::I32GeU);
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0xE000u32 as i32));
                                out.push(Instruction::I32LtU);
                                out.push(Instruction::I32And);
                                out.push(Instruction::LocalGet(is_invalid));
                                out.push(Instruction::I32Or);
                                out.push(Instruction::If(BlockType::Empty));
                                out.push(Instruction::I32Const(0xFFFD));
                                out.push(Instruction::LocalSet(rune_local));
                                out.push(Instruction::End);
                            }

                            // UTF-8 encode: 1-byte (0..0x80), 2-byte (0x80..0x800),
                            // 3-byte (0x800..0x10000), 4-byte (0x10000..0x110000)
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x80));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 1-byte: buf[0] = rune
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(1));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x800));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 2-byte: buf[0] = 0xC0 | (rune >> 6), buf[1] = 0x80 | (rune & 0x3F)
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xC0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(2));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            out.push(Instruction::LocalGet(rune_local));
                            out.push(Instruction::I32Const(0x10000));
                            out.push(Instruction::I32LtU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 3-byte
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xE0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(3));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::Else);
                            {
                                // 4-byte
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0xF0));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(18));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32ShrU);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::I32Const(0x80));
                                out.push(Instruction::LocalGet(rune_local));
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::I32Store8(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                out.push(Instruction::I32Const(4));
                                out.push(Instruction::LocalSet(len_local));
                            }
                            out.push(Instruction::End); // closes if/else (3-byte vs 4-byte)
                            out.push(Instruction::End); // closes if/else (2-byte vs rest)
                            out.push(Instruction::End); // closes if/else (1-byte vs rest)

                            out.push(Instruction::LocalGet(buf_local));
                            out.push(Instruction::LocalGet(len_local));
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                            }
                            return Ok(GoType::String);
                        }
                        return Ok(GoType::String);
                    }
                    "new" => {
                        if let Some(type_arg) = call.args.first() {
                            let gc_type = if let ast::Expression::Ident(ti) = type_arg {
                                self.struct_defs.get(&ti.name).and_then(|sd| sd.gc_type_idx)
                            } else {
                                None
                            };

                            if let Some(gc_idx) = gc_type {
                                out.push(Instruction::StructNewDefault(gc_idx));
                            } else {
                                let alloc_size = if let ast::Expression::Ident(ti) = type_arg {
                                    if let Some(sdef) = self.struct_defs.get(&ti.name) {
                                        sdef.total_size
                                    } else {
                                        match ti.name.as_str() {
                                            "int" | "int64" | "uint" | "uint64" | "float64" => 8,
                                            "int32" | "uint32" | "float32" | "rune" => 4,
                                            "int16" | "uint16" => 2,
                                            "int8" | "uint8" | "byte" | "bool" => 1,
                                            _ => 8,
                                        }
                                    }
                                } else {
                                    8u32
                                };
                                let alloc_aligned = alloc_size.max(8);
                                let used_stack = if let Some(ref target) = self.stack_alloc_target.take() {
                                    if let Some(sf) = &self.current_stack_frame {
                                        if let Some(sl) = sf.find(target) {
                                            if let Some(fb) = sf.frame_base_local {
                                                out.push(Instruction::LocalGet(fb));
                                                if sl.offset > 0 {
                                                    out.push(Instruction::I32Const(sl.offset as i32));
                                                    out.push(Instruction::I32Add);
                                                }
                                                true
                                            } else { false }
                                        } else { false }
                                    } else { false }
                                } else { false };
                                if !used_stack {
                                    out.push(Instruction::I32Const(alloc_aligned as i32));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                }
                                let ptr_local = locals.add_local(
                                    &format!("__new_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalTee(ptr_local));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Const(alloc_aligned as i32));
                                out.push(Instruction::MemoryFill(0));
                                out.push(Instruction::LocalGet(ptr_local));
                            }
                        }
                        return Ok(GoType::Pointer(Box::new(GoType::Int32)));
                    }
                    "complex" => {
                        if call.args.len() < 2 {
                            return Err(Error::InternalError(
                                "complex() requires two arguments".to_string(),
                            ));
                        }
                        let r_vt = self.infer_val_type(&call.args[0], locals);
                        let is_complex64 = r_vt == ValType::F32;
                        let gc_idx = if is_complex64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };

                        self.compile_expression(&call.args[0], out, locals)?;
                        let real_local = locals.add_local(
                            &format!("__cplx_r_{}", locals.locals.len()),
                            if is_complex64 { ValType::F32 } else { ValType::F64 },
                        );
                        out.push(Instruction::LocalSet(real_local));

                        self.compile_expression(&call.args[1], out, locals)?;
                        if !is_complex64 && self.infer_val_type(&call.args[1], locals) == ValType::F32 {
                            out.push(Instruction::F64PromoteF32);
                        }
                        let imag_local = locals.add_local(
                            &format!("__cplx_i_{}", locals.locals.len()),
                            if is_complex64 { ValType::F32 } else { ValType::F64 },
                        );
                        out.push(Instruction::LocalSet(imag_local));

                        if let Some(gc_idx) = gc_idx {
                            out.push(Instruction::LocalGet(real_local));
                            out.push(Instruction::LocalGet(imag_local));
                            out.push(Instruction::StructNew(gc_idx));
                        } else {
                            let total_size: i32 = if is_complex64 { 8 } else { 16 };
                            let float_align: u32 = if is_complex64 { 2 } else { 3 };

                            out.push(Instruction::I32Const(total_size));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let ptr = locals.add_local(
                                &format!("__cplx_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalSet(ptr));

                            out.push(Instruction::LocalGet(ptr));
                            out.push(Instruction::LocalGet(real_local));
                            if is_complex64 {
                                out.push(Instruction::F32Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                            } else {
                                out.push(Instruction::F64Store(MemArg { offset: 0, align: float_align, memory_index: 0 }));
                            }

                            let imag_offset = if is_complex64 { 4u64 } else { 8u64 };
                            out.push(Instruction::LocalGet(ptr));
                            out.push(Instruction::LocalGet(imag_local));
                            if is_complex64 {
                                out.push(Instruction::F32Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                            } else {
                                out.push(Instruction::F64Store(MemArg { offset: imag_offset, align: float_align, memory_index: 0 }));
                            }

                            out.push(Instruction::LocalGet(ptr));
                        }
                        return Ok(GoType::Complex128);
                    }
                    "real" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let is_64 = self.is_complex64_expr(arg, locals);
                            let gc_idx = if is_64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };
                            if let Some(gc_idx) = gc_idx {
                                out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 0 });
                            } else {
                                let align = if is_64 { 2u32 } else { 3u32 };
                                if is_64 {
                                    out.push(Instruction::F32Load(MemArg { offset: 0, align, memory_index: 0 }));
                                } else {
                                    out.push(Instruction::F64Load(MemArg { offset: 0, align, memory_index: 0 }));
                                }
                            }
                        }
                        return Ok(GoType::Float64);
                    }
                    "imag" => {
                        if let Some(arg) = call.args.first() {
                            self.compile_expression(arg, out, locals)?;
                            let is_64 = self.is_complex64_expr(arg, locals);
                            let gc_idx = if is_64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };
                            if let Some(gc_idx) = gc_idx {
                                out.push(Instruction::StructGet { struct_type_index: gc_idx, field_index: 1 });
                            } else {
                                let (imag_offset, align) = if is_64 { (4u64, 2u32) } else { (8u64, 3u32) };
                                if is_64 {
                                    out.push(Instruction::F32Load(MemArg { offset: imag_offset, align, memory_index: 0 }));
                                } else {
                                    out.push(Instruction::F64Load(MemArg { offset: imag_offset, align, memory_index: 0 }));
                                }
                            }
                        }
                        return Ok(GoType::Float64);
                    }
                    "min" | "max" => {
                        let is_min = ident.name == "min";
                        if call.args.len() >= 2 && self.is_string_expr(&call.args[0], locals) {
                            self.emit_string_min_max(&call.args, is_min, out, locals)?;
                        } else if call.args.len() >= 2 {
                            self.compile_expression(&call.args[0], out, locals)?;
                            let vt = self.infer_val_type(&call.args[0], locals);
                            let is_unsigned = self.is_unsigned_expr(&call.args[0], locals);
                            for arg in &call.args[1..] {
                                self.compile_expression(arg, out, locals)?;
                                let arg_vt = self.infer_val_type(arg, locals);
                                Self::emit_typed_coerce(arg_vt, vt, out)?;
                                match vt {
                                    ValType::F64 => out.push(if is_min { Instruction::F64Min } else { Instruction::F64Max }),
                                    ValType::F32 => out.push(if is_min { Instruction::F32Min } else { Instruction::F32Max }),
                                    _ => {
                                        let tmp_a = locals.add_local(
                                            &format!("__mm_a_{}", locals.locals.len()),
                                            vt,
                                        );
                                        let tmp_b = locals.add_local(
                                            &format!("__mm_b_{}", locals.locals.len()),
                                            vt,
                                        );
                                        out.push(Instruction::LocalSet(tmp_b));
                                        out.push(Instruction::LocalSet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_b));
                                        out.push(Instruction::LocalGet(tmp_a));
                                        out.push(Instruction::LocalGet(tmp_b));
                                        match (vt, is_min) {
                                            (ValType::I64, true) => out.push(if is_unsigned { Instruction::I64LeU } else { Instruction::I64LeS }),
                                            (ValType::I64, false) => out.push(if is_unsigned { Instruction::I64GeU } else { Instruction::I64GeS }),
                                            (_, true) => out.push(if is_unsigned { Instruction::I32LeU } else { Instruction::I32LeS }),
                                            (_, false) => out.push(if is_unsigned { Instruction::I32GeU } else { Instruction::I32GeS }),
                                        }
                                        out.push(Instruction::Select);
                                    }
                                }
                            }
                        } else if call.args.len() == 1 {
                            self.compile_expression(&call.args[0], out, locals)?;
                        }
                        let mm_vt = if call.args.is_empty() { GoType::Int32 } else { GoType::from_val_type(self.infer_val_type(&call.args[0], locals)) };
                        return Ok(mm_vt);
                    }
                    "println" | "print" => {
                        let is_println = ident.name == "println";
                        if call.args.is_empty() {
                            if is_println {
                                // println() with no args: emit a newline
                                let nl_ptr = locals.add_local("__pln_nl_ptr", ValType::I32);
                                out.push(Instruction::I32Const(1));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                out.push(Instruction::LocalTee(nl_ptr));
                                out.push(Instruction::I32Const(b'\n' as i32));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                out.push(Instruction::LocalGet(nl_ptr));
                                out.push(Instruction::I32Const(1));
                            } else {
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Const(0));
                            }
                        } else if call.args.len() == 1 {
                            self.emit_expr_to_string_on_stack(&call.args[0], out, locals)?;
                            if is_println {
                                self.emit_append_newline(out, locals)?;
                            }
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                            }
                        } else {
                            // Multiple args: convert each to string, store (ptr,len) pairs
                            let n = call.args.len();
                            let mut arg_ptrs = Vec::with_capacity(n);
                            let mut arg_lens = Vec::with_capacity(n);
                            for (i, arg) in call.args.iter().enumerate() {
                                self.emit_expr_to_string_on_stack(arg, out, locals)?;
                                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                    self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                                }
                                let l = locals.add_local(&format!("__pln_len_{i}"), ValType::I32);
                                let p = locals.add_local(&format!("__pln_ptr_{i}"), ValType::I32);
                                out.push(Instruction::LocalSet(l));
                                out.push(Instruction::LocalSet(p));
                                arg_ptrs.push(p);
                                arg_lens.push(l);
                            }

                            let separator = if is_println { b' ' } else { 0u8 };
                            let has_sep = is_println;
                            let num_seps = if has_sep { n - 1 } else { 0 };
                            let nl_extra: u32 = if is_println { 1 } else { 0 };

                            // Calculate total length
                            let total = locals.add_local("__pln_total", ValType::I32);
                            out.push(Instruction::I32Const(0));
                            for &l in &arg_lens {
                                out.push(Instruction::LocalGet(l));
                                out.push(Instruction::I32Add);
                            }
                            out.push(Instruction::I32Const((num_seps as i32) + (nl_extra as i32)));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(total));

                            // Allocate buffer
                            let buf = locals.add_local("__pln_buf", ValType::I32);
                            out.push(Instruction::LocalGet(total));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(buf));

                            // Copy each string with separators
                            let offset_local = locals.add_local("__pln_off", ValType::I32);
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(offset_local));

                            for i in 0..n {
                                // Insert space separator before args after the first (println only)
                                if has_sep && i > 0 {
                                    out.push(Instruction::LocalGet(buf));
                                    out.push(Instruction::LocalGet(offset_local));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::I32Const(separator as i32));
                                    out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(offset_local));
                                    out.push(Instruction::I32Const(1));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalSet(offset_local));
                                }
                                // memory.copy(buf + offset, ptr_i, len_i)
                                out.push(Instruction::LocalGet(buf));
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalGet(arg_ptrs[i]));
                                out.push(Instruction::LocalGet(arg_lens[i]));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
                                // offset += len_i
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::LocalGet(arg_lens[i]));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalSet(offset_local));
                            }

                            if is_println {
                                // Append newline
                                out.push(Instruction::LocalGet(buf));
                                out.push(Instruction::LocalGet(offset_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::I32Const(b'\n' as i32));
                                out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            }

                            out.push(Instruction::LocalGet(buf));
                            out.push(Instruction::LocalGet(total));
                        }
                        out.push(Instruction::Call(0)); // ctx_log is import index 0
                        return Ok(GoType::Void);
                    }
                    "close" => {
                        return Err(Error::InternalError(
                            "close() is not supported in WASM UDFs (channels are not available)".to_string(),
                        ));
                    }
                    "delete" => {
                        if call.args.len() < 2 {
                            return Err(Error::InternalError(
                                "delete() requires 2 arguments: map and key".to_string(),
                            ));
                        }
                        let key_expr = call.args[1].clone();
                        match &call.args[0] {
                            ast::Expression::Ident(id) => {
                                self.compile_map_delete(&id.name, &key_expr, out, locals)?;
                            }
                            ast::Expression::Selector(sel) => {
                                if let ast::Expression::Ident(recv) = sel.x.as_ref() {
                                    let synth = format!("{}.{}", recv.name, sel.sel.name);
                                    if locals.map_types.contains_key(&synth) {
                                        self.compile_map_delete(&synth, &key_expr, out, locals)?;
                                    } else if self.is_selector_map_field(sel, locals) {
                                        if let Some(parent_type) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                                            if let Some(mti) = self.struct_field_map_types.get(&(parent_type, sel.sel.name.clone())).cloned() {
                                                self.compile_expression(&call.args[0], out, locals)?;
                                                let tmp_name = format!("__del_map_tmp_{}", locals.locals.len());
                                                let tmp_local = locals.add_local(&tmp_name, ValType::I32);
                                                out.push(Instruction::LocalSet(tmp_local));
                                                locals.set_var_struct_type(&tmp_name, "__map");
                                                locals.map_types.insert(tmp_name.clone(), mti);
                                                self.compile_map_delete(&tmp_name, &key_expr, out, locals)?;
                                            } else {
                                                return Err(Error::InternalError(
                                                    "delete() first argument: map type info not found for struct field".to_string(),
                                                ));
                                            }
                                        } else {
                                            return Err(Error::InternalError(
                                                "delete() first argument: could not resolve struct type for selector".to_string(),
                                            ));
                                        }
                                    } else {
                                        return Err(Error::InternalError(
                                            "delete() first argument: map type info not found for selector expression".to_string(),
                                        ));
                                    }
                                } else {
                                    return Err(Error::InternalError(
                                        "delete() first argument must be a map variable or field selector".to_string(),
                                    ));
                                }
                            }
                            _ => {
                                return Err(Error::InternalError(
                                    "delete() first argument must be a map variable".to_string(),
                                ));
                            }
                        }
                        return Ok(GoType::Void);
                    }
                    "clear" => {
                        if let Some(arg) = call.args.first() {
                            if let ast::Expression::Selector(_sel) = arg {
                                self.compile_expression(arg, out, locals)?;
                                let hdr_local = locals.add_local(
                                    &format!("__clr_sel_hdr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(hdr_local));

                                let sel_expr = arg;
                                let sel_elem_vt = if let ast::Expression::Selector(sel) = sel_expr {
                                    if let ast::Expression::Ident(recv_id) = sel.x.as_ref() {
                                        locals.slice_elem_types.get(&format!("{}.{}", recv_id.name, sel.sel.name))
                                            .or_else(|| locals.slice_elem_types.get(&recv_id.name))
                                            .copied()
                                            .unwrap_or(ValType::I64)
                                    } else {
                                        ValType::I64
                                    }
                                } else {
                                    ValType::I64
                                };
                                let (sel_elem_size, _) = Self::elem_size_and_align(sel_elem_vt);

                                // nil check: skip if header pointer is 0
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Ne);
                                out.push(Instruction::If(BlockType::Empty));
                                // memory.fill(data_ptr, 0, len * elem_size)
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(hdr_local));
                                out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                out.push(Instruction::I32Const(sel_elem_size));
                                out.push(Instruction::I32Mul);
                                out.push(Instruction::MemoryFill(0));
                                out.push(Instruction::End);
                                return Ok(GoType::Void);
                            }
                            if let ast::Expression::Ident(ident_arg) = arg {
                                if let Some(local_idx) = locals.find(&ident_arg.name) {
                                    let struct_type = locals.get_var_struct_type(&ident_arg.name);
                                    if struct_type == Some("__slice") || locals.slice_elem_types.contains_key(&ident_arg.name) {
                                        let elem_vt = locals.slice_elem_types.get(&ident_arg.name).copied().unwrap_or(ValType::I64);
                                        let (elem_size, _) = Self::elem_size_and_align(elem_vt);
                                        // nil check: skip if slice header pointer is 0
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Ne);
                                        out.push(Instruction::If(BlockType::Empty));
                                        // memory.fill(data_ptr, 0, len * elem_size)
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                        out.push(Instruction::I32Const(elem_size));
                                        out.push(Instruction::I32Mul);
                                        out.push(Instruction::MemoryFill(0));
                                        out.push(Instruction::End);
                                        return Ok(GoType::Void);
                                    }
                                    if struct_type == Some("__map") {
                                        // nil check: skip if map pointer is 0
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Ne);
                                        out.push(Instruction::If(BlockType::Empty));
                                        // Zero out count field (offset 0)
                                        out.push(Instruction::LocalGet(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Store(MemArg {
                                            offset: 0,
                                            align: 2,
                                            memory_index: 0,
                                        }));
                                        // Zero out all entries in the data array
                                        let map_info = locals.map_types.get(&ident_arg.name).cloned();
                                        if let Some(info) = map_info {
                                            let entry_size = Self::map_entry_size(info.key_size, info.val_size);
                                            let cap_tmp = locals.add_local(
                                                &format!("__clr_cap_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Load(MemArg {
                                                offset: 4,
                                                align: 2,
                                                memory_index: 0,
                                            }));
                                            out.push(Instruction::LocalSet(cap_tmp));

                                            let data_ptr_tmp = locals.add_local(
                                                &format!("__clr_dptr_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Load(MemArg {
                                                offset: 8,
                                                align: 2,
                                                memory_index: 0,
                                            }));
                                            out.push(Instruction::LocalSet(data_ptr_tmp));

                                            let total_tmp = locals.add_local(
                                                &format!("__clr_tot_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalGet(cap_tmp));
                                            out.push(Instruction::I32Const(entry_size as i32));
                                            out.push(Instruction::I32Mul);
                                            out.push(Instruction::LocalSet(total_tmp));

                                            let loop_i = locals.add_local(
                                                &format!("__clr_i_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::LocalSet(loop_i));

                                            out.push(Instruction::Block(BlockType::Empty));
                                            out.push(Instruction::Loop(BlockType::Empty));

                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::LocalGet(total_tmp));
                                            out.push(Instruction::I32GeU);
                                            out.push(Instruction::BrIf(1));

                                            out.push(Instruction::LocalGet(data_ptr_tmp));
                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::I32Add);
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::I32Store8(MemArg {
                                                offset: 0,
                                                align: 0,
                                                memory_index: 0,
                                            }));

                                            out.push(Instruction::LocalGet(loop_i));
                                            out.push(Instruction::I32Const(1));
                                            out.push(Instruction::I32Add);
                                            out.push(Instruction::LocalSet(loop_i));
                                            out.push(Instruction::Br(0));

                                            out.push(Instruction::End); // loop
                                            out.push(Instruction::End); // block
                                        }
                                        out.push(Instruction::End); // if (nil check)
                                        return Ok(GoType::Void);
                                    }
                                    if struct_type == Some("__array") {
                                        if let Some(&(_elem_vt, arr_len, go_es, _)) = locals.array_info.get(&ident_arg.name) {
                                            let elem_size = go_es;
                                            let total_bytes = elem_size as i32 * arr_len as i32;
                                            out.push(Instruction::LocalGet(local_idx));
                                            out.push(Instruction::I32Const(0));
                                            out.push(Instruction::I32Const(total_bytes));
                                            out.push(Instruction::MemoryFill(0));
                                            return Ok(GoType::Void);
                                        }
                                    }
                                }
                            }
                        }
                        return Err(Error::InternalError(
                            "clear() is currently only supported on slices, maps, and arrays".to_string(),
                        ));
                    }
                    _ => {}
                }

                // Check if it's a function-typed parameter (call_indirect)
                if let Some(ftp) = locals.func_typed_params.get(&ident.name).cloned() {
                    out.push(Instruction::LocalGet(ftp.env_ptr_local));
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::LocalGet(ftp.func_idx_local));
                    out.push(Instruction::CallIndirect {
                        type_index: ftp.call_type_idx,
                        table_index: 0,
                    });
                    return Ok(GoType::Int32);
                }

                // Check if it's a closure variable or method expression
                if let Some(&(func_idx, env_local)) =
                    locals.closure_info.get(&ident.name)
                {
                    let is_method_expr = locals.method_expr_vars.contains(&ident.name);

                    if is_method_expr {
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(func_idx));
                        return Ok(GoType::Int32);
                    }

                    let cap_info = locals.closure_env_captures.get(&ident.name).cloned().unwrap_or_default();

                    if env_local != u32::MAX && !cap_info.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_info {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(env_local));
                                out.push(Instruction::LocalGet(outer_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_store(*vt, *env_offset as u64, align, out);
                            }
                        }
                    }

                    if env_local != u32::MAX {
                        out.push(Instruction::LocalGet(env_local));
                    } else {
                        out.push(Instruction::I32Const(0));
                    }
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));

                    if env_local != u32::MAX && !cap_info.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_info {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(env_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_load(*vt, *env_offset as u64, align, out);
                                out.push(Instruction::LocalSet(outer_local));
                            }
                        }
                    }

                    return Ok(GoType::Int32);
                }

                // Look up as a user function (or stdlib function within package context)
                let fi_lookup = self.find_func_in_pkg(&ident.name).cloned();
                if let Some(func_info) = fi_lookup {
                    if func_info.is_variadic {
                        // Fixed params (excluding variadic)
                        let fixed_count = func_info.params.len() - 1;
                        for arg in call.args.iter().take(fixed_count) {
                            self.compile_expression(arg, out, locals)?;
                        }
                        // Variadic args: pack into a slice
                        let variadic_args = &call.args[fixed_count..];
                        let elem_vt = func_info.variadic_elem_vt.unwrap_or(ValType::I64);
                        let (elem_size, elem_align) = Self::elem_size_and_align(elem_vt);
                        let n_variadic = variadic_args.len() as i32;

                        if call.dots.is_some() && n_variadic == 1 {
                            // s... spreading: arg is already a slice, pass directly
                            self.compile_expression(&variadic_args[0], out, locals)?;
                        } else {
                            // Allocate slice header (12 bytes: ptr, len, cap)
                            let hdr = locals.add_local(&format!("__va_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(hdr));

                            // Allocate data buffer
                            let data_ptr = locals.add_local(&format!("__va_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(n_variadic * elem_size));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            out.push(Instruction::LocalSet(data_ptr));

                            // Store each variadic arg
                            for (j, arg) in variadic_args.iter().enumerate() {
                                out.push(Instruction::LocalGet(data_ptr));
                                self.compile_expression(arg, out, locals)?;
                                let arg_vt = self.infer_val_type(arg, locals);
                                Self::emit_typed_coerce(arg_vt, elem_vt, out)?;
                                let offset = j as u64 * elem_size as u64;
                                Self::emit_typed_store(elem_vt, offset, elem_align, out);
                            }

                            // Write header: ptr, len, cap
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data_ptr));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::I32Const(n_variadic));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::I32Const(n_variadic));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                        }
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    } else {
                        // Multi-value call as argument: f(g()) where g() returns
                        // multiple values matching f's parameter count
                        if call.args.len() == 1 {
                            if let ast::Expression::Call(_) = &call.args[0] {
                                let inner_count = self.expression_result_count(&call.args[0], Some(locals));
                                if inner_count > 1 && inner_count == func_info.params.len() {
                                    self.compile_expression(&call.args[0], out, locals)?;
                                    out.push(Instruction::Call(func_info.wasm_func_idx));
                                    return Ok(GoType::Int32);
                                }
                            }
                        }
                        let mut wasm_param_idx_local = 0usize;
                        for (arg_i, arg) in call.args.iter().enumerate() {
                            self.compile_expression(arg, out, locals)?;
                            let is_str_arg = self.is_string_expr(arg, locals);
                            if !is_str_arg {
                                if let ast::Expression::Ident(arg_ident) = arg {
                                    if let Some((gc_idx, gc_sd)) = self.get_gc_copy_info(&arg_ident.name, locals) {
                                        Self::emit_gc_value_deep_copy(gc_idx, &gc_sd, out, locals);
                                    } else if let Some(copy_size) = self.get_value_copy_size(&arg_ident.name, locals) {
                                        self.emit_value_deep_copy(copy_size, out, locals)?;
                                    }
                                }
                                if let Some(expected_wt) = func_info.params.get(wasm_param_idx_local) {
                                    let expected_vt = expected_wt.1.to_val_type();
                                    let actual_vt = self.infer_val_type(arg, locals);
                                    if actual_vt != expected_vt && !matches!(expected_vt, ValType::Ref(_)) && !matches!(actual_vt, ValType::Ref(_)) {
                                        Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                                    }
                                }
                            }
                            wasm_param_idx_local += if is_str_arg && self.gc_builtin_types.go_string.is_none() { 2 } else { 1 };
                            if func_info.iface_param_indices.contains(&arg_i) {
                                if let ast::Expression::Ident(arg_ident) = arg {
                                    if arg_ident.name == "nil" {
                                        out.push(Instruction::I32Const(0));
                                    } else if let Some(tid) = self.get_iface_type_id_local(&arg_ident.name, locals) {
                                        out.push(Instruction::LocalGet(tid));
                                    } else {
                                        let concrete_type = locals.get_var_struct_type(&arg_ident.name)
                                            .map(|s| s.to_string())
                                            .unwrap_or_else(|| arg_ident.name.clone());
                                        let type_id = self.get_or_create_type_id(&concrete_type);
                                        let rhs_vt = self.infer_val_type(arg, locals);
                                        let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                        let box_tmp = locals.add_local(
                                            &format!("__ibox_tmp_{}", locals.locals.len()),
                                            rhs_vt,
                                        );
                                        out.push(Instruction::LocalSet(box_tmp));
                                        let alloc_size = (elem_size as i32).max(8);
                                        out.push(Instruction::I32Const(alloc_size));
                                        out.push(Instruction::Call(self.alloc_func_idx()?));
                                        let box_ptr = locals.add_local(
                                            &format!("__ibox_ptr_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalTee(box_ptr));
                                        out.push(Instruction::LocalGet(box_tmp));
                                        let (_, align) = Self::elem_size_and_align(rhs_vt);
                                        Self::emit_typed_store(rhs_vt, 0, align, out);
                                        out.push(Instruction::LocalGet(box_ptr));
                                        out.push(Instruction::I32Const(type_id as i32));
                                    }
                                } else if let ast::Expression::CompositeLit(comp) = arg {
                                    let concrete_type = if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                                        ti.name.clone()
                                    } else {
                                        "unknown".to_string()
                                    };
                                    let type_id = self.get_or_create_type_id(&concrete_type);
                                    let rhs_vt = ValType::I32;
                                    let box_tmp = locals.add_local(
                                        &format!("__ibox_tmp_{}", locals.locals.len()),
                                        rhs_vt,
                                    );
                                    out.push(Instruction::LocalSet(box_tmp));
                                    out.push(Instruction::I32Const(8));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    let box_ptr = locals.add_local(
                                        &format!("__ibox_ptr_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalTee(box_ptr));
                                    out.push(Instruction::LocalGet(box_tmp));
                                    out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(box_ptr));
                                    out.push(Instruction::I32Const(type_id as i32));
                                } else if let ast::Expression::Operation(addr_op) = arg {
                                    if addr_op.op == Operator::And {
                                        let concrete_type = if let ast::Expression::Ident(inner_id) = &*addr_op.x {
                                            locals.get_var_struct_type(&inner_id.name)
                                                .map(|s| s.to_string())
                                                .unwrap_or_else(|| inner_id.name.clone())
                                        } else if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                            if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                                                ti.name.clone()
                                            } else {
                                                "unknown".to_string()
                                            }
                                        } else {
                                            "unknown".to_string()
                                        };
                                        let type_id = self.get_or_create_type_id(&concrete_type);
                                        let rhs_vt = ValType::I32;
                                        let box_tmp = locals.add_local(
                                            &format!("__ibox_tmp_{}", locals.locals.len()),
                                            rhs_vt,
                                        );
                                        out.push(Instruction::LocalSet(box_tmp));
                                        out.push(Instruction::I32Const(8));
                                        out.push(Instruction::Call(self.alloc_func_idx()?));
                                        let box_ptr = locals.add_local(
                                            &format!("__ibox_ptr_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalTee(box_ptr));
                                        out.push(Instruction::LocalGet(box_tmp));
                                        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                        out.push(Instruction::LocalGet(box_ptr));
                                        out.push(Instruction::I32Const(type_id as i32));
                                    } else {
                                        out.push(Instruction::I32Const(0));
                                    }
                                } else {
                                    out.push(Instruction::I32Const(0));
                                }
                            }
                        }
                        out.push(Instruction::Call(func_info.wasm_func_idx));
                    }
                } else if self.type_aliases.contains_key(&ident.name)
                    || self.current_package.as_ref().map_or(false, |pkg| {
                        self.type_aliases.contains_key(&format!("{}.{}", pkg, ident.name))
                    })
                {
                    if let Some(arg) = call.args.first() {
                        self.compile_expression(arg, out, locals)?;
                        let src_vt = self.infer_val_type(arg, locals);
                        let alias_key = if self.type_aliases.contains_key(&ident.name) {
                            ident.name.clone()
                        } else {
                            format!("{}.{}", self.current_package.as_ref().unwrap(), ident.name)
                        };
                        let resolved = self.resolve_type_name(&alias_key);
                        let target_vt = Self::val_type_for_type_name(resolved);
                        if src_vt != target_vt && !(matches!(src_vt, ValType::Ref(_)) && resolved == "string" && self.gc_builtin_types.go_string.is_some()) {
                            Self::emit_typed_coerce(src_vt, target_vt, out)?;
                        }
                        let ret_vt = if resolved == "string" { if let Some(idx) = self.gc_builtin_types.go_string { Self::gc_ref_val_type(idx) } else { target_vt } } else { target_vt };
                        return Ok(GoType::from_val_type(ret_vt));
                    }
                } else if self.generic_funcs.contains_key(&ident.name) {
                    // Type inference for generic function calls: F(args) instead of F[T](args)
                    let template = self.generic_funcs.get(&ident.name).cloned().ok_or_else(|| {
                        Error::InternalError(format!(
                            "generic function template not found for '{}'", ident.name
                        ))
                    })?;
                    let type_param_names: std::collections::HashSet<String> = template.typ.typ_params.list.iter()
                        .flat_map(|f| f.name.iter().map(|n| n.name.clone()))
                        .collect();

                    let mut subst: HashMap<String, String> = HashMap::new();
                    let mut arg_idx = 0usize;
                    for field in &template.typ.params.list {
                        for _name in &field.name {
                            if arg_idx < call.args.len() {
                                if let Some(arg_type) = self.infer_go_type_from_expr(&call.args[arg_idx], locals) {
                                    Self::try_unify_type_param(&field.typ, &arg_type, &type_param_names, &mut subst);
                                }
                            }
                            arg_idx += 1;
                        }
                    }

                    if subst.len() == type_param_names.len() {
                        let type_args: Vec<String> = template.typ.typ_params.list.iter()
                            .flat_map(|f| f.name.iter())
                            .filter_map(|n| subst.get(&n.name).cloned())
                            .collect();
                        let mono_name = format!("{}__mono_{}", ident.name, type_args.join("_"));

                        if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(func_idx));
                        } else {
                            Self::validate_type_constraints(&template, &subst)?;
                            let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                            let saved = self.generic_funcs.clone();
                            self.compile_func_decl(&specialized, true)?;
                            self.generic_funcs = saved;

                            let func_idx = self.functions.iter()
                                .find(|f| f.name == mono_name)
                                .map(|f| f.wasm_func_idx);
                            if let Some(idx) = func_idx {
                                self.monomorphized.insert(mono_name, idx);
                                for arg in &call.args {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                out.push(Instruction::Call(idx));
                            }
                        }
                    } else {
                        return Err(Error::InternalError(format!(
                            "cannot infer type parameters for generic function: {}",
                            ident.name
                        )));
                    }
                } else if let Some(&func_idx) = self.global_func_vars.get(&ident.name) {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));
                } else if let Some((cap_func_idx, cap_env_local, cap_env_captures)) = self.find_captured_closure(&ident.name) {
                    if !cap_env_captures.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_env_captures {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(cap_env_local));
                                out.push(Instruction::LocalGet(outer_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_store(*vt, *env_offset as u64, align, out);
                            }
                        }
                    }
                    if cap_env_local != u32::MAX {
                        out.push(Instruction::LocalGet(cap_env_local));
                    } else {
                        out.push(Instruction::I32Const(0));
                    }
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(cap_func_idx));
                    if !cap_env_captures.is_empty() {
                        for (cap_name, env_offset, vt) in &cap_env_captures {
                            if let Some(outer_local) = locals.find(cap_name) {
                                out.push(Instruction::LocalGet(cap_env_local));
                                let (_, align) = Self::elem_size_and_align(*vt);
                                Self::emit_typed_load(*vt, *env_offset as u64, align, out);
                                out.push(Instruction::LocalSet(outer_local));
                            }
                        }
                    }
                } else if self.iface_defs.contains_key(&ident.name)
                    || ident.name == "error"
                    || ident.name == "any"
                {
                    if let Some(arg) = call.args.first() {
                        let is_str = self.is_string_expr(arg, locals);
                        self.compile_expression(arg, out, locals)?;
                        let concrete_type = if let ast::Expression::Ident(arg_id) = arg {
                            locals.get_var_struct_type(&arg_id.name)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| arg_id.name.clone())
                        } else if let ast::Expression::Call(inner_call) = arg {
                            if let ast::Expression::Ident(fn_id) = inner_call.func.as_ref() {
                                fn_id.name.clone()
                            } else {
                                "unknown".to_string()
                            }
                        } else {
                            "unknown".to_string()
                        };
                        let type_id = self.get_or_create_type_id(&concrete_type);
                        if is_str {
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                            }
                            let str_len = locals.add_local(
                                &format!("__ibox_slen_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let str_ptr = locals.add_local(
                                &format!("__ibox_sptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalSet(str_len));
                            out.push(Instruction::LocalSet(str_ptr));
                            out.push(Instruction::I32Const(8));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let box_ptr = locals.add_local(
                                &format!("__ibox_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalSet(box_ptr));
                            out.push(Instruction::LocalGet(box_ptr));
                            out.push(Instruction::LocalGet(str_ptr));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(box_ptr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(box_ptr));
                            out.push(Instruction::I32Const(type_id as i32));
                        } else {
                            let arg_vt = self.infer_val_type(arg, locals);
                            let (elem_size, _) = Self::elem_size_and_align(arg_vt);
                            let box_tmp = locals.add_local(
                                &format!("__ibox_tmp_{}", locals.locals.len()),
                                arg_vt,
                            );
                            out.push(Instruction::LocalSet(box_tmp));
                            let alloc_size = (elem_size as i32).max(8);
                            out.push(Instruction::I32Const(alloc_size));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let box_ptr = locals.add_local(
                                &format!("__ibox_ptr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalTee(box_ptr));
                            out.push(Instruction::LocalGet(box_tmp));
                            let (_, align) = Self::elem_size_and_align(arg_vt);
                            Self::emit_typed_store(arg_vt, 0, align, out);
                            out.push(Instruction::LocalGet(box_ptr));
                            out.push(Instruction::I32Const(type_id as i32));
                        }
                    }
                } else if self.struct_defs.contains_key(&ident.name)
                    || self.current_package.as_ref().map_or(false, |pkg| {
                        self.struct_defs.contains_key(&format!("{}.{}", pkg, ident.name))
                    })
                {
                    // Struct type conversion: just compile the argument
                    if let Some(arg) = call.args.first() {
                        self.compile_expression(arg, out, locals)?;
                    }
                } else {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    return Err(Error::InternalError(format!(
                        "undefined function: {}",
                        ident.name
                    )));
                }
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                    // Handle context method calls (ctx.Log, ctx.QueryID, etc.)
                    if locals.get_var_struct_type(&pkg_ident.name) == Some("__context") {
                        let ctx_host_idx: Option<u32> = match sel.sel.name.as_str() {
                            "Log" => Some(0),
                            "QueryID" => Some(1),
                            "Database" => Some(2),
                            "Schema" => Some(3),
                            "User" => Some(4),
                            "Config" => Some(5),
                            _ => None,
                        };
                        if let Some(host_idx) = ctx_host_idx {
                            if sel.sel.name == "Log" {
                                if let Some(arg) = call.args.first() {
                                    self.compile_expression(arg, out, locals)?;
                                    if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                        if self.is_string_expr(arg, locals) {
                                            self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                                        }
                                    }
                                }
                                out.push(Instruction::Call(host_idx));
                                return Ok(GoType::Int32);
                            } else if sel.sel.name == "Config" {
                                if let Some(arg) = call.args.first() {
                                    self.compile_expression(arg, out, locals)?;
                                    if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                        if self.is_string_expr(arg, locals) {
                                            self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                                        }
                                    }
                                }
                                let key_len = locals.add_local(
                                    &format!("__cfg_key_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                let key_ptr = locals.add_local(
                                    &format!("__cfg_key_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(key_len));
                                out.push(Instruction::LocalSet(key_ptr));

                                let buf_size = 256i32;
                                out.push(Instruction::I32Const(buf_size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let buf_local = locals.add_local(
                                    &format!("__cfg_buf_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(buf_local));

                                out.push(Instruction::LocalGet(key_ptr));
                                out.push(Instruction::LocalGet(key_len));
                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::Call(host_idx));
                                let len_local = locals.add_local(
                                    &format!("__cfg_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(len_local));
                                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                    self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                                }
                                return Ok(GoType::String);
                            } else {
                                let buf_size = 256i32;
                                out.push(Instruction::I32Const(buf_size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let buf_local = locals.add_local(
                                    &format!("__ctx_buf_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(buf_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::Call(host_idx));
                                let len_local = locals.add_local(
                                    &format!("__ctx_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_local));

                                out.push(Instruction::LocalGet(buf_local));
                                out.push(Instruction::LocalGet(len_local));
                                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                    self.emit_linear_to_gc_string(go_string_idx, out, locals)?;
                                }
                                return Ok(GoType::String);
                            }
                        }
                    }

                    // Check compiled stdlib functions before hardcoded intrinsics
                    let is_inlined = Self::INLINED_NATIVE_FUNCTIONS.contains(&sel.sel.name.as_str());
                    // #region agent log
                    if sel.sel.name == "New" || pkg_ident.name == "errors" {
                        use std::io::Write;
                        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                            let in_compiled = self.compiled_packages.contains(pkg_ident.name.as_str());
                            let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                            let found_fn = self.functions.iter().find(|f| f.name == qualified && f.recv_type.is_none()).map(|f| f.wasm_func_idx);
                            let _ = writeln!(f, r#"{{"hypothesisId":"CALL","location":"calls.rs:selector","message":"selector call trace","data":{{"pkg":"{}","method":"{}","is_inlined":{},"in_compiled_pkgs":{},"found_fn_idx":"{:?}","current_pkg":"{}"}},"timestamp":{}}}"#,
                                pkg_ident.name, sel.sel.name, is_inlined, in_compiled, found_fn, self.current_package.as_deref().unwrap_or(""),
                                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                        }
                    }
                    // #endregion
                    if !is_inlined && self.compiled_packages.contains(pkg_ident.name.as_str()) {
                        let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                        if let Some(fi) = self.functions.iter().find(|f| f.name == qualified && f.recv_type.is_none()).cloned() {
                            if fi.is_variadic {
                                let fixed_count = fi.params.len() - 1;
                                for arg in call.args.iter().take(fixed_count) {
                                    self.compile_expression(arg, out, locals)?;
                                }
                                let variadic_args = &call.args[fixed_count..];
                                let elem_vt = fi.variadic_elem_vt.unwrap_or(ValType::I64);
                                let (elem_size, elem_align) = Self::elem_size_and_align(elem_vt);
                                let n_variadic = variadic_args.len() as i32;

                                if call.dots.is_some() && n_variadic == 1 {
                                    self.compile_expression(&variadic_args[0], out, locals)?;
                                } else {
                                    let hdr = locals.add_local(&format!("__va_hdr_{}", locals.locals.len()), ValType::I32);
                                    out.push(Instruction::I32Const(12));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(hdr));

                                    let data_ptr = locals.add_local(&format!("__va_data_{}", locals.locals.len()), ValType::I32);
                                    out.push(Instruction::I32Const(n_variadic * elem_size));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalSet(data_ptr));

                                    for (j, arg) in variadic_args.iter().enumerate() {
                                        out.push(Instruction::LocalGet(data_ptr));
                                        self.compile_expression(arg, out, locals)?;
                                        let arg_vt = self.infer_val_type(arg, locals);
                                        Self::emit_typed_coerce(arg_vt, elem_vt, out)?;
                                        let offset = j as u64 * elem_size as u64;
                                        Self::emit_typed_store(elem_vt, offset, elem_align, out);
                                    }

                                    out.push(Instruction::LocalGet(hdr));
                                    out.push(Instruction::LocalGet(data_ptr));
                                    out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(hdr));
                                    out.push(Instruction::I32Const(n_variadic));
                                    out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalGet(hdr));
                                    out.push(Instruction::I32Const(n_variadic));
                                    out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                                    out.push(Instruction::LocalGet(hdr));
                                }
                            } else {
                                let mut wasm_param_idx = 0usize;
                                for (i, arg) in call.args.iter().enumerate() {
                                    self.compile_expression(arg, out, locals)?;
                                    let is_str_arg = self.is_string_expr(arg, locals);
                                    if !is_str_arg {
                                        if let Some(expected_wt) = fi.params.get(wasm_param_idx) {
                                            let expected_vt = expected_wt.1.to_val_type();
                                            let actual_vt = self.infer_val_type(arg, locals);
                                            if actual_vt != expected_vt {
                                                Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                                            }
                                        }
                                    }
                                    wasm_param_idx += if is_str_arg && self.gc_builtin_types.go_string.is_none() { 2 } else { 1 };
                                    if fi.iface_param_indices.contains(&i) {
                                        if let ast::Expression::Ident(arg_ident) = arg {
                                            if arg_ident.name == "nil" {
                                                out.push(Instruction::I32Const(0));
                                            } else if let Some(tid) = self.get_iface_type_id_local(&arg_ident.name, locals) {
                                                out.push(Instruction::LocalGet(tid));
                                            } else {
                                                let concrete_type = locals.get_var_struct_type(&arg_ident.name)
                                                    .map(|s| s.to_string())
                                                    .unwrap_or_else(|| arg_ident.name.clone());
                                                let type_id = self.get_or_create_type_id(&concrete_type);
                                                let rhs_vt = self.infer_val_type(arg, locals);
                                                let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                                let box_tmp = locals.add_local(
                                                    &format!("__ibox_tmp_{}", locals.locals.len()),
                                                    rhs_vt,
                                                );
                                                out.push(Instruction::LocalSet(box_tmp));
                                                let alloc_size = (elem_size as i32).max(8);
                                                out.push(Instruction::I32Const(alloc_size));
                                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                                let box_ptr = locals.add_local(
                                                    &format!("__ibox_ptr_{}", locals.locals.len()),
                                                    ValType::I32,
                                                );
                                                out.push(Instruction::LocalTee(box_ptr));
                                                out.push(Instruction::LocalGet(box_tmp));
                                                let (_, align) = Self::elem_size_and_align(rhs_vt);
                                                Self::emit_typed_store(rhs_vt, 0, align, out);
                                                out.push(Instruction::LocalGet(box_ptr));
                                                out.push(Instruction::I32Const(type_id as i32));
                                            }
                                        } else if let ast::Expression::Operation(addr_op) = arg {
                                            if addr_op.op == Operator::And {
                                                let concrete_type = if let ast::Expression::Ident(inner_id) = &*addr_op.x {
                                                    locals.get_var_struct_type(&inner_id.name)
                                                        .map(|s| s.to_string())
                                                        .unwrap_or_else(|| inner_id.name.clone())
                                                } else if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                                    if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                                                        ti.name.clone()
                                                    } else {
                                                        "unknown".to_string()
                                                    }
                                                } else {
                                                    "unknown".to_string()
                                                };
                                                let type_id = self.get_or_create_type_id(&concrete_type);
                                                let rhs_vt = ValType::I32;
                                                let box_tmp = locals.add_local(
                                                    &format!("__ibox_tmp_{}", locals.locals.len()),
                                                    rhs_vt,
                                                );
                                                out.push(Instruction::LocalSet(box_tmp));
                                                out.push(Instruction::I32Const(8));
                                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                                let box_ptr = locals.add_local(
                                                    &format!("__ibox_ptr_{}", locals.locals.len()),
                                                    ValType::I32,
                                                );
                                                out.push(Instruction::LocalTee(box_ptr));
                                                out.push(Instruction::LocalGet(box_tmp));
                                                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                                out.push(Instruction::LocalGet(box_ptr));
                                                out.push(Instruction::I32Const(type_id as i32));
                                            } else {
                                                out.push(Instruction::I32Const(0));
                                            }
                                        } else {
                                            out.push(Instruction::I32Const(0));
                                        }
                                    }
                                }
                            }
                            out.push(Instruction::Call(fi.wasm_func_idx));
                            return Ok(GoType::Int32);
                        }

                        let qualified_alias = format!("{}.{}", pkg_ident.name, sel.sel.name);
                        if self.type_aliases.contains_key(&qualified_alias) {
                            if let Some(arg) = call.args.first() {
                                self.compile_expression(arg, out, locals)?;
                                let src_vt = self.infer_val_type(arg, locals);
                                let resolved = self.resolve_type_name(&qualified_alias);
                                let target_vt = Self::val_type_for_type_name(resolved);
                                if src_vt != target_vt && !(matches!(src_vt, ValType::Ref(_)) && resolved == "string" && self.gc_builtin_types.go_string.is_some()) {
                                    Self::emit_typed_coerce(src_vt, target_vt, out)?;
                                }
                                let ret_vt = if resolved == "string" { if let Some(idx) = self.gc_builtin_types.go_string { Self::gc_ref_val_type(idx) } else { target_vt } } else { target_vt };
                                return Ok(GoType::from_val_type(ret_vt));
                            }
                        }

                    }

                    match (pkg_ident.name.as_str(), sel.sel.name.as_str()) {
                        ("fmt", "Errorf" | "Sprintf") => {
                            return Err(Error::InternalError(format!(
                                "fmt.{} is not yet available; stdlib will be provided as host functions",
                                sel.sel.name
                            )));
                        }
                        ("math", "Sqrt") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Sqrt requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Sqrt);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Abs") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Abs requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Abs);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Floor") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Floor requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Floor);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Ceil") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Ceil requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Ceil);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Min") => {
                            if call.args.len() < 2 {
                                return Err(Error::InternalError(
                                    "math.Min requires 2 arguments".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            self.compile_expression(&call.args[1], out, locals)?;
                            out.push(Instruction::F64Min);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Max") => {
                            if call.args.len() < 2 {
                                return Err(Error::InternalError(
                                    "math.Max requires 2 arguments".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            self.compile_expression(&call.args[1], out, locals)?;
                            out.push(Instruction::F64Max);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Trunc") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Trunc requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Trunc);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Round") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Round requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64Nearest);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Float64frombits") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Float64frombits requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::F64ReinterpretI64);
                            return Ok(GoType::Float64);
                        }
                        ("math", "Float64bits") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Float64bits requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            out.push(Instruction::I64ReinterpretF64);
                            return Ok(GoType::Uint64);
                        }
                        ("math", "Float32frombits") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Float32frombits requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            let arg_vt = self.infer_val_type(&call.args[0], locals);
                            if arg_vt == ValType::I64 {
                                out.push(Instruction::I32WrapI64);
                            }
                            out.push(Instruction::F32ReinterpretI32);
                            return Ok(GoType::Float32);
                        }
                        ("math", "Float32bits") => {
                            if call.args.is_empty() {
                                return Err(Error::InternalError(
                                    "math.Float32bits requires 1 argument".to_string(),
                                ));
                            }
                            self.compile_expression(&call.args[0], out, locals)?;
                            let arg_vt = self.infer_val_type(&call.args[0], locals);
                            if arg_vt == ValType::F64 {
                                out.push(Instruction::F32DemoteF64);
                            }
                            out.push(Instruction::I32ReinterpretF32);
                            return Ok(GoType::Int32);
                        }
                        
                        (pkg, func_name)
                            if matches!(
                                pkg,
                                "strings" | "sort"
                                    | "bytes" | "encoding" | "fmt"
                            ) =>
                        {
                            return Err(Error::InternalError(format!(
                                "{}.{} is not yet implemented; stdlib functions will be available in a future release",
                                pkg, func_name
                            )));
                        }
                        _ => {
                            if locals.find(&pkg_ident.name).is_none()
                                && !self.struct_defs.contains_key(&pkg_ident.name)
                                && !self.type_registry.contains_key(&pkg_ident.name)
                                && !self.is_interface_var(&pkg_ident.name, locals)
                            {
                                return Err(Error::InternalError(format!(
                                    "unsupported package function: {}.{}",
                                    pkg_ident.name, sel.sel.name
                                )));
                            }
                        }
                    }

                    // Check if the receiver is an interface variable
                    if self.is_interface_var(&pkg_ident.name, locals) {
                        self.compile_interface_method_call(
                            &pkg_ident.name,
                            &sel.sel.name,
                            &call.args,
                            out,
                            locals,
                        )?;
                        return Ok(GoType::Int32);
                    }

                    // Method expression: TypeName.Method(receiver, args...)
                    if (self.struct_defs.contains_key(&pkg_ident.name) || self.type_registry.contains_key(&pkg_ident.name))
                        && locals.find(&pkg_ident.name).is_none()
                    {
                        let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                        if let Some(fi_idx) = self.functions.iter().position(|f| f.name == qualified && f.recv_type.is_some()) {
                            let func_idx = self.functions[fi_idx].wasm_func_idx;
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(func_idx));
                            return Ok(GoType::Int32);
                        }
                    }

                    // Method call on a receiver
                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }

                    // Resolve the receiver's struct type name for qualified lookup
                    let recv_type_name = locals
                        .get_var_struct_type(&pkg_ident.name)
                        .map(|s| s.to_string());

                    let found = if let Some(ref type_name) = recv_type_name {
                        let qualified = format!("{}.{}", type_name, sel.sel.name);
                        self.functions.iter().find(|f| f.name == qualified).map(|f| f.wasm_func_idx)
                    } else {
                        None
                    };

                    let method_suffix = format!(".{}", sel.sel.name);
                    let func_idx = found
                        .or_else(|| {
                            if let Some(ref type_name) = recv_type_name {
                                if let Some(ref pkg) = self.current_package {
                                    let pkg_qual = format!("{}.{}.{}", pkg, type_name, sel.sel.name);
                                    if let Some(f) = self.functions.iter().find(|f| f.name == pkg_qual) {
                                        return Some(f.wasm_func_idx);
                                    }
                                }
                                for cpkg in &self.compiled_packages {
                                    let pkg_qual = format!("{}.{}.{}", cpkg, type_name, sel.sel.name);
                                    if let Some(f) = self.functions.iter().find(|f| f.name == pkg_qual) {
                                        return Some(f.wasm_func_idx);
                                    }
                                }
                            }
                            None
                        })
                        .or_else(|| {
                            let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                            self.functions.iter().find(|f| f.name == qualified).map(|f| f.wasm_func_idx)
                        })
                        .or_else(|| {
                            if let Some(ref type_name) = recv_type_name {
                                let qualified = format!("{}{}", type_name, method_suffix);
                                self.functions.iter().find(|f| f.recv_type.is_some() && f.name == qualified).map(|f| f.wasm_func_idx)
                            } else {
                                let matches: Vec<_> = self.functions.iter()
                                    .filter(|f| f.recv_type.is_some() && f.name.ends_with(&method_suffix))
                                    .collect();
                                if matches.len() == 1 {
                                    Some(matches[0].wasm_func_idx)
                                } else {
                                    None
                                }
                            }
                        });

                    if let Some(idx) = func_idx {
                        out.push(Instruction::Call(idx));
                    } else {
                        // Check embedded types for promoted methods
                        let mut embed_found = false;
                        if let Some(ref type_name) = recv_type_name {
                            if let Some(sdef) = self.struct_defs.get(type_name).cloned() {
                                for (embed_type, embed_offset) in &sdef.embedded_types {
                                    let qualified = format!("{}.{}", embed_type, sel.sel.name);
                                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                                        // Adjust receiver: add embedding offset
                                        // The receiver is already on the stack (pushed for args earlier)
                                        // We need to insert the offset adjustment before the call
                                        // The receiver was the first arg pushed. Rewrite: pop all args,
                                        // adjust receiver, push args back, call.
                                        // Simplification: insert I32Const(offset) + I32Add before the args
                                        // were pushed. Since args are already on stack, we can't easily do this.
                                        // Instead, let's re-emit: receiver was pushed as first thing.
                                        // We'll adjust by emitting before args were compiled above.
                                        // Actually, since we already compiled args, we need to work with what's on the stack.
                                        // For now, handle the common case: no extra args besides receiver.
                                        let embed_func_idx = fi.wasm_func_idx;
                                        let wasm_params: Vec<_> = fi.params.iter()
                                            .skip(1) // skip receiver
                                            .map(|(_, wt)| wt.to_val_type())
                                            .collect();
                                        if wasm_params.is_empty() && *embed_offset == 0 {
                                            out.push(Instruction::Call(embed_func_idx));
                                        } else {
                                            let mut saved = Vec::new();
                                            for (j, vt) in wasm_params.iter().enumerate().rev() {
                                                let tmp = locals.add_local(
                                                    &format!("__embed_arg_{}_{}", j, locals.locals.len()),
                                                    *vt
                                                );
                                                out.push(Instruction::LocalSet(tmp));
                                                saved.push(tmp);
                                            }
                                            if *embed_offset > 0 {
                                                out.push(Instruction::I32Const(*embed_offset as i32));
                                                out.push(Instruction::I32Add);
                                            }
                                            for tmp in saved.iter().rev() {
                                                out.push(Instruction::LocalGet(*tmp));
                                            }
                                            out.push(Instruction::Call(embed_func_idx));
                                        }
                                        embed_found = true;
                                        break;
                                    }
                                }
                            }
                        }
                        if !embed_found {
                            return Err(Error::InternalError(format!(
                                "undefined method: {}.{}",
                                pkg_ident.name, sel.sel.name
                            )));
                        }
                    }
                } else {
                    // Non-ident receiver: method chaining (e.g., b.Add(10).Add(20))
                    let recv_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);
                    if let Some(type_name) = recv_type {
                        self.compile_expression(sel.x.as_ref(), out, locals)?;
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        let func_idx = self.find_method_func(&type_name, &sel.sel.name)
                            .map(|f| f.wasm_func_idx);
                        if let Some(idx) = func_idx {
                            out.push(Instruction::Call(idx));
                        } else {
                            return Err(Error::InternalError(format!(
                                "undefined method: {}.{}",
                                type_name, sel.sel.name
                            )));
                        }
                    } else if self.is_interface_field_selector(sel.x.as_ref(), locals) {
                        self.compile_expression(sel.x.as_ref(), out, locals)?;
                        let tid_tmp = locals.add_local(
                            &format!("__imc_fsel_tid_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let data_tmp = locals.add_local(
                            &format!("__imc_fsel_data_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(tid_tmp));
                        out.push(Instruction::LocalSet(data_tmp));
                        self.compile_interface_method_call_with_locals(
                            tid_tmp, data_tmp, &sel.sel.name, &call.args, out, locals,
                        )?;
                        return Ok(GoType::Int32);
                    } else {
                        self.compile_expression(sel.x.as_ref(), out, locals)?;
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        let method_suffix = format!(".{}", sel.sel.name);
                        let matches: Vec<_> = self.functions.iter()
                            .filter(|f| f.recv_type.is_some() && f.name.ends_with(&method_suffix))
                            .collect();
                        if matches.len() == 1 {
                            out.push(Instruction::Call(matches[0].wasm_func_idx));
                        } else {
                            return Err(Error::InternalError(format!(
                                "cannot infer receiver type for method call .{}",
                                sel.sel.name
                            )));
                        }
                    }
                }
            }
            ast::Expression::TypeSlice(slice_type) => {
                // Type conversion: []byte(str) or []rune(str)
                if let ast::Expression::Ident(elem_ident) = slice_type.typ.as_ref() {
                    if let Some(arg) = call.args.first() {
                        if self.is_string_expr(arg, locals) && (elem_ident.name == "byte" || elem_ident.name == "uint8") {
                            // []byte(str): unpack string bytes into 4-byte I32 slots
                            self.compile_expression(arg, out, locals)?;
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                            }
                            let str_len = locals.add_local(&format!("__sb_len_{}", locals.locals.len()), ValType::I32);
                            let str_ptr = locals.add_local(&format!("__sb_ptr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(str_len));
                            out.push(Instruction::LocalSet(str_ptr));

                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let hdr = locals.add_local(&format!("__sb_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(hdr));

                            // Allocate str_len * 4 bytes (each byte stored as I32)
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let data = locals.add_local(&format!("__sb_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(data));

                            // Loop: unpack each byte into a 4-byte slot
                            let idx_l = locals.add_local(&format!("__sb_idx_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(idx_l));

                            out.push(Instruction::Block(BlockType::Empty));
                            out.push(Instruction::Loop(BlockType::Empty));

                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::BrIf(1));

                            // data[idx*4] = str_ptr[idx] (load byte, store as I32)
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(str_ptr));
                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(idx_l));
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(idx_l));
                            out.push(Instruction::Br(0));

                            out.push(Instruction::End); // loop
                            out.push(Instruction::End); // block

                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                            return Ok(GoType::Slice(Box::new(GoType::Int32)));
                        }
                        if self.is_string_expr(arg, locals) && (elem_ident.name == "rune" || elem_ident.name == "int32") {
                            // []rune(str): decode UTF-8 into a []int32 slice
                            self.compile_expression(arg, out, locals)?;
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                            }
                            let str_len = locals.add_local(&format!("__sr_len_{}", locals.locals.len()), ValType::I32);
                            let str_ptr = locals.add_local(&format!("__sr_ptr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(str_len));
                            out.push(Instruction::LocalSet(str_ptr));

                            // Worst case: each byte is a rune, allocate str_len * 4 bytes
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let hdr = locals.add_local(&format!("__sr_hdr_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(hdr));

                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::Call(self.alloc_func_idx()?));
                            let data = locals.add_local(&format!("__sr_data_{}", locals.locals.len()), ValType::I32);
                            out.push(Instruction::LocalSet(data));

                            // Decode loop: src_idx iterates over bytes, dst_idx over runes
                            let src_idx = locals.add_local(&format!("__sr_si_{}", locals.locals.len()), ValType::I32);
                            let dst_idx = locals.add_local(&format!("__sr_di_{}", locals.locals.len()), ValType::I32);
                            let byte0 = locals.add_local(&format!("__sr_b0_{}", locals.locals.len()), ValType::I32);
                            let addr = locals.add_local(&format!("__sr_addr_{}", locals.locals.len()), ValType::I32);
                            let rune_v = locals.add_local(&format!("__sr_rv_{}", locals.locals.len()), ValType::I32);
                            let rune_w = locals.add_local(&format!("__sr_rw_{}", locals.locals.len()), ValType::I32);

                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(src_idx));
                            out.push(Instruction::I32Const(0));
                            out.push(Instruction::LocalSet(dst_idx));

                            out.push(Instruction::Block(BlockType::Empty));
                            out.push(Instruction::Loop(BlockType::Empty));

                            // Break if src_idx >= str_len
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::BrIf(1));

                            // addr = str_ptr + src_idx
                            out.push(Instruction::LocalGet(str_ptr));
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(addr));

                            // byte0 = mem[addr]
                            out.push(Instruction::LocalGet(addr));
                            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                            out.push(Instruction::LocalSet(byte0));

                            // Default: width=1, rune=byte0
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::LocalSet(rune_w));
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::LocalSet(rune_v));

                            // Multi-byte UTF-8 decode with boundary checks
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::I32Const(0x80));
                            out.push(Instruction::I32GeU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                // 2-byte: (byte0 & 0xE0) == 0xC0
                                out.push(Instruction::LocalGet(byte0));
                                out.push(Instruction::I32Const(0xE0));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(0xC0));
                                out.push(Instruction::I32Eq);
                                out.push(Instruction::If(BlockType::Empty));
                                {
                                    // Boundary check: src_idx + 2 <= str_len
                                    out.push(Instruction::LocalGet(src_idx));
                                    out.push(Instruction::I32Const(2));
                                    out.push(Instruction::I32Add);
                                    out.push(Instruction::LocalGet(str_len));
                                    out.push(Instruction::I32LeU);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        out.push(Instruction::I32Const(2));
                                        out.push(Instruction::LocalSet(rune_w));
                                        out.push(Instruction::LocalGet(byte0));
                                        out.push(Instruction::I32Const(0x1F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Const(6));
                                        out.push(Instruction::I32Shl);
                                        out.push(Instruction::LocalGet(addr));
                                        out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                        out.push(Instruction::I32Const(0x3F));
                                        out.push(Instruction::I32And);
                                        out.push(Instruction::I32Or);
                                        out.push(Instruction::LocalSet(rune_v));
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        out.push(Instruction::I32Const(0xFFFD));
                                        out.push(Instruction::LocalSet(rune_v));
                                    }
                                    out.push(Instruction::End);
                                }
                                out.push(Instruction::Else);
                                {
                                    // 3-byte: (byte0 & 0xF0) == 0xE0
                                    out.push(Instruction::LocalGet(byte0));
                                    out.push(Instruction::I32Const(0xF0));
                                    out.push(Instruction::I32And);
                                    out.push(Instruction::I32Const(0xE0));
                                    out.push(Instruction::I32Eq);
                                    out.push(Instruction::If(BlockType::Empty));
                                    {
                                        // Boundary check: src_idx + 3 <= str_len
                                        out.push(Instruction::LocalGet(src_idx));
                                        out.push(Instruction::I32Const(3));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(str_len));
                                        out.push(Instruction::I32LeU);
                                        out.push(Instruction::If(BlockType::Empty));
                                        {
                                            out.push(Instruction::I32Const(3));
                                            out.push(Instruction::LocalSet(rune_w));
                                            out.push(Instruction::LocalGet(byte0));
                                            out.push(Instruction::I32Const(0x0F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(12));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(6));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::Else);
                                        {
                                            out.push(Instruction::I32Const(0xFFFD));
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::End);
                                    }
                                    out.push(Instruction::Else);
                                    {
                                        // 4-byte: assume (byte0 & 0xF8) == 0xF0
                                        // Boundary check: src_idx + 4 <= str_len
                                        out.push(Instruction::LocalGet(src_idx));
                                        out.push(Instruction::I32Const(4));
                                        out.push(Instruction::I32Add);
                                        out.push(Instruction::LocalGet(str_len));
                                        out.push(Instruction::I32LeU);
                                        out.push(Instruction::If(BlockType::Empty));
                                        {
                                            out.push(Instruction::I32Const(4));
                                            out.push(Instruction::LocalSet(rune_w));
                                            out.push(Instruction::LocalGet(byte0));
                                            out.push(Instruction::I32Const(0x07));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(18));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 1, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(12));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 2, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Const(6));
                                            out.push(Instruction::I32Shl);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalGet(addr));
                                            out.push(Instruction::I32Load8U(MemArg { offset: 3, align: 0, memory_index: 0 }));
                                            out.push(Instruction::I32Const(0x3F));
                                            out.push(Instruction::I32And);
                                            out.push(Instruction::I32Or);
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::Else);
                                        {
                                            out.push(Instruction::I32Const(0xFFFD));
                                            out.push(Instruction::LocalSet(rune_v));
                                        }
                                        out.push(Instruction::End);
                                    }
                                    out.push(Instruction::End);
                                }
                                out.push(Instruction::End);
                            }
                            out.push(Instruction::End);

                            // Store rune at data[dst_idx * 4]
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Mul);
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(rune_v));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));

                            // src_idx += rune_w, dst_idx += 1
                            out.push(Instruction::LocalGet(src_idx));
                            out.push(Instruction::LocalGet(rune_w));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(src_idx));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalSet(dst_idx));

                            out.push(Instruction::Br(0));
                            out.push(Instruction::End); // end loop
                            out.push(Instruction::End); // end block

                            // Fill header: data_ptr, len=dst_idx, cap=str_len
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(data));
                            out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(dst_idx));
                            out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalGet(hdr));
                            out.push(Instruction::LocalGet(str_len));
                            out.push(Instruction::I32Store(MemArg { offset: 8, align: 2, memory_index: 0 }));

                            out.push(Instruction::LocalGet(hdr));
                            return Ok(GoType::Slice(Box::new(GoType::Int32)));
                        }
                    }
                }
                if let Some(arg) = call.args.first() {
                    self.compile_expression(arg, out, locals)?;
                    return Ok(GoType::Void);
                }
                return Err(Error::InternalError(format!(
                    "unsupported slice type conversion: {:?}",
                    call.func
                )));
            }
            ast::Expression::TypeArray(arr_type) => {
                // Slice-to-array conversion: [N]T(slice)
                if let Some(arg) = call.args.first() {
                    let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                        Self::parse_go_int(&lit.value)
                            .map_err(|e| Error::SyntaxError(e))? as u32
                    } else {
                        return Err(Error::InternalError(
                            "slice-to-array conversion requires a constant array length".to_string(),
                        ));
                    };
                    let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                    let (elem_size, _) = Self::elem_size_and_align(elem_vt);

                    self.compile_expression(arg, out, locals)?;
                    let slice_hdr = locals.add_local(&format!("__s2a_hdr_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalSet(slice_hdr));

                    // Load slice length and check >= arr_len
                    out.push(Instruction::LocalGet(slice_hdr));
                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::I32Const(arr_len as i32));
                    out.push(Instruction::I32LtU);
                    out.push(Instruction::If(BlockType::Empty));
                    out.push(Instruction::Unreachable);
                    out.push(Instruction::End);

                    // Allocate array memory
                    let total_bytes = elem_size * arr_len as i32;
                    out.push(Instruction::I32Const(total_bytes));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    let arr_ptr = locals.add_local(&format!("__s2a_ptr_{}", locals.locals.len()), ValType::I32);
                    out.push(Instruction::LocalSet(arr_ptr));

                    // Copy elements: memory.copy(arr_ptr, slice_data_ptr, total_bytes)
                    out.push(Instruction::LocalGet(arr_ptr));
                    out.push(Instruction::LocalGet(slice_hdr));
                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    out.push(Instruction::I32Const(total_bytes));
                    out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                    out.push(Instruction::LocalGet(arr_ptr));
                    return Ok(GoType::Int32);
                }
                return Err(Error::InternalError(
                    "slice-to-array conversion requires exactly one argument".to_string(),
                ));
            }
            ast::Expression::Index(idx_expr)
                if idx_expr.left.as_ref().map_or(false, |l| {
                    if let ast::Expression::Ident(id) = l.as_ref() {
                        self.generic_funcs.contains_key(&id.name)
                    } else {
                        false
                    }
                }) =>
            {
                // Generic function call with single type arg: F[T](args)
                let func_ident = if let Some(ast::Expression::Ident(id)) = idx_expr.left.as_deref() {
                    id
                } else {
                    return Err(Error::InternalError(
                        "generic function call requires an identifier as the function name".to_string(),
                    ));
                };
                let type_arg = if let ast::Expression::Ident(ti) = idx_expr.index.as_ref() {
                    ti.name.clone()
                } else {
                    format!("{:?}", idx_expr.index)
                };
                let type_args = vec![type_arg];
                let mono_name = format!("{}__mono_{}", func_ident.name, type_args.join("_"));

                if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                    for arg in &call.args {
                        self.compile_expression(arg, out, locals)?;
                    }
                    out.push(Instruction::Call(func_idx));
                    return Ok(GoType::Int32);
                }

                let generic_decl = self.generic_funcs.get(&func_ident.name).cloned();
                if let Some(template) = generic_decl {
                    let mut subst: HashMap<String, String> = HashMap::new();
                    for (i, field) in template.typ.typ_params.list.iter().enumerate() {
                        for name_ident in &field.name {
                            if i < type_args.len() {
                                subst.insert(name_ident.name.clone(), type_args[i].clone());
                            }
                        }
                    }
                    Self::validate_type_constraints(&template, &subst)?;
                    let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                    let saved_generic_funcs = self.generic_funcs.clone();
                    self.compile_func_decl(&specialized, true)?;
                    self.generic_funcs = saved_generic_funcs;

                    let func_idx = self.functions.iter()
                        .find(|f| f.name == mono_name)
                        .map(|f| f.wasm_func_idx);
                    if let Some(idx) = func_idx {
                        self.monomorphized.insert(mono_name.clone(), idx);
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(idx));
                    }
                    return Ok(GoType::Int32);
                }

                return Err(Error::InternalError(format!(
                    "undefined generic function: {}",
                    func_ident.name
                )));
            }
            ast::Expression::IndexList(idx_list) => {
                // Generic function call: F[T1, T2, ...](args)
                if let ast::Expression::Ident(func_ident) = idx_list.left.as_ref() {
                    let type_args: Vec<String> = idx_list.indices.iter().map(|idx| {
                        if let ast::Expression::Ident(ti) = idx {
                            ti.name.clone()
                        } else {
                            format!("{:?}", idx)
                        }
                    }).collect();

                    let mono_name = format!("{}__mono_{}", func_ident.name, type_args.join("_"));

                    // Check if already monomorphized
                    if let Some(&func_idx) = self.monomorphized.get(&mono_name) {
                        for arg in &call.args {
                            self.compile_expression(arg, out, locals)?;
                        }
                        out.push(Instruction::Call(func_idx));
                        return Ok(GoType::Int32);
                    }

                    // Look up the generic function template
                    let generic_decl = self.generic_funcs.get(&func_ident.name).cloned();
                    if let Some(template) = generic_decl {
                        // Build type parameter substitution map
                        let mut subst: HashMap<String, String> = HashMap::new();
                        for (i, field) in template.typ.typ_params.list.iter().enumerate() {
                            for name_ident in &field.name {
                                if i < type_args.len() {
                                    subst.insert(name_ident.name.clone(), type_args[i].clone());
                                }
                            }
                        }

                        // Validate type constraints before monomorphization
                        Self::validate_type_constraints(&template, &subst)?;
                        let specialized = self.monomorphize_func_decl(&template, &mono_name, &subst);
                        let saved_generic_funcs = self.generic_funcs.clone();
                        self.compile_func_decl(&specialized, true)?;
                        self.generic_funcs = saved_generic_funcs;

                        // Record the monomorphized func_idx
                        let func_idx = self.functions.iter()
                            .find(|f| f.name == mono_name)
                            .map(|f| f.wasm_func_idx);
                        if let Some(idx) = func_idx {
                            self.monomorphized.insert(mono_name.clone(), idx);
                            for arg in &call.args {
                                self.compile_expression(arg, out, locals)?;
                            }
                            out.push(Instruction::Call(idx));
                        }
                        return Ok(GoType::Int32);
                    }

                    return Err(Error::InternalError(format!(
                        "undefined generic function: {}",
                        func_ident.name
                    )));
                }
                return Err(Error::InternalError(format!(
                    "unsupported generic call expression: {:?}",
                    call.func
                )));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported call expression: {:?}",
                    call.func
                )));
            }
        }
        Ok(GoType::Int32)
    }
}
