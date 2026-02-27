use super::*;

impl WasmCompiler {
    pub(crate) fn emit_go_typed_store(elem_size: i32, align: u32, offset: u64, vt: ValType, out: &mut Vec<Instruction<'static>>) {
        match elem_size {
            1 => out.push(Instruction::I32Store8(MemArg { offset, align, memory_index: 0 })),
            2 => out.push(Instruction::I32Store16(MemArg { offset, align, memory_index: 0 })),
            _ => Self::emit_typed_store(vt, offset, align, out),
        }
    }

    pub(crate) fn emit_go_typed_load(elem_size: i32, align: u32, offset: u64, vt: ValType, out: &mut Vec<Instruction<'static>>) {
        match elem_size {
            1 => out.push(Instruction::I32Load8U(MemArg { offset, align, memory_index: 0 })),
            2 => out.push(Instruction::I32Load16U(MemArg { offset, align, memory_index: 0 })),
            _ => Self::emit_typed_load(vt, offset, align, out),
        }
    }

    pub(crate) fn emit_typed_store(vt: ValType, offset: u64, align: u32, out: &mut Vec<Instruction<'static>>) {
        match vt {
            ValType::I32 => out.push(Instruction::I32Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F32 => out.push(Instruction::F32Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F64 => out.push(Instruction::F64Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            _ => out.push(Instruction::I64Store(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
        }
    }

    pub(crate) fn emit_typed_load(vt: ValType, offset: u64, align: u32, out: &mut Vec<Instruction<'static>>) {
        match vt {
            ValType::I32 => out.push(Instruction::I32Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F32 => out.push(Instruction::F32Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            ValType::F64 => out.push(Instruction::F64Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
            _ => out.push(Instruction::I64Load(MemArg {
                offset,
                align,
                memory_index: 0,
            })),
        }
    }

    pub(crate) fn get_value_copy_size(&self, var_name: &str, locals: &LocalAlloc) -> Option<u32> {
        if locals.pointer_to_struct_vars.contains(var_name) {
            return None;
        }
        if let Some(type_name) = locals.get_var_struct_type(var_name) {
            if let Some(sd) = self.struct_defs.get(type_name) {
                return Some(sd.total_size);
            }
        }
        if let Some(&(_elem_vt, arr_len, go_es, _)) = locals.array_info.get(var_name) {
            let elem_size = go_es;
            return Some(elem_size as u32 * arr_len);
        }
        None
    }

    pub(crate) fn get_gc_copy_info(&self, var_name: &str, locals: &LocalAlloc) -> Option<(u32, StructDef)> {
        if locals.pointer_to_struct_vars.contains(var_name) {
            return None;
        }
        if let Some(type_name) = locals.get_var_struct_type(var_name) {
            if let Some(sd) = self.struct_defs.get(type_name) {
                if let Some(gc_idx) = sd.gc_type_idx {
                    return Some((gc_idx, sd.clone()));
                }
            }
        }
        None
    }

    pub(crate) fn get_struct_copy_size_from_type(&self, type_name: &str) -> Option<u32> {
        if let Some(sd) = self.struct_defs.get(type_name) {
            Some(sd.total_size)
        } else {
            None
        }
    }

    /// Emit instructions that deep-copy a heap-allocated value (struct or array).
    /// Expects the source pointer on top of the stack.
    /// Leaves the new (destination) pointer on top of the stack.

    pub(crate) fn emit_value_deep_copy(
        &self,
        size: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let src_tmp = locals.add_local(
            &format!("__copy_src_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(src_tmp));

        out.push(Instruction::I32Const(size as i32));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let dst_tmp = locals.add_local(
            &format!("__copy_dst_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalSet(dst_tmp));

        // memory.copy(dst, src, size)
        out.push(Instruction::LocalGet(dst_tmp));
        out.push(Instruction::LocalGet(src_tmp));
        out.push(Instruction::I32Const(size as i32));
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        out.push(Instruction::LocalGet(dst_tmp));
        Ok(())
    }

    pub(crate) fn emit_gc_value_deep_copy(
        gc_type_idx: u32,
        struct_def: &StructDef,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
        let src_tmp = locals.add_local(
            &format!("__gc_copy_src_{}", locals.locals.len()),
            Self::gc_ref_val_type(gc_type_idx),
        );
        out.push(Instruction::LocalSet(src_tmp));

        let dst_tmp = locals.add_local(
            &format!("__gc_copy_dst_{}", locals.locals.len()),
            Self::gc_ref_val_type(gc_type_idx),
        );
        out.push(Instruction::StructNewDefault(gc_type_idx));
        out.push(Instruction::LocalSet(dst_tmp));

        for field in &struct_def.fields {
            out.push(Instruction::LocalGet(dst_tmp));
            out.push(Instruction::LocalGet(src_tmp));
            out.push(Instruction::StructGet {
                struct_type_index: gc_type_idx,
                field_index: field.field_index,
            });
            out.push(Instruction::StructSet {
                struct_type_index: gc_type_idx,
                field_index: field.field_index,
            });
        }

        out.push(Instruction::LocalGet(dst_tmp));
    }

    pub(crate) fn reject_unsupported_type(typ: &ast::Expression) -> Result<(), Error> {
        match typ {
            ast::Expression::TypeChannel(_) => Err(Error::InternalError(
                "channels are not supported in WASM UDFs".to_string(),
            )),
            _ => Ok(()),
        }
    }

    pub(crate) fn emit_typed_coerce(from: ValType, to: ValType, out: &mut Vec<Instruction<'static>>) -> Result<(), Error> {
        if from == to {
            return Ok(());
        }
        match (from, to) {
            (ValType::I64, ValType::I32) => out.push(Instruction::I32WrapI64),
            (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32S),
            (ValType::F64, ValType::F32) => out.push(Instruction::F32DemoteF64),
            (ValType::F32, ValType::F64) => out.push(Instruction::F64PromoteF32),
            (ValType::I64, ValType::F64) => out.push(Instruction::F64ConvertI64S),
            (ValType::I32, ValType::F64) => out.push(Instruction::F64ConvertI32S),
            (ValType::I64, ValType::F32) => out.push(Instruction::F32ConvertI64S),
            (ValType::I32, ValType::F32) => out.push(Instruction::F32ConvertI32S),
            (ValType::F64, ValType::I64) => out.push(Instruction::I64TruncF64S),
            (ValType::F64, ValType::I32) => out.push(Instruction::I32TruncF64S),
            (ValType::F32, ValType::I64) => out.push(Instruction::I64TruncF32S),
            (ValType::F32, ValType::I32) => out.push(Instruction::I32TruncF32S),
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported type coercion from {:?} to {:?}",
                    from, to
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn coerce_go_types(from: &GoType, to: &GoType, out: &mut Vec<Instruction<'static>>) -> Result<(), Error> {
        let from_wt = from.wasm_type();
        let to_wt = to.wasm_type();
        if from_wt != to_wt {
            if from.is_unsigned() {
                match (from_wt, to_wt) {
                    (ValType::I32, ValType::I64) => out.push(Instruction::I64ExtendI32U),
                    (ValType::I32, ValType::F64) => out.push(Instruction::F64ConvertI32U),
                    (ValType::I32, ValType::F32) => out.push(Instruction::F32ConvertI32U),
                    (ValType::I64, ValType::F64) => out.push(Instruction::F64ConvertI64U),
                    (ValType::I64, ValType::F32) => out.push(Instruction::F32ConvertI64U),
                    _ => Self::emit_typed_coerce(from_wt, to_wt, out)?,
                }
            } else {
                Self::emit_typed_coerce(from_wt, to_wt, out)?;
            }
        }
        Ok(())
    }

    pub(crate) fn coerce_and_set_local(
        from: &GoType,
        to: &GoType,
        local_idx: u32,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        Self::coerce_go_types(from, to, out)?;
        out.push(Instruction::LocalSet(local_idx));
        Ok(())
    }

    pub(crate) fn coerce_and_set_global(
        from: &GoType,
        to: &GoType,
        global_idx: u32,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        Self::coerce_go_types(from, to, out)?;
        out.push(Instruction::GlobalSet(global_idx));
        Ok(())
    }

    pub(crate) fn emit_compound_op(
        &self,
        op: &Operator,
        vt: ValType,
        is_unsigned: bool,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        match op {
            Operator::AddAssign => out.push(Self::typed_add(vt)),
            Operator::SubAssign => out.push(Self::typed_sub(vt)),
            Operator::MulAssign => out.push(Self::typed_mul(vt)),
            Operator::QuoAssign => out.push(Self::typed_div(vt, is_unsigned)),
            Operator::RemAssign
            | Operator::AndAssign
            | Operator::OrAssign
            | Operator::XorAssign
            | Operator::ShlAssign
            | Operator::ShrAssign
            | Operator::AndNotAssign
                if matches!(vt, ValType::F32 | ValType::F64) =>
            {
                return Err(Error::InternalError(format!(
                    "operator {:?} is not valid on floating-point type {:?}",
                    op, vt
                )));
            }
            Operator::RemAssign => match vt {
                ValType::I32 => out.push(if is_unsigned { Instruction::I32RemU } else { Instruction::I32RemS }),
                _ => out.push(if is_unsigned { Instruction::I64RemU } else { Instruction::I64RemS }),
            },
            Operator::AndAssign => match vt {
                ValType::I32 => out.push(Instruction::I32And),
                _ => out.push(Instruction::I64And),
            },
            Operator::OrAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Or),
                _ => out.push(Instruction::I64Or),
            },
            Operator::XorAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Xor),
                _ => out.push(Instruction::I64Xor),
            },
            Operator::ShlAssign => match vt {
                ValType::I32 => out.push(Instruction::I32Shl),
                _ => out.push(Instruction::I64Shl),
            },
            Operator::ShrAssign => match vt {
                ValType::I32 => out.push(if is_unsigned { Instruction::I32ShrU } else { Instruction::I32ShrS }),
                _ => out.push(if is_unsigned { Instruction::I64ShrU } else { Instruction::I64ShrS }),
            },
            Operator::AndNotAssign => match vt {
                ValType::I32 => {
                    out.push(Instruction::I32Const(-1));
                    out.push(Instruction::I32Xor);
                    out.push(Instruction::I32And);
                }
                _ => {
                    out.push(Instruction::I64Const(-1));
                    out.push(Instruction::I64Xor);
                    out.push(Instruction::I64And);
                }
            },
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported compound operator: {:?}",
                    op
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn emit_deferred_calls(&self, out: &mut Vec<Instruction<'static>>) {
        if let Some(scope) = self.deferred_calls.last() {
            for call in scope.iter().rev() {
                // Write current named return locals into the closure environment
                // so the deferred closure sees the values set by the return statement.
                for nrc in &call.named_return_captures {
                    out.push(Instruction::LocalGet(nrc.env_local));
                    out.push(Instruction::LocalGet(nrc.named_return_local));
                    match nrc.val_type {
                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                        _ => out.push(Instruction::I32Store(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                    }
                }

                for (local_idx, _vt) in &call.arg_locals {
                    out.push(Instruction::LocalGet(*local_idx));
                }
                out.push(Instruction::Call(call.func_idx));
                let n_results = self.functions.iter()
                    .find(|f| f.wasm_func_idx == call.func_idx)
                    .map_or(0, |f| f.results.len());
                for _ in 0..n_results {
                    out.push(Instruction::Drop);
                }

                for nrc in &call.named_return_captures {
                    out.push(Instruction::LocalGet(nrc.env_local));
                    match nrc.val_type {
                        ValType::I64 => out.push(Instruction::I64Load(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F64 => out.push(Instruction::F64Load(MemArg {
                            offset: nrc.env_offset as u64, align: 3, memory_index: 0,
                        })),
                        ValType::F32 => out.push(Instruction::F32Load(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                        _ => out.push(Instruction::I32Load(MemArg {
                            offset: nrc.env_offset as u64, align: 2, memory_index: 0,
                        })),
                    }
                    out.push(Instruction::LocalSet(nrc.named_return_local));
                }
            }
        }
    }
}
