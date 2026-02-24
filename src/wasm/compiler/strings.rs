use super::*;

impl WasmCompiler {
    pub(crate) fn emit_string_eq_from_locals(
        tag_ptr: u32,
        tag_len: u32,
        case_ptr: u32,
        case_len: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) {
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

    pub(crate) fn emit_string_concat(
        &mut self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Compile both strings: each pushes (ptr, len)
        self.compile_expression(lhs, out, locals)?;
        self.compile_expression(rhs, out, locals)?;

        let len2 = locals.add_local("__scat_len2", ValType::I32);
        let ptr2 = locals.add_local("__scat_ptr2", ValType::I32);
        let len1 = locals.add_local("__scat_len1", ValType::I32);
        let ptr1 = locals.add_local("__scat_ptr1", ValType::I32);

        out.push(Instruction::LocalSet(len2));
        out.push(Instruction::LocalSet(ptr2));
        out.push(Instruction::LocalSet(len1));
        out.push(Instruction::LocalSet(ptr1));

        // Total length (with overflow check)
        let total_len = locals.add_local("__scat_total", ValType::I32);
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::LocalGet(len2));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalTee(total_len));

        // Overflow: if total_len < len1 then the add wrapped around
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::I32LtU);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Call(self.oom_func_idx));
        out.push(Instruction::Unreachable);
        out.push(Instruction::End);

        // Allocate buffer
        out.push(Instruction::LocalGet(total_len));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let new_ptr = locals.add_local("__scat_new", ValType::I32);
        out.push(Instruction::LocalSet(new_ptr));

        // Copy first string: memory.copy(new_ptr, ptr1, len1)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(ptr1));
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        // Copy second string: memory.copy(new_ptr + len1, ptr2, len2)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(len1));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(ptr2));
        out.push(Instruction::LocalGet(len2));
        out.push(Instruction::MemoryCopy {
            dst_mem: 0,
            src_mem: 0,
        });

        // Push result (ptr, len)
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(total_len));

        Ok(())
    }

    /// Compile a single expression and convert its result to a (ptr, len) string pair on the stack.

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

        // Detect bool literals
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
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32Const(0));
            }
        }
        Ok(())
    }

    /// Emit code to produce the string "true" or "false" as (ptr, len) on the stack.

    pub(crate) fn emit_bool_to_string(
        &mut self,
        val: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let s = if val { b"true" as &[u8] } else { b"false" as &[u8] };
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
        Ok(())
    }

    /// Takes a (ptr, len) string on the stack and appends a newline, producing a new (ptr, len).

    pub(crate) fn emit_append_newline(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
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

        // Copy original string
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(src_ptr));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Write newline at end
        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(src_len));
        out.push(Instruction::I32Add);
        out.push(Instruction::I32Const(b'\n' as i32));
        out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

        out.push(Instruction::LocalGet(new_ptr));
        out.push(Instruction::LocalGet(new_len));
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

        let len2 = locals.add_local(&format!("__scmp_len2_{}", locals.locals.len()), ValType::I32);
        let ptr2 = locals.add_local(&format!("__scmp_ptr2_{}", locals.locals.len()), ValType::I32);
        let len1 = locals.add_local(&format!("__scmp_len1_{}", locals.locals.len()), ValType::I32);
        let ptr1 = locals.add_local(&format!("__scmp_ptr1_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(len2));
        out.push(Instruction::LocalSet(ptr2));
        out.push(Instruction::LocalSet(len1));
        out.push(Instruction::LocalSet(ptr1));

        if op == Operator::Equal || op == Operator::NotEqual {
            // Equality: compare lengths, then bytes
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
        } else {
            // Lexicographic comparison: <, >, <=, >=
            // result: -1 if lhs < rhs, 0 if equal, 1 if lhs > rhs
            let cmp = locals.add_local(&format!("__scmp_cmp_{}", locals.locals.len()), ValType::I32);
            let idx = locals.add_local(&format!("__scmp_idx_{}", locals.locals.len()), ValType::I32);
            let min_len = locals.add_local(&format!("__scmp_min_{}", locals.locals.len()), ValType::I32);
            let b1 = locals.add_local(&format!("__scmp_b1_{}", locals.locals.len()), ValType::I32);
            let b2 = locals.add_local(&format!("__scmp_b2_{}", locals.locals.len()), ValType::I32);

            // min_len = min(len1, len2)
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::LocalGet(len1));
            out.push(Instruction::LocalGet(len2));
            out.push(Instruction::I32LeU);
            out.push(Instruction::Select);
            out.push(Instruction::LocalSet(min_len));

            // cmp = 0 (equal so far)
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

            // If bytes were equal, compare lengths
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

            // Convert cmp to boolean based on operator
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

        Ok(())
    }

    pub(crate) fn emit_string_min_max(
        &mut self,
        args: &[ast::Expression],
        is_min: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
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

            // For min: if cmp > 0 (best > cur), replace best with cur
            // For max: if cmp < 0 (best < cur), replace best with cur
            let should_replace = if is_min {
                // cmp > 0 means best > cur, so cur is smaller
                out.push(Instruction::LocalGet(cmp));
                out.push(Instruction::I32Const(0));
                Instruction::I32GtS
            } else {
                // cmp < 0 means best < cur, so cur is larger
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
        Ok(())
    }

    pub(crate) fn emit_i64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Converts an i64 on the stack to a string (ptr, len).
        // Algorithm: write digits in reverse, then reverse in place.
        // Max i64 has 20 digits + sign = 21 chars.
        let val = locals.add_local(&format!("__itos_v_{}", locals.locals.len()), ValType::I64);
        let buf = locals.add_local(&format!("__itos_buf_{}", locals.locals.len()), ValType::I32);
        let pos = locals.add_local(&format!("__itos_pos_{}", locals.locals.len()), ValType::I32);
        let neg = locals.add_local(&format!("__itos_neg_{}", locals.locals.len()), ValType::I32);
        let slen = locals.add_local(&format!("__itos_len_{}", locals.locals.len()), ValType::I32);
        let final_ptr = locals.add_local(&format!("__itos_fp_{}", locals.locals.len()), ValType::I32);

        out.push(Instruction::LocalSet(val));

        // Allocate 24-byte temp buffer
        out.push(Instruction::I32Const(24));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(buf));

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(pos));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(neg));

        // Handle zero
        out.push(Instruction::LocalGet(val));
        out.push(Instruction::I64Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(pos));
        }
        out.push(Instruction::Else);
        {
            // Handle negative
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(0));
            out.push(Instruction::I64LtS);
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(neg));
                out.push(Instruction::I64Const(0));
                out.push(Instruction::LocalGet(val));
                out.push(Instruction::I64Sub);
                out.push(Instruction::LocalSet(val));
            }
            out.push(Instruction::End);

            // Extract digits in reverse
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Eqz);
            out.push(Instruction::BrIf(1));

            // digit = val % 10
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(10));
            out.push(Instruction::I64RemU);
            out.push(Instruction::I32WrapI64);
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            // val = val / 10
            out.push(Instruction::LocalGet(val));
            out.push(Instruction::I64Const(10));
            out.push(Instruction::I64DivU);
            out.push(Instruction::LocalSet(val));

            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(pos));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);

            // Reverse the digits in buf[0..pos]
            let lo = locals.add_local(&format!("__itos_lo_{}", locals.locals.len()), ValType::I32);
            let hi = locals.add_local(&format!("__itos_hi_{}", locals.locals.len()), ValType::I32);
            let tmp = locals.add_local(&format!("__itos_tmp_{}", locals.locals.len()), ValType::I32);

            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(lo));
            out.push(Instruction::LocalGet(pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(hi));

            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32GeU);
            out.push(Instruction::BrIf(1));

            // swap buf[lo] and buf[hi]
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalSet(tmp));

            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            out.push(Instruction::LocalGet(buf));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(tmp));
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));

            out.push(Instruction::LocalGet(lo));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(lo));
            out.push(Instruction::LocalGet(hi));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(hi));
            out.push(Instruction::Br(0));
            out.push(Instruction::End);
            out.push(Instruction::End);
        }
        out.push(Instruction::End); // end if zero/nonzero

        // Build final string: if negative, prepend '-'
        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::LocalGet(pos));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalSet(slen));

        out.push(Instruction::LocalGet(slen));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(final_ptr));

        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::If(BlockType::Empty));
        {
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
        }
        out.push(Instruction::End);

        // Copy digits from buf to final_ptr+neg
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(neg));
        out.push(Instruction::I32Add);
        out.push(Instruction::LocalGet(buf));
        out.push(Instruction::LocalGet(pos));
        out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

        // Push (ptr, len)
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(slen));

        Ok(())
    }

    pub(crate) fn emit_f64_to_string(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let fval = locals.add_local(&format!("__ftos_v_{}", locals.locals.len()), ValType::F64);
        let abs_val = locals.add_local(&format!("__ftos_abs_{}", locals.locals.len()), ValType::F64);
        let is_neg = locals.add_local(&format!("__ftos_neg_{}", locals.locals.len()), ValType::I32);
        let int_part = locals.add_local(&format!("__ftos_ip_{}", locals.locals.len()), ValType::I64);
        let frac_val = locals.add_local(&format!("__ftos_fv_{}", locals.locals.len()), ValType::F64);
        let frac_int = locals.add_local(&format!("__ftos_fi_{}", locals.locals.len()), ValType::I64);
        let int_ptr = locals.add_local(&format!("__ftos_iptr_{}", locals.locals.len()), ValType::I32);
        let int_len = locals.add_local(&format!("__ftos_ilen_{}", locals.locals.len()), ValType::I32);
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

        // abs_val = abs(fval)
        out.push(Instruction::LocalGet(fval));
        out.push(Instruction::F64Abs);
        out.push(Instruction::LocalSet(abs_val));

        // int_part = i64(trunc(abs_val))
        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::F64Floor);
        out.push(Instruction::I64TruncF64S);
        out.push(Instruction::LocalSet(int_part));

        // frac_val = abs_val - f64(int_part)
        out.push(Instruction::LocalGet(abs_val));
        out.push(Instruction::LocalGet(int_part));
        out.push(Instruction::F64ConvertI64S);
        out.push(Instruction::F64Sub);
        out.push(Instruction::LocalSet(frac_val));

        // Convert int_part to string using existing helper
        out.push(Instruction::LocalGet(int_part));
        self.emit_i64_to_string(out, locals)?;
        out.push(Instruction::LocalSet(int_len));
        out.push(Instruction::LocalSet(int_ptr));

        // Check if frac is zero (frac_val < 1e-9)
        out.push(Instruction::LocalGet(frac_val));
        out.push(Instruction::F64Const(1e-9_f64.into()));
        out.push(Instruction::F64Lt);
        out.push(Instruction::If(BlockType::Empty));
        {
            // No fractional part: build sign + int_str
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(total_len));
            out.push(Instruction::LocalGet(total_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(final_ptr));

            // If negative, write '-'
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::End);

            // Copy int digits
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(int_ptr));
            out.push(Instruction::LocalGet(int_len));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
        }
        out.push(Instruction::Else);
        {
            // Has fractional part: always generate 6 fractional digits then strip trailing '0's
            // frac_int = i64(round(frac_val * 1e6))
            out.push(Instruction::LocalGet(frac_val));
            out.push(Instruction::F64Const(1e6_f64.into()));
            out.push(Instruction::F64Mul);
            out.push(Instruction::F64Const(0.5_f64.into()));
            out.push(Instruction::F64Add);
            out.push(Instruction::F64Floor);
            out.push(Instruction::I64TruncF64S);
            out.push(Instruction::LocalSet(frac_int));

            // Allocate 8-byte buffer and write exactly 6 digits (right-to-left)
            out.push(Instruction::I32Const(8));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            out.push(Instruction::LocalSet(frac_buf));

            {
                let digit_idx = locals.add_local(&format!("__ftos_di_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::I32Const(5));
                out.push(Instruction::LocalSet(digit_idx));

                // Write 6 digits right-to-left
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

            // frac_pos = 6 initially
            out.push(Instruction::I32Const(6));
            out.push(Instruction::LocalSet(frac_pos));

            // Strip trailing '0' characters from frac_buf
            out.push(Instruction::Block(BlockType::Empty));
            out.push(Instruction::Loop(BlockType::Empty));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32LeS);
            out.push(Instruction::BrIf(1)); // keep at least 1 digit

            out.push(Instruction::LocalGet(frac_buf));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::I32Const(48)); // '0'
            out.push(Instruction::I32Ne);
            out.push(Instruction::BrIf(1)); // stop if not '0'

            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Sub);
            out.push(Instruction::LocalSet(frac_pos));
            out.push(Instruction::Br(0));
            out.push(Instruction::End); // loop
            out.push(Instruction::End); // block

            // Build final: sign + int_str + '.' + frac_str
            // total = is_neg + int_len + 1 + frac_pos
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

            // Write '-' if negative
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::I32Const(45)); // '-'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(is_neg));
            out.push(Instruction::LocalSet(cursor));
            out.push(Instruction::End);

            // Copy integer digits
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

            // Write '.'
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(46)); // '.'
            out.push(Instruction::I32Store8(MemArg { offset: 0, align: 0, memory_index: 0 }));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(cursor));

            // Copy fractional digits
            out.push(Instruction::LocalGet(final_ptr));
            out.push(Instruction::LocalGet(cursor));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalGet(frac_buf));
            out.push(Instruction::LocalGet(frac_pos));
            out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });
        }
        out.push(Instruction::End); // end if frac == 0

        // Push result (ptr, len)
        out.push(Instruction::LocalGet(final_ptr));
        out.push(Instruction::LocalGet(total_len));

        Ok(())
    }
}
