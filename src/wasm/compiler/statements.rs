use super::*;

impl WasmCompiler {
    pub(crate) fn compile_block(
        &mut self,
        block: &ast::BlockStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        locals.push_scope();
        self.symbols.enter_scope();
        for stmt in &block.list {
            self.compile_statement(stmt, out, locals, result_types)?;
        }
        self.symbols.leave_scope();
        locals.pop_scope();
        Ok(())
    }

    pub(crate) fn compile_statement(
        &mut self,
        stmt: &ast::Statement,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        match stmt {
            ast::Statement::Return(ret) => self.compile_return(ret, out, locals, result_types),
            ast::Statement::Expr(expr_stmt) => {
                if let ast::Expression::Call(call) = &expr_stmt.expr {
                    if let ast::Expression::Ident(ident) = call.func.as_ref() {
                        match ident.name.as_str() {
                            "append" | "cap" | "complex" | "imag" | "len" | "make" | "new" | "real" => {
                                return Err(Error::SyntaxError(format!(
                                    "{}() not used (value is discarded); not permitted in statement context",
                                    ident.name
                                )));
                            }
                            _ => {}
                        }
                    }
                }
                self.compile_expression(&expr_stmt.expr, out, locals)?;
                let wasm_types = self.expression_result_count(&expr_stmt.expr, Some(locals));
                // #region agent log
                if wasm_types > 0 {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                        let expr_desc = match &expr_stmt.expr {
                            ast::Expression::Call(c) => format!("call:{:?}", c.func),
                            other => format!("{:?}", other),
                        };
                        let _ = writeln!(f, r#"{{"hypothesisId":"E","location":"statements.rs:ExprStmt","message":"dropping values","data":{{"count":{},"expr":"{}","pkg":"{}"}},"timestamp":{}}}"#,
                            wasm_types, expr_desc.replace('"', "'").chars().take(200).collect::<String>(), self.current_package.as_deref().unwrap_or(""), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                    }
                }
                // #endregion
                for _ in 0..wasm_types {
                    out.push(Instruction::Drop);
                }
                Ok(())
            }
            ast::Statement::Assign(assign) => self.compile_assign(assign, out, locals),
            ast::Statement::If(if_stmt) => {
                self.compile_if(if_stmt, out, locals, result_types)
            }
            ast::Statement::For(for_stmt) => {
                self.compile_for(for_stmt, out, locals, result_types)
            }
            ast::Statement::Block(block) => {
                self.compile_block(block, out, locals, result_types)
            }
            ast::Statement::IncDec(incdec) => self.compile_incdec(incdec, out, locals),
            ast::Statement::Switch(switch) => {
                self.compile_switch(switch, out, locals, result_types)
            }
            ast::Statement::Branch(branch) => self.compile_branch(branch, out),
            ast::Statement::Declaration(decl_stmt) => {
                self.compile_decl_stmt(decl_stmt, out, locals)
            }
            ast::Statement::Defer(defer) => {
                // Compile arguments eagerly at the defer site
                let mut arg_locals = Vec::new();
                for arg in &defer.call.args {
                    let vt = self.infer_val_type(arg, locals);
                    self.compile_expression(arg, out, locals)?;
                    let temp = locals.add_local(
                        &format!("__defer_arg_{}", locals.locals.len()),
                        vt,
                    );
                    out.push(Instruction::LocalSet(temp));
                    arg_locals.push((temp, vt));
                }

                if let ast::Expression::FuncLit(_) = defer.call.func.as_ref() {
                    // Compile the closure literal — this adds it as a function
                    self.compile_expression(&defer.call.func, out, locals)?;
                    // compile_func_lit pushes func_idx onto the WASM stack; drop it
                    // since we read it from last_closure_func_idx instead.
                    out.push(Instruction::Drop);
                    let closure_func_idx = self.last_closure_func_idx
                        .ok_or_else(|| Error::InternalError(
                            "defer: closure function index not set".to_string(),
                        ))?;

                    let env_local = self.last_closure_env;

                    // Build arg_locals for env_ptr + explicit args
                    let mut closure_arg_locals = Vec::new();
                    if let Some(el) = env_local {
                        closure_arg_locals.push((el, ValType::I32));
                    } else {
                        let zero_env = locals.add_local(
                            &format!("__defer_env_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::LocalSet(zero_env));
                        closure_arg_locals.push((zero_env, ValType::I32));
                    }
                    closure_arg_locals.extend(arg_locals);

                    let mut named_return_captures = Vec::new();
                    if let Some(el) = env_local {
                        for cap in &self.last_closure_captures {
                            if let Some((_, vt)) = self.named_returns.iter().find(|(n, _)| *n == cap.name) {
                                if let Some(nr_local) = locals.find(&cap.name) {
                                    named_return_captures.push(NamedReturnCapture {
                                        env_local: el,
                                        env_offset: cap.env_offset,
                                        named_return_local: nr_local,
                                        val_type: *vt,
                                    });
                                }
                            }
                        }
                    }

                    if let Some(deferred) = self.deferred_calls.last_mut() {
                        deferred.push(DeferredCall {
                            func_idx: closure_func_idx,
                            arg_locals: closure_arg_locals,
                            named_return_captures,
                        });
                    }
                    return Ok(());
                }

                let func_idx =
                    if let ast::Expression::Ident(ident) = defer.call.func.as_ref() {
                        self.functions
                            .iter()
                            .find(|f| f.name == ident.name)
                            .map(|f| f.wasm_func_idx)
                    } else if let ast::Expression::Selector(sel) =
                        defer.call.func.as_ref()
                    {
                        if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
                            let qname = format!("{}.{}", recv_ident.name, sel.sel.name);
                            let pkg_func = self.functions
                                .iter()
                                .find(|f| f.name == qname)
                                .map(|f| f.wasm_func_idx);

                            if pkg_func.is_some() {
                                pkg_func
                            } else if let Some(type_name) = locals.get_var_struct_type(&recv_ident.name).map(|s| s.to_string()) {
                                let method_qname = format!("{}.{}", type_name, sel.sel.name);
                                let method_func = self.functions
                                    .iter()
                                    .find(|f| f.name == method_qname)
                                    .map(|f| f.wasm_func_idx);
                                if let Some(idx) = method_func {
                                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                                    let recv_temp = locals.add_local(
                                        &format!("__defer_recv_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalSet(recv_temp));
                                    arg_locals.insert(0, (recv_temp, ValType::I32));
                                    Some(idx)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            // Chained selector: e.g., obj.field.Method()
                            // Compile the full receiver expression and resolve the method
                            let recv_type = self.infer_selector_struct_type(sel.x.as_ref(), locals);
                            if let Some(type_name) = recv_type {
                                let method_qname = format!("{}.{}", type_name, sel.sel.name);
                                let method_func = self.functions
                                    .iter()
                                    .find(|f| f.name == method_qname)
                                    .map(|f| f.wasm_func_idx);
                                if let Some(idx) = method_func {
                                    self.compile_expression(sel.x.as_ref(), out, locals)?;
                                    let recv_temp = locals.add_local(
                                        &format!("__defer_recv_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalSet(recv_temp));
                                    arg_locals.insert(0, (recv_temp, ValType::I32));
                                    Some(idx)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    } else {
                        None
                    };

                match func_idx {
                    Some(idx) => {
                        if let Some(deferred) = self.deferred_calls.last_mut() {
                            deferred.push(DeferredCall {
                                func_idx: idx,
                                arg_locals,
                                named_return_captures: Vec::new(),
                            });
                        }
                        Ok(())
                    }
                    None => Err(Error::InternalError(
                        "defer: could not resolve function".to_string(),
                    )),
                }
            }
            ast::Statement::Range(range) => {
                self.compile_range(range, out, locals, result_types)
            }
            ast::Statement::Empty(_) => Ok(()),
            ast::Statement::Label(labeled) => {
                match labeled.stmt.as_ref() {
                    ast::Statement::For(for_stmt) => {
                        self.compile_for_labeled(
                            for_stmt, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::Range(range) => {
                        self.compile_range_labeled(
                            range, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::Switch(sw) => {
                        self.compile_switch_labeled(
                            sw, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    ast::Statement::TypeSwitch(ts) => {
                        self.compile_type_switch_labeled(
                            ts, out, locals, result_types,
                            Some(labeled.name.name.clone()),
                        )
                    }
                    _ => self.compile_statement(&labeled.stmt, out, locals, result_types),
                }
            }
            ast::Statement::Go(_) => Err(Error::InternalError(
                "goroutines not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::Send(_) => Err(Error::InternalError(
                "channel send not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::Select(_) => Err(Error::InternalError(
                "select not supported in WASM UDFs".to_string(),
            )),
            ast::Statement::TypeSwitch(ts) => {
                self.compile_type_switch(ts, out, locals, result_types)
            }
        }
    }

    pub(crate) fn compile_return(
        &mut self,
        ret: &ast::ReturnStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        let has_deferred_named_return_captures = self.deferred_calls.last()
            .map_or(false, |scope| scope.iter().any(|dc| !dc.named_return_captures.is_empty()));

        let result_go_types = self.current_result_go_types.clone();

        // Detect `return f()` where f() returns multiple values
        let is_multi_return_forward = ret.ret.len() == 1
            && result_types.len() > 1
            && matches!(&ret.ret[0], ast::Expression::Call(_))
            && {
                let call_ret = self.call_return_val_types(&ret.ret[0], locals);
                call_ret.len() == result_types.len()
            };

        if ret.ret.is_empty() && !self.named_returns.is_empty() {
            self.emit_deferred_calls(out);
            for (name, _vt) in &self.named_returns.clone() {
                if let Some(idx) = locals.find(name) {
                    out.push(Instruction::LocalGet(idx));
                }
            }
        } else if !self.named_returns.is_empty() && has_deferred_named_return_captures {
            let named_returns_clone = self.named_returns.clone();
            if is_multi_return_forward {
                // `return f()` where f returns multiple values and we have named returns with defer captures.
                // Compile the call, then store each result into named return locals (in reverse stack order).
                self.compile_expression(&ret.ret[0], out, locals)?;
                for (name, _vt) in named_returns_clone.iter().rev() {
                    if let Some(idx) = locals.find(name) {
                        out.push(Instruction::LocalSet(idx));
                    }
                }
            } else {
                let mut ws = 0usize;
                for (i, expr) in ret.ret.iter().enumerate() {
                    self.compile_expression(expr, out, locals)?;
                    let is_str = self.is_string_expr(expr, locals);
                    if is_str {
                        if self.gc_builtin_types.go_string.is_some() {
                            ws += 1;
                        } else {
                            ws += 2;
                        }
                    } else {
                        if let Some(&expected_vt) = result_types.get(ws) {
                            let actual_vt = self.infer_val_type(expr, locals);
                            if actual_vt != expected_vt
                                && !matches!(actual_vt, ValType::Ref(_))
                                && !matches!(expected_vt, ValType::Ref(_))
                            {
                                Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                            }
                        }
                        ws += 1;
                    }
                    if let Some((name, _vt)) = named_returns_clone.get(i) {
                        if let Some(idx) = locals.find(name) {
                            out.push(Instruction::LocalSet(idx));
                        }
                    }
                }
            }
            self.emit_deferred_calls(out);
            for (name, _vt) in &named_returns_clone {
                if let Some(idx) = locals.find(name) {
                    out.push(Instruction::LocalGet(idx));
                }
            }
        } else {
            let mut wasm_slot = 0usize;
            for (i, expr) in ret.ret.iter().enumerate() {
                let is_iface_return = result_go_types.get(i)
                    .map_or(false, |gt| self.is_iface_go_type(gt));
                let is_nil = matches!(expr, ast::Expression::Ident(id) if id.name == "nil");
                let is_iface_field_sel = self.is_interface_field_selector(expr, locals);
                if is_iface_return && is_nil {
                    self.emit_nil_iface_box(out, locals)?;
                    wasm_slot += 1;
                } else if is_iface_return && is_iface_field_sel {
                    self.compile_expression(expr, out, locals)?;
                    let tid_tmp = locals.add_local(
                        &format!("__ret_isel_tid_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    let data_tmp = locals.add_local(
                        &format!("__ret_isel_data_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(tid_tmp));
                    out.push(Instruction::LocalSet(data_tmp));
                    out.push(Instruction::I32Const(8));
                    out.push(Instruction::Call(self.alloc_func_idx()?));
                    let wrapper = locals.add_local(
                        &format!("__ret_isel_w_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    out.push(Instruction::LocalSet(wrapper));
                    out.push(Instruction::LocalGet(wrapper));
                    out.push(Instruction::LocalGet(tid_tmp));
                    out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalGet(wrapper));
                    out.push(Instruction::LocalGet(data_tmp));
                    out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    out.push(Instruction::LocalGet(wrapper));
                    wasm_slot += 1;
                } else if is_iface_return && !self.is_interface_var_expr(expr, locals) {
                    self.emit_return_iface_box(expr, out, locals)?;
                    wasm_slot += 1;
                } else {
                    self.compile_expression(expr, out, locals)?;
                    let is_string = self.is_string_expr(expr, locals);
                    if is_string {
                        if self.gc_builtin_types.go_string.is_some() {
                            wasm_slot += 1;
                        } else {
                            wasm_slot += 2;
                        }
                    } else {
                        if let Some(&expected_vt) = result_types.get(wasm_slot) {
                            let actual_vt = self.infer_val_type(expr, locals);
                            if actual_vt != expected_vt
                                && !matches!(actual_vt, ValType::Ref(_))
                                && !matches!(expected_vt, ValType::Ref(_))
                            {
                                Self::emit_typed_coerce(actual_vt, expected_vt, out)?;
                            }
                        }
                        wasm_slot += 1;
                    }
                }
            }
            self.emit_deferred_calls(out);
        }
        out.push(Instruction::Return);
        Ok(())
    }

    pub(crate) fn compile_assign(
        &mut self,
        assign: &ast::AssignStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let is_define = assign.op == Operator::Define;

        if is_define {
            // Validate short variable declaration rules
            let mut seen_names: Vec<&str> = Vec::new();
            let mut has_new = false;
            for left in &assign.left {
                if let ast::Expression::Ident(ident) = left {
                    if ident.name == "_" {
                        continue;
                    }
                    if seen_names.contains(&ident.name.as_str()) {
                        return Err(Error::SyntaxError(format!(
                            "{} repeated on left side of :=",
                            ident.name
                        )));
                    }
                    seen_names.push(&ident.name);
                    if locals.find_at_current_scope(&ident.name).is_none() {
                        has_new = true;
                    }
                }
            }
            if !seen_names.is_empty() && !has_new {
                return Err(Error::SyntaxError(
                    "no new variables on left side of :=".to_string(),
                ));
            }

            // Map comma-ok: v, ok := m[key]
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::Index(idx_expr) = &assign.right[0] {
                    if let Some(left_expr) = idx_expr.left.as_deref() {
                        if let ast::Expression::Ident(map_ident) = left_expr {
                            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_val".to_string()
                                };
                                let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_flag".to_string()
                                };
                                let map_name = map_ident.name.clone();
                                let key_expr = idx_expr.index.clone();
                                return self.compile_map_get_ok(&map_name, &key_expr, &val_var, &ok_var, out, locals);
                            }
                        }
                    }
                }
            }

            // Type assertion comma-ok: v, ok := x.(T)
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::TypeAssert(ta) = &assign.right[0] {
                    if ta.right.is_some() {
                        let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                            id.name.clone()
                        } else {
                            "__ta_val".to_string()
                        };
                        let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                            id.name.clone()
                        } else {
                            "__ta_ok".to_string()
                        };
                        return self.compile_type_assert_ok(ta, &val_var, &ok_var, out, locals);
                    }
                }
            }

            // Multi-return: single function call on the right, multiple vars on the left
            if assign.right.len() == 1 && assign.left.len() > 1 {
                if let ast::Expression::Call(_) = &assign.right[0] {
                    let ret_types = self.call_return_val_types(&assign.right[0], locals);
                    let go_types = self.call_return_go_types(&assign.right[0], locals);
                    if ret_types.is_empty() {
                        let call_desc = if let ast::Expression::Call(c) = &assign.right[0] {
                            format!("{:?}", c.func)
                        } else { "?".to_string() };
                        return Err(Error::InternalError(format!(
                            "assignment mismatch: {} variables but function returns no values (call: {})",
                            assign.left.len(),
                            call_desc
                        )));
                    }
                    let gc_strings = self.gc_builtin_types.go_string.is_some();
                    let go_count = Self::count_go_level_returns(&go_types, gc_strings);
                    if go_count != assign.left.len() {
                        return Err(Error::InternalError(format!(
                            "assignment mismatch: {} variables but function returns {} values",
                            assign.left.len(),
                            go_count,
                        )));
                    }
                    return self.compile_multi_return_define(assign, &ret_types, &go_types, out, locals);
                }
            }

            for (i, left) in assign.left.iter().enumerate() {
                if let ast::Expression::Ident(ident) = left {
                    if ident.name == "_" {
                        if i < assign.right.len() {
                            self.compile_expression(&assign.right[i], out, locals)?;
                            out.push(Instruction::Drop);
                        }
                        continue;
                    }

                    let is_string_rhs = if i < assign.right.len() {
                        self.is_string_expr(&assign.right[i], locals)
                    } else {
                        false
                    };

                    let gc_string_mode = is_string_rhs && self.gc_builtin_types.go_string.is_some();

                    let vt = if is_string_rhs && gc_string_mode {
                        Self::gc_ref_val_type(self.gc_builtin_types.go_string.unwrap())
                    } else if is_string_rhs {
                        ValType::I32
                    } else if i < assign.right.len() {
                        self.infer_val_type(&assign.right[i], locals)
                    } else {
                        ValType::I32
                    };

                    let local_idx = if let Some(existing) = locals.find_at_current_scope(&ident.name) {
                        existing
                    } else {
                        locals.add_local(&ident.name, vt)
                    };

                    if i < assign.right.len() {
                        if let Some(dt) = self.infer_define_type_from_expr(&assign.right[i], locals) {
                            self.define_var(&ident.name, dt);
                        }
                        // Track struct type from composite literals
                        if let ast::Expression::CompositeLit(comp) = &assign.right[i] {
                            if let ast::Expression::Ident(type_ident) = comp.typ.as_ref()
                            {
                                if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                } else {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                }
                            }
                        }

                        // Track type from &expr (address-of)
                        if let ast::Expression::Operation(addr_op) = &assign.right[i] {
                            if addr_op.op == Operator::And && addr_op.y.is_none() {
                                if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                    if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            &type_ident.name,
                                        );
                                        if self.struct_defs.contains_key(&type_ident.name) {
                                            locals.pointer_to_struct_vars.insert(ident.name.clone());
                                        }
                                    }
                                } else if let ast::Expression::Index(idx_expr) = &*addr_op.x {
                                    if let Some(ast::Expression::Ident(arr_ident)) = idx_expr.left.as_deref() {
                                        if let Some(elem_struct_type) = locals.slice_elem_struct_types
                                            .get(&arr_ident.name).cloned()
                                        {
                                            locals.set_var_struct_type(&ident.name, &elem_struct_type);
                                            if self.struct_defs.contains_key(&elem_struct_type) {
                                                locals.pointer_to_struct_vars.insert(ident.name.clone());
                                            }
                                        }
                                    }
                                } else if let ast::Expression::Ident(ref_ident) = &*addr_op.x {
                                    let ptr_tag = if let Some(st) = locals.get_var_struct_type(&ref_ident.name).map(|s| s.to_string()) {
                                        if self.struct_defs.contains_key(&st) {
                                            locals.pointer_to_struct_vars.insert(ident.name.clone());
                                            Some(st)
                                        } else {
                                            None
                                        }
                                    } else {
                                        let vt = self.infer_val_type(&addr_op.x, locals);
                                        Some(match vt {
                                            ValType::I64 => "__ptr_i64".to_string(),
                                            ValType::F32 => "__ptr_f32".to_string(),
                                            ValType::F64 => "__ptr_f64".to_string(),
                                            _ => "__ptr_i32".to_string(),
                                        })
                                    };
                                    if let Some(tag) = ptr_tag {
                                        locals.set_var_struct_type(&ident.name, &tag);
                                    }
                                }
                            }
                        }

                        if let ast::Expression::Selector(sel) = &assign.right[i] {
                            if let Some(parent_type) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                                if let Some(sd) = self.struct_defs.get(&parent_type) {
                                    if let Some(field) = sd.find_field(&sel.sel.name) {
                                        if field.go_type_tag.as_deref() == Some("__slice") {
                                            locals.set_var_struct_type(&ident.name, "__slice");
                                            if let Some(ref elem_tag) = field.slice_elem_type_tag {
                                                locals.slice_elem_struct_types.insert(
                                                    ident.name.clone(),
                                                    elem_tag.clone(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if self.is_unsigned_expr(&assign.right[i], locals) {
                            locals.unsigned_vars.insert(ident.name.clone());
                        }

                        if is_string_rhs {
                            self.track_local_var_type(&ident.name, "string", local_idx, locals);
                        }

                        // Track type alias from arithmetic expressions with typed constants
                        if !is_string_rhs && locals.get_var_struct_type(&ident.name).is_none() {
                            if let Some(alias_type) = self.infer_type_alias_from_expr(&assign.right[i]) {
                                locals.set_var_struct_type(&ident.name, &alias_type);
                            }
                        }

                        // Track named return type from single-return function/method calls
                        if locals.get_var_struct_type(&ident.name).is_none() {
                            if let ast::Expression::Call(call_expr) = &assign.right[i] {
                                if let Some(ret_type) = self.infer_return_struct_type(call_expr, locals) {
                                    locals.set_var_struct_type(&ident.name, &ret_type);
                                }
                            }
                        }

                        // Track slice variables from make/append calls
                        if let ast::Expression::Call(call_expr) = &assign.right[i] {
                            if let ast::Expression::Ident(fn_ident) =
                                call_expr.func.as_ref()
                            {
                                if fn_ident.name == "make" {
                                    if let Some(ast::Expression::TypeMap(map_type)) = call_expr.args.first() {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            "__map",
                                        );
                                        let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                        let nested = self.build_nested_map_type_info(map_type);
                                        locals.map_types.insert(
                                            ident.name.clone(),
                                            MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                        );
                                    } else {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            "__slice",
                                        );
                                        let elem_vt =
                                            Self::infer_slice_elem_type(
                                                call_expr.args.first(),
                                            );
                                        locals.slice_elem_types.insert(
                                            ident.name.clone(),
                                            elem_vt,
                                        );
                                        if let Some(ast::Expression::TypeSlice(slice_type)) = call_expr.args.first() {
                                            if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                                let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                                locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                            }
                                            if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                                let resolved_elem = self.resolve_struct_in_pkg(&el_id.name);
                                                if self.struct_defs.contains_key(&resolved_elem) {
                                                    locals.slice_elem_struct_types.insert(ident.name.clone(), resolved_elem);
                                                }
                                            }
                                        }
                                    }
                                } else if fn_ident.name == "append" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__slice",
                                    );
                                } else if fn_ident.name == "new" {
                                    if let Some(type_arg) = call_expr.args.first() {
                                        if let ast::Expression::Ident(ti) = type_arg {
                                            let resolved_new = self.resolve_struct_in_pkg(&ti.name);
                                            let ptr_tag = match ti.name.as_str() {
                                                "int" | "int64" | "uint" | "uint64" => "__ptr_i64".to_string(),
                                                "float32" => "__ptr_f32".to_string(),
                                                "float64" => "__ptr_f64".to_string(),
                                                _ => {
                                                    if self.struct_defs.contains_key(&resolved_new) {
                                                        locals.pointer_to_struct_vars.insert(ident.name.clone());
                                                        resolved_new
                                                    } else if self.struct_defs.contains_key(&ti.name) {
                                                        locals.pointer_to_struct_vars.insert(ident.name.clone());
                                                        ti.name.clone()
                                                    } else {
                                                        "__ptr_i32".to_string()
                                                    }
                                                }
                                            };
                                            locals.set_var_struct_type(
                                                &ident.name,
                                                &ptr_tag,
                                            );
                                        }
                                    }
                                } else if fn_ident.name == "complex" {
                                    let is_c64 = call_expr.args.first().map_or(false, |a| {
                                        self.infer_val_type(a, locals) == ValType::F32
                                    });
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        if is_c64 { "__complex64" } else { "__complex128" },
                                    );
                                }
                            }

                            // Track []byte(s) and []rune(s) type conversions as slice variables
                            if let ast::Expression::TypeSlice(slice_type) = call_expr.func.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    if el_id.name == "rune" || el_id.name == "int32" {
                                        locals.rune_slices.insert(ident.name.clone());
                                    }
                                }
                            }

                            // Track [N]T(slice) slice-to-array conversions
                            if let ast::Expression::TypeArray(arr_type) = call_expr.func.as_ref() {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else {
                                    0
                                };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                let (go_es, go_ea) = Self::go_type_elem_size_and_align(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len, go_es, go_ea));
                            }
                        }

                        // Track struct type from function/method call return types
                        let mut is_iface_from_call = false;
                        if let ast::Expression::Call(call_expr) = &assign.right[i] {
                            if let ast::Expression::Selector(sel) = call_expr.func.as_ref() {
                                if let ast::Expression::Ident(pkg_id) = sel.x.as_ref() {
                                    let qualified_alias = format!("{}.{}", pkg_id.name, sel.sel.name);
                                    if self.type_aliases.contains_key(&qualified_alias) {
                                        locals.set_var_struct_type(&ident.name, &qualified_alias);
                                    }
                                }
                            } else if let ast::Expression::Ident(fn_id) = call_expr.func.as_ref() {
                                let resolved = self.resolve_struct_in_pkg(&fn_id.name);
                                if self.type_aliases.contains_key(&resolved) {
                                    locals.set_var_struct_type(&ident.name, &resolved);
                                }
                            }

                            let call_go_types = self.call_return_go_types(&assign.right[i], locals);
                            if let Some(go_type) = call_go_types.first() {
                                is_iface_from_call = self.track_local_var_type(&ident.name, go_type, local_idx, locals);
                            }
                            if self.is_string_expr(&assign.right[i], locals) {
                                locals.set_var_struct_type(&ident.name, "__string");
                            }
                        }

                        // Track imaginary literal assignments
                        if let ast::Expression::BasicLit(lit) = &assign.right[i] {
                            if lit.kind == LitKind::Imag {
                                locals.set_var_struct_type(
                                    &ident.name,
                                    "__complex128",
                                );
                            }
                        }

                        // Track composite literal assignments
                        if let ast::Expression::CompositeLit(comp) = &assign.right[i] {
                            if let ast::Expression::TypeArray(arr_type) = comp.typ.as_ref() {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else if matches!(arr_type.len.as_ref(), ast::Expression::Ellipsis(_)) {
                                    comp.val.values.len() as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                let (go_es, go_ea) = Self::go_type_elem_size_and_align(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len, go_es, go_ea));
                            } else if let ast::Expression::TypeSlice(slice_type) = comp.typ.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                    locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                }
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    if el_id.name == "rune" || el_id.name == "int32" {
                                        locals.rune_slices.insert(ident.name.clone());
                                    }
                                    let resolved_elem = self.resolve_struct_in_pkg(&el_id.name);
                                    if self.struct_defs.contains_key(&resolved_elem) {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), resolved_elem);
                                    }
                                }
                            } else if let ast::Expression::TypeMap(map_type) = comp.typ.as_ref() {
                                locals.set_var_struct_type(&ident.name, "__map");
                                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                let nested = self.build_nested_map_type_info(map_type);
                                locals.map_types.insert(
                                    ident.name.clone(),
                                    MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                );
                            } else if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                } else if self.struct_defs.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(&ident.name, &type_ident.name);
                                }
                            } else if let ast::Expression::Index(idx) = comp.typ.as_ref() {
                                // Generic type instantiation: Pair[int]{...}
                                if let Some(ast::Expression::Ident(type_ident)) = idx.left.as_ref().map(|l| l.as_ref()) {
                                    if self.generic_types.contains_key(&type_ident.name) {
                                        let type_arg = Self::type_expr_to_go_string(&idx.index);
                                        if let Ok(mono_name) = self.monomorphize_generic_type(&type_ident.name, &[type_arg]) {
                                            locals.set_var_struct_type(&ident.name, &mono_name);
                                        }
                                    }
                                }
                            }
                        }

                        // Track reslice assignments: t := s[lo:hi] or t := s[lo:hi:max]
                        if let ast::Expression::Slice(sl) = &assign.right[i] {
                            if !self.is_string_expr(&assign.right[i], locals) {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                if let ast::Expression::Ident(src_ident) = &*sl.left {
                                    if let Some(&evtype) = locals.slice_elem_types.get(&src_ident.name) {
                                        locals.slice_elem_types.insert(ident.name.clone(), evtype);
                                    }
                                    if let Some(&inner_vt) = locals.nested_slice_inner_elem_types.get(&src_ident.name) {
                                        locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                    }
                                    if let Some(st) = locals.slice_elem_struct_types.get(&src_ident.name).cloned() {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), st);
                                    }
                                }
                            }
                        }

                        // Track type assertion results as struct types
                        if let ast::Expression::TypeAssert(ta) = &assign.right[i] {
                            if let Some(ref target_type) = ta.right {
                                if let ast::Expression::Ident(type_id) = target_type.as_ref() {
                                    if self.struct_defs.contains_key(&type_id.name) {
                                        locals.set_var_struct_type(&ident.name, &type_id.name);
                                    }
                                }
                            }
                        }

                        // Track struct type from map or slice index result
                        if let ast::Expression::Index(idx_expr) = &assign.right[i] {
                            if let Some(left_expr) = idx_expr.left.as_deref() {
                                if let ast::Expression::Ident(src_ident) = left_expr {
                                    let map_val_struct = locals.map_types.get(&src_ident.name)
                                        .and_then(|mti| mti.val_struct_type.clone());
                                    if let Some(st) = map_val_struct {
                                        locals.set_var_struct_type(&ident.name, &st);
                                    } else if let Some(st) = locals.slice_elem_struct_types.get(&src_ident.name).cloned() {
                                        locals.set_var_struct_type(&ident.name, &st);
                                    }
                                }
                            }
                        }

                        if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                            if let Some(struct_type) = locals.get_var_struct_type(&rhs_ident.name).map(|s| s.to_string()) {
                                if locals.get_var_struct_type(&ident.name).is_none() {
                                    locals.set_var_struct_type(&ident.name, &struct_type);
                                }
                            }
                            if locals.pointer_to_struct_vars.contains(&rhs_ident.name) {
                                locals.pointer_to_struct_vars.insert(ident.name.clone());
                            }
                            if let Some(&info) = locals.array_info.get(&rhs_ident.name) {
                                if !locals.array_info.contains_key(&ident.name) {
                                    locals.array_info.insert(ident.name.clone(), info);
                                }
                            }
                        }

                        if let ast::Expression::FuncLit(_) = &assign.right[i] {
                            locals.closure_info.insert(
                                ident.name.clone(),
                                (self.next_func_idx, u32::MAX),
                            );
                        }

                        if self.current_stack_frame.is_some() {
                            let is_struct_lit = matches!(
                                &assign.right[i],
                                ast::Expression::CompositeLit(comp)
                                    if matches!(comp.typ.as_ref(),
                                        ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name))
                            );
                            let is_addr_of_struct_lit = matches!(
                                &assign.right[i],
                                ast::Expression::Operation(op)
                                    if op.y.is_none()
                                        && matches!(op.op, Operator::And)
                                        && matches!(op.x.as_ref(),
                                            ast::Expression::CompositeLit(comp)
                                                if matches!(comp.typ.as_ref(),
                                                    ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name)))
                            );
                            let is_new_struct = matches!(
                                &assign.right[i],
                                ast::Expression::Call(call)
                                    if matches!(call.func.as_ref(), ast::Expression::Ident(fn_id) if fn_id.name == "new")
                                        && call.args.first().map_or(false, |a|
                                            matches!(a, ast::Expression::Ident(ti) if self.struct_defs.contains_key(&ti.name)))
                            );
                            if is_struct_lit || is_addr_of_struct_lit || is_new_struct {
                                self.stack_alloc_target = Some(ident.name.clone());
                            }
                        }

                        let compile_result = self.compile_expression(&assign.right[i], out, locals);
                        self.stack_alloc_target = None;
                        compile_result?;
                        if is_iface_from_call {
                            let wrapper = locals.add_local(
                                &format!("__iface_wrap_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalSet(wrapper));
                            let tid_local = self.get_iface_type_id_local(&ident.name, locals).ok_or_else(|| {
                                Error::InternalError(format!(
                                    "interface type-id local not found for '{}'", ident.name
                                ))
                            })?;
                            out.push(Instruction::LocalGet(wrapper));
                            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalSet(tid_local));
                            out.push(Instruction::LocalGet(wrapper));
                            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                            out.push(Instruction::LocalSet(local_idx));
                        } else if is_string_rhs {
                            if let Some(&gc_ref_idx) = locals.gc_string_locals.get(&ident.name) {
                                out.push(Instruction::LocalSet(gc_ref_idx));
                            } else {
                                let (ptr_local, len_local) = locals.string_locals[&ident.name];
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            }
                        } else if let Some(&(mb_offset, mb_vt)) = locals.memory_backed_vars.get(&ident.name) {
                            if let Some(sf) = &self.current_stack_frame {
                                if let Some(fb) = sf.frame_base_local {
                                    let val_tmp = locals.add_local(
                                        &format!("__mb_val_{}", locals.locals.len()),
                                        mb_vt,
                                    );
                                    Self::emit_typed_coerce(vt, mb_vt, out)?;
                                    out.push(Instruction::LocalSet(val_tmp));
                                    out.push(Instruction::LocalGet(fb));
                                    if mb_offset > 0 {
                                        out.push(Instruction::I32Const(mb_offset as i32));
                                        out.push(Instruction::I32Add);
                                    }
                                    out.push(Instruction::LocalGet(val_tmp));
                                    let (_, align) = Self::elem_size_and_align(mb_vt);
                                    Self::emit_typed_store(mb_vt, 0, align, out);
                                }
                            }
                        } else {
                            let gc_copy = if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                self.get_gc_copy_info(&rhs_ident.name, locals)
                            } else { None };
                            if let Some((gc_idx, gc_sd)) = gc_copy {
                                Self::emit_gc_value_deep_copy(gc_idx, &gc_sd, out, locals);
                            } else {
                                let needs_deep_copy = if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                    self.get_value_copy_size(&rhs_ident.name, locals).is_some()
                                } else { false };
                                if needs_deep_copy {
                                    if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                        let size = self.get_value_copy_size(&rhs_ident.name, locals).unwrap();
                                        self.emit_value_deep_copy(size, out, locals)?;
                                    }
                                } else {
                                    let var_vt = locals.find_type(&ident.name).unwrap_or(vt);
                                    if vt != var_vt {
                                        Self::emit_typed_coerce(vt, var_vt, out)?;
                                    }
                                }
                            }
                            out.push(Instruction::LocalSet(local_idx));
                        }

                        if let Some(func_idx) = self.last_closure_func_idx.take() {
                            let env_local =
                                self.last_closure_env.take().unwrap_or(u32::MAX);
                            locals
                                .closure_info
                                .insert(ident.name.clone(), (func_idx, env_local));
                            let cap_info: Vec<(String, u32, ValType)> = self.last_closure_captures.iter()
                                .map(|c| (c.name.clone(), c.env_offset, c.val_type))
                                .collect();
                            locals.closure_env_captures.insert(ident.name.clone(), cap_info);
                        }
                        if let Some(func_idx) = self.last_func_value_idx.take() {
                            locals.closure_info.insert(ident.name.clone(), (func_idx, u32::MAX));
                            if self.last_is_method_expr {
                                locals.method_expr_vars.insert(ident.name.clone());
                                self.last_is_method_expr = false;
                            }
                        }
                    }
                }
            }
        } else {
            // Multi-return with =: a, b = func()
            if assign.right.len() == 1 && assign.left.len() > 1 {
                if let ast::Expression::Call(c) = &assign.right[0] {
                    let ret_types = self.call_return_val_types(&assign.right[0], locals);
                    let go_types = self.call_return_go_types(&assign.right[0], locals);
                    let gc_strings = self.gc_builtin_types.go_string.is_some();
                    let go_count = Self::count_go_level_returns(&go_types, gc_strings);
                    if go_count == assign.left.len() {
                        return self.compile_multi_return_define(assign, &ret_types, &go_types, out, locals);
                    }
                }
            }

            // Map comma-ok with =: v, ok = m[key]
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::Index(idx_expr) = &assign.right[0] {
                    if let Some(left_expr) = idx_expr.left.as_deref() {
                        if let ast::Expression::Ident(map_ident) = left_expr {
                            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_val".to_string()
                                };
                                let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                                    id.name.clone()
                                } else {
                                    "__comma_ok_flag".to_string()
                                };
                                let map_name = map_ident.name.clone();
                                let key_expr = idx_expr.index.clone();
                                return self.compile_map_get_ok(&map_name, &key_expr, &val_var, &ok_var, out, locals);
                            }
                        }
                    }
                }
            }

            // Type assertion comma-ok with =: v, ok = x.(T)
            if assign.right.len() == 1 && assign.left.len() == 2 {
                if let ast::Expression::TypeAssert(ta) = &assign.right[0] {
                    if ta.right.is_some() {
                        let val_var = if let ast::Expression::Ident(id) = &assign.left[0] {
                            id.name.clone()
                        } else {
                            "__ta_val".to_string()
                        };
                        let ok_var = if let ast::Expression::Ident(id) = &assign.left[1] {
                            id.name.clone()
                        } else {
                            "__ta_ok".to_string()
                        };
                        return self.compile_type_assert_ok(ta, &val_var, &ok_var, out, locals);
                    }
                }
            }

            let needs_parallel = assign.left.len() > 1
                && assign.right.len() > 1
                && assign.op == Operator::Assign;

            let mut par_temps: Vec<(u32, Option<u32>)> = Vec::new();
            if needs_parallel {
                for right in &assign.right {
                    let is_str = self.is_string_expr(right, locals);
                    self.compile_expression(right, out, locals)?;
                    if is_str {
                        let len_tmp = locals.add_local(
                            &format!("__par_l_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        let ptr_tmp = locals.add_local(
                            &format!("__par_p_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(len_tmp));
                        out.push(Instruction::LocalSet(ptr_tmp));
                        par_temps.push((ptr_tmp, Some(len_tmp)));
                    } else {
                        let vt = self.infer_val_type(right, locals);
                        let tmp = locals.add_local(
                            &format!("__par_{}", locals.locals.len()),
                            vt,
                        );
                        out.push(Instruction::LocalSet(tmp));
                        par_temps.push((tmp, None));
                    }
                }
            }

            for (i, left) in assign.left.iter().enumerate() {
                if needs_parallel {
                    if i < par_temps.len() {
                        let (val_tmp, len_tmp_opt) = par_temps[i];
                        out.push(Instruction::LocalGet(val_tmp));
                        if let Some(len_tmp) = len_tmp_opt {
                            out.push(Instruction::LocalGet(len_tmp));
                        }
                    }
                } else if i < assign.right.len() {
                    if let ast::Expression::Ident(lhs_ident) = left {
                        if let ast::Expression::FuncLit(_) = &assign.right[i] {
                            locals.closure_info.insert(
                                lhs_ident.name.clone(),
                                (self.next_func_idx, u32::MAX),
                            );
                        }
                    }
                    self.compile_expression(&assign.right[i], out, locals)?;
                }

                match left {
                    ast::Expression::Ident(ident) => {
                        if ident.name == "_" {
                            out.push(Instruction::Drop);
                            continue;
                        }
                        // Interface variable assignment: box value and set type_id
                        if self.is_interface_var(&ident.name, locals) {
                            if let Some(tid_local) = self.get_iface_type_id_local(&ident.name, locals) {
                                let data_local = locals.find(&ident.name).ok_or_else(|| {
                                    Error::InternalError(format!("interface var '{}' not found", ident.name))
                                })?;

                                let rhs_is_iface_call = (i < assign.right.len()
                                    && self.is_call_returning_interface(&assign.right[i], locals))
                                    || self.last_iface_call_returns_iface;
                                self.last_iface_call_returns_iface = false;
                                let rhs_is_iface_var = i < assign.right.len()
                                    && self.is_interface_var_expr(&assign.right[i], locals);

                                if rhs_is_iface_call {
                                    // RHS is a call returning an interface wrapper pointer.
                                    // Unbox: read type_id from wrapper[0], data_ptr from wrapper[4].
                                    let wrapper = locals.add_local(
                                        &format!("__iface_wrap_{}", locals.locals.len()),
                                        ValType::I32,
                                    );
                                    out.push(Instruction::LocalSet(wrapper));
                                    out.push(Instruction::LocalGet(wrapper));
                                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(tid_local));
                                    out.push(Instruction::LocalGet(wrapper));
                                    out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                                    out.push(Instruction::LocalSet(data_local));
                                    continue;
                                } else if rhs_is_iface_var {
                                    // RHS is another interface variable — copy type_id and data_ptr.
                                    if let ast::Expression::Ident(rhs_id) = &assign.right[i] {
                                        if let Some(rhs_tid) = self.get_iface_type_id_local(&rhs_id.name, locals) {
                                            let rhs_data = locals.find(&rhs_id.name).ok_or_else(|| {
                                                Error::InternalError(format!("variable '{}' not found", rhs_id.name))
                                            })?;
                                            // The compile_expression already pushed rhs data_local;
                                            // drop it — we'll use LocalGet directly.
                                            out.push(Instruction::Drop);
                                            out.push(Instruction::LocalGet(rhs_tid));
                                            out.push(Instruction::LocalSet(tid_local));
                                            out.push(Instruction::LocalGet(rhs_data));
                                            out.push(Instruction::LocalSet(data_local));
                                            continue;
                                        }
                                    }
                                }

                                let rhs_vt = if i < assign.right.len() {
                                    self.infer_val_type(&assign.right[i], locals)
                                } else {
                                    ValType::I64
                                };
                                let rhs_type_name = if i < assign.right.len() {
                                    self.infer_concrete_type_name(&assign.right[i], locals)
                                } else {
                                    "int".to_string()
                                };
                                if let Some(st) = locals.get_var_struct_type(&ident.name) {
                                    if let Some(iface_name) = st.strip_prefix("__iface_") {
                                        let iface_name = iface_name.to_string();
                                        if !self.is_interface_var_expr(&assign.right[i], locals) {
                                            self.check_iface_satisfaction(&iface_name, &rhs_type_name)?;
                                        }
                                    }
                                }
                                let type_id = self.get_or_create_type_id(&rhs_type_name);
                                let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                self.emit_box_value(rhs_vt, elem_size, type_id, tid_local, data_local, out, locals)?;
                                continue;
                            }
                        }
                        if let Some(&gc_ref_idx) = locals.gc_string_locals.get(&ident.name) {
                            if assign.op == Operator::Assign {
                                out.push(Instruction::LocalSet(gc_ref_idx));
                            } else if assign.op == Operator::AddAssign {
                                let go_string_idx = self.gc_builtin_types.go_string.unwrap();
                                let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                                let gc_vt = Self::gc_ref_val_type(go_string_idx);
                                let ba_vt = Self::gc_ref_val_type(byte_array_idx);

                                let rhs_ref = locals.add_local(&format!("__sadd_rhs_{}", locals.locals.len()), gc_vt);
                                out.push(Instruction::LocalSet(rhs_ref));

                                let lhs_arr = locals.add_local(&format!("__sadd_la_{}", locals.locals.len()), ba_vt);
                                let lhs_len = locals.add_local(&format!("__sadd_ll_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalGet(gc_ref_idx));
                                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                                out.push(Instruction::LocalSet(lhs_arr));
                                out.push(Instruction::LocalGet(gc_ref_idx));
                                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                                out.push(Instruction::LocalSet(lhs_len));

                                let rhs_arr = locals.add_local(&format!("__sadd_ra_{}", locals.locals.len()), ba_vt);
                                let rhs_len = locals.add_local(&format!("__sadd_rl_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalGet(rhs_ref));
                                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
                                out.push(Instruction::LocalSet(rhs_arr));
                                out.push(Instruction::LocalGet(rhs_ref));
                                out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
                                out.push(Instruction::LocalSet(rhs_len));

                                let total = locals.add_local(&format!("__sadd_tot_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalGet(lhs_len));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalSet(total));

                                let new_arr = locals.add_local(&format!("__sadd_na_{}", locals.locals.len()), ba_vt);
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::ArrayNew(byte_array_idx));
                                out.push(Instruction::LocalSet(new_arr));

                                out.push(Instruction::LocalGet(new_arr));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(lhs_arr));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(lhs_len));
                                out.push(Instruction::ArrayCopy { array_type_index_dst: byte_array_idx, array_type_index_src: byte_array_idx });

                                out.push(Instruction::LocalGet(new_arr));
                                out.push(Instruction::LocalGet(lhs_len));
                                out.push(Instruction::LocalGet(rhs_arr));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::ArrayCopy { array_type_index_dst: byte_array_idx, array_type_index_src: byte_array_idx });

                                out.push(Instruction::LocalGet(new_arr));
                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::StructNew(go_string_idx));
                                out.push(Instruction::LocalSet(gc_ref_idx));
                            } else {
                                return Err(Error::TypeError(format!(
                                    "operator {:?} not defined for string (only += is valid for string concatenation)",
                                    assign.op
                                )));
                            }
                            continue;
                        }
                        if let Some(&(ptr_local, len_local)) = locals.string_locals.get(&ident.name) {
                            if assign.op == Operator::Assign {
                                out.push(Instruction::LocalSet(len_local));
                                out.push(Instruction::LocalSet(ptr_local));
                            } else if assign.op == Operator::AddAssign {
                                let rhs_len = locals.add_local(&format!("__sadd_rlen_{}", locals.locals.len()), ValType::I32);
                                let rhs_ptr = locals.add_local(&format!("__sadd_rptr_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalSet(rhs_len));
                                out.push(Instruction::LocalSet(rhs_ptr));

                                let total = locals.add_local(&format!("__sadd_total_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalSet(total));

                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                let new_ptr = locals.add_local(&format!("__sadd_np_{}", locals.locals.len()), ValType::I32);
                                out.push(Instruction::LocalSet(new_ptr));

                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalGet(ptr_local));
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalGet(len_local));
                                out.push(Instruction::I32Add);
                                out.push(Instruction::LocalGet(rhs_ptr));
                                out.push(Instruction::LocalGet(rhs_len));
                                out.push(Instruction::MemoryCopy { dst_mem: 0, src_mem: 0 });

                                out.push(Instruction::LocalGet(new_ptr));
                                out.push(Instruction::LocalSet(ptr_local));
                                out.push(Instruction::LocalGet(total));
                                out.push(Instruction::LocalSet(len_local));
                            } else {
                                return Err(Error::TypeError(format!(
                                    "operator {:?} not defined for string (only += is valid for string concatenation)",
                                    assign.op
                                )));
                            }
                            continue;
                        }
                        if let Some(cap_info) = self.find_or_add_capture(&ident.name) {
                            let env_offset = cap_info.0 as u64;
                            let vt = cap_info.1;
                            match assign.op {
                                Operator::Assign => {
                                    let tmp = locals.add_local(
                                        &format!("__cap_w_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(0)); // env_ptr
                                    out.push(Instruction::LocalGet(tmp));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                }
                                _ => {
                                    let rhs_tmp = locals.add_local(
                                        &format!("__cap_rhs_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(rhs_tmp));
                                    out.push(Instruction::LocalGet(0));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Load(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Load(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Load(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Load(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                    out.push(Instruction::LocalGet(rhs_tmp));
                                    let arith = match assign.op {
                                        Operator::AddAssign => Self::typed_add(vt),
                                        Operator::SubAssign => Self::typed_sub(vt),
                                        Operator::MulAssign => Self::typed_mul(vt),
                                        _ => Self::typed_add(vt),
                                    };
                                    out.push(arith);
                                    let result_tmp = locals.add_local(
                                        &format!("__cap_res_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(result_tmp));
                                    out.push(Instruction::LocalGet(0));
                                    out.push(Instruction::LocalGet(result_tmp));
                                    match vt {
                                        ValType::I64 => out.push(Instruction::I64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F64 => out.push(Instruction::F64Store(MemArg {
                                            offset: env_offset, align: 3, memory_index: 0,
                                        })),
                                        ValType::F32 => out.push(Instruction::F32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                        _ => out.push(Instruction::I32Store(MemArg {
                                            offset: env_offset, align: 2, memory_index: 0,
                                        })),
                                    }
                                }
                            }
                            continue;
                        }
                        if let Some(&(mb_offset, mb_vt)) = locals.memory_backed_vars.get(&ident.name) {
                            if let Some(sf) = &self.current_stack_frame {
                                if let Some(fb) = sf.frame_base_local {
                                    match assign.op {
                                        Operator::Assign => {
                                            let val_tmp = locals.add_local(
                                                &format!("__mb_asgn_{}", locals.locals.len()),
                                                mb_vt,
                                            );
                                            let rhs_vt = if i < assign.right.len() {
                                                self.infer_val_type(&assign.right[i], locals)
                                            } else { mb_vt };
                                            if rhs_vt != mb_vt {
                                                Self::emit_typed_coerce(rhs_vt, mb_vt, out)?;
                                            }
                                            out.push(Instruction::LocalSet(val_tmp));
                                            out.push(Instruction::LocalGet(fb));
                                            if mb_offset > 0 {
                                                out.push(Instruction::I32Const(mb_offset as i32));
                                                out.push(Instruction::I32Add);
                                            }
                                            out.push(Instruction::LocalGet(val_tmp));
                                            let (_, align) = Self::elem_size_and_align(mb_vt);
                                            Self::emit_typed_store(mb_vt, 0, align, out);
                                        }
                                        Operator::AddAssign | Operator::SubAssign |
                                        Operator::MulAssign | Operator::QuoAssign |
                                        Operator::RemAssign => {
                                            let rhs_tmp = locals.add_local(
                                                &format!("__mb_rhs_{}", locals.locals.len()),
                                                mb_vt,
                                            );
                                            let rhs_vt = if i < assign.right.len() {
                                                self.infer_val_type(&assign.right[i], locals)
                                            } else { mb_vt };
                                            if rhs_vt != mb_vt {
                                                Self::emit_typed_coerce(rhs_vt, mb_vt, out)?;
                                            }
                                            out.push(Instruction::LocalSet(rhs_tmp));

                                            out.push(Instruction::LocalGet(fb));
                                            if mb_offset > 0 {
                                                out.push(Instruction::I32Const(mb_offset as i32));
                                                out.push(Instruction::I32Add);
                                            }
                                            let addr_tmp = locals.add_local(
                                                &format!("__mb_addr_{}", locals.locals.len()),
                                                ValType::I32,
                                            );
                                            out.push(Instruction::LocalTee(addr_tmp));

                                            let (_, align) = Self::elem_size_and_align(mb_vt);
                                            Self::emit_typed_load(mb_vt, 0, align, out);
                                            out.push(Instruction::LocalGet(rhs_tmp));
                                            let op_instr = match assign.op {
                                                Operator::AddAssign => Self::typed_add(mb_vt),
                                                Operator::SubAssign => Self::typed_sub(mb_vt),
                                                Operator::MulAssign => Self::typed_mul(mb_vt),
                                                _ => Self::typed_add(mb_vt),
                                            };
                                            out.push(op_instr);

                                            let result_tmp = locals.add_local(
                                                &format!("__mb_res_{}", locals.locals.len()),
                                                mb_vt,
                                            );
                                            out.push(Instruction::LocalSet(result_tmp));
                                            out.push(Instruction::LocalGet(addr_tmp));
                                            out.push(Instruction::LocalGet(result_tmp));
                                            Self::emit_typed_store(mb_vt, 0, align, out);
                                        }
                                        _ => {}
                                    }
                                    continue;
                                }
                            }
                        }
                        if let Some(idx) = locals.find(&ident.name) {
                            let vt = locals
                                .find_type(&ident.name)
                                .unwrap_or(ValType::I64);
                            match assign.op {
                                Operator::Assign => {
                                    let gc_copy = if i < assign.right.len() {
                                        if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                            self.get_gc_copy_info(&rhs_ident.name, locals)
                                        } else { None }
                                    } else { None };
                                    if let Some((gc_idx, gc_sd)) = gc_copy {
                                        Self::emit_gc_value_deep_copy(gc_idx, &gc_sd, out, locals);
                                    } else {
                                        let needs_deep_copy = if i < assign.right.len() {
                                            if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                                self.get_value_copy_size(&rhs_ident.name, locals).is_some()
                                            } else { false }
                                        } else { false };
                                        if needs_deep_copy {
                                            if let ast::Expression::Ident(rhs_ident) = &assign.right[i] {
                                                let size = self.get_value_copy_size(&rhs_ident.name, locals).unwrap();
                                                self.emit_value_deep_copy(size, out, locals)?;
                                            }
                                        } else {
                                            if i < assign.right.len() {
                                                let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                                if rhs_vt != vt
                                                    && !matches!(rhs_vt, ValType::Ref(_))
                                                    && !matches!(vt, ValType::Ref(_))
                                                {
                                                    Self::emit_typed_coerce(rhs_vt, vt, out)?;
                                                }
                                            }
                                        }
                                    }
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::AddAssign => {
                                    if i < assign.right.len() {
                                        let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                        if rhs_vt != vt { Self::emit_typed_coerce(rhs_vt, vt, out)?; }
                                    }
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_add(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::SubAssign => {
                                    if i < assign.right.len() {
                                        let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                        if rhs_vt != vt { Self::emit_typed_coerce(rhs_vt, vt, out)?; }
                                    }
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_sub(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::MulAssign => {
                                    if i < assign.right.len() {
                                        let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                        if rhs_vt != vt { Self::emit_typed_coerce(rhs_vt, vt, out)?; }
                                    }
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_mul(vt));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::QuoAssign => {
                                    let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                    if i < assign.right.len() {
                                        let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                        if rhs_vt != vt { Self::emit_typed_coerce(rhs_vt, vt, out)?; }
                                    }
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    out.push(Self::typed_div(vt, is_unsigned));
                                    out.push(Instruction::LocalSet(idx));
                                }
                                Operator::RemAssign
                                | Operator::AndAssign
                                | Operator::OrAssign
                                | Operator::XorAssign
                                | Operator::ShlAssign
                                | Operator::ShrAssign
                                | Operator::AndNotAssign => {
                                    let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                    if i < assign.right.len() {
                                        let rhs_vt = self.infer_val_type(&assign.right[i], locals);
                                        if rhs_vt != vt { Self::emit_typed_coerce(rhs_vt, vt, out)?; }
                                    }
                                    let tmp = locals.add_local(
                                        &format!("__ca_tmp_{}", locals.locals.len()),
                                        vt,
                                    );
                                    out.push(Instruction::LocalSet(tmp));
                                    out.push(Instruction::LocalGet(idx));
                                    out.push(Instruction::LocalGet(tmp));
                                    self.emit_compound_op(&assign.op, vt, is_unsigned, out)?;
                                    out.push(Instruction::LocalSet(idx));
                                }
                                _ => {
                                    out.push(Instruction::LocalSet(idx));
                                }
                            }
                        } else if let Some(&(global_idx, vt)) = self.resolve_global_var(&ident.name) {
                            if matches!(vt, ValType::Ref(_)) {
                                out.push(Instruction::GlobalSet(global_idx));
                            } else {
                            let resolved_name = self.resolve_global_var_name(&ident.name);
                            let len_key = format!("{}_1", resolved_name);
                            if let Some(&(len_global_idx, _)) = self.global_vars.get(&len_key) {
                                let len_tmp = locals.add_local(
                                    &format!("__gsa_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                let ptr_tmp = locals.add_local(
                                    &format!("__gsa_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(len_tmp));
                                out.push(Instruction::LocalSet(ptr_tmp));
                                out.push(Instruction::LocalGet(ptr_tmp));
                                out.push(Instruction::GlobalSet(global_idx));
                                out.push(Instruction::LocalGet(len_tmp));
                                out.push(Instruction::GlobalSet(len_global_idx));
                            } else {
                                match assign.op {
                                    Operator::Assign => {
                                        out.push(Instruction::GlobalSet(global_idx));
                                    }
                                    _ => {
                                        let is_unsigned = locals.unsigned_vars.contains(&ident.name);
                                        let tmp = locals.add_local(
                                            &format!("__gca_tmp_{}", locals.locals.len()),
                                            vt,
                                        );
                                        out.push(Instruction::LocalSet(tmp));
                                        out.push(Instruction::GlobalGet(global_idx));
                                        out.push(Instruction::LocalGet(tmp));
                                        self.emit_compound_op(&assign.op, vt, is_unsigned, out)?;
                                        out.push(Instruction::GlobalSet(global_idx));
                                    }
                                }
                            }
                            }
                        }
                    }
                    ast::Expression::Index(idx_expr) => {
                        // Check if this is a map index assignment
                        if let Some(left_expr) = idx_expr.left.as_deref() {
                            if let ast::Expression::Ident(map_ident) = left_expr {
                                if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                                    let mti_is_string_val = locals.map_types.get(&map_ident.name)
                                        .map_or(false, |mti| mti.is_string_val);
                                    let mut rhs_vt = if i < assign.right.len() {
                                        self.infer_val_type(&assign.right[i], locals)
                                    } else {
                                        ValType::I64
                                    };
                                    if mti_is_string_val {
                                        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                            self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                                            rhs_vt = ValType::I32;
                                        }
                                    }
                                    let val_len_tmp = if mti_is_string_val {
                                        let vl = locals.add_local(
                                            &format!("__midx_rlen_{}", locals.locals.len()),
                                            ValType::I32,
                                        );
                                        out.push(Instruction::LocalSet(vl));
                                        Some(vl)
                                    } else {
                                        None
                                    };
                                    let rhs_tmp = locals.add_local(
                                        &format!("__midx_rhs_{}", locals.locals.len()),
                                        if mti_is_string_val { ValType::I32 } else { rhs_vt },
                                    );
                                    out.push(Instruction::LocalSet(rhs_tmp));
                                    let map_name = map_ident.name.clone();
                                    let key_expr = idx_expr.index.clone();
                                    self.compile_map_set(&map_name, &key_expr, rhs_tmp, val_len_tmp, rhs_vt, out, locals)?;
                                    continue;
                                }
                            }
                        }

                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__idx_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        let (elem_vt, align) =
                            self.compile_index_store_addr(idx_expr, out, locals)?;

                        let addr_tmp = locals.add_local(
                            &format!("__idx_addr_{}", locals.locals.len()),
                            ValType::I32,
                        );
                        out.push(Instruction::LocalSet(addr_tmp));

                        match assign.op {
                            Operator::Assign => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out)?;
                                Self::emit_typed_store(elem_vt, 0, align, out);
                            }
                            _ => {
                                out.push(Instruction::LocalGet(addr_tmp));
                                Self::emit_typed_load(elem_vt, 0, align, out);

                                out.push(Instruction::LocalGet(rhs_tmp));
                                Self::emit_typed_coerce(rhs_vt, elem_vt, out)?;

                                self.emit_compound_op(&assign.op, elem_vt, false, out)?;

                                let result_tmp = locals.add_local(
                                    &format!("__idx_res_{}", locals.locals.len()),
                                    elem_vt,
                                );
                                out.push(Instruction::LocalSet(result_tmp));
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(result_tmp));
                                Self::emit_typed_store(elem_vt, 0, align, out);
                            }
                        }
                    }
                    ast::Expression::Selector(sel) => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__sel_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        let struct_type_name = self.infer_struct_type_from_expr(sel.x.as_ref(), locals);
                        let gc_info = struct_type_name.as_ref().and_then(|name| {
                            let sd = self.struct_defs.get(name)?;
                            let gc_idx = sd.gc_type_idx?;
                            let field = sd.find_field(&sel.sel.name)?;
                            Some((gc_idx, field.field_index, field.wasm_type.to_val_type()))
                        });

                        if let Some((gc_type_idx, field_index, field_vt)) = gc_info {
                            self.compile_expression(&sel.x, out, locals)?;
                            let ref_tmp = locals.add_local(
                                &format!("__sel_gc_ref_{}", locals.locals.len()),
                                Self::gc_ref_val_type(gc_type_idx),
                            );
                            out.push(Instruction::LocalSet(ref_tmp));

                            match assign.op {
                                Operator::Assign => {
                                    out.push(Instruction::LocalGet(ref_tmp));
                                    out.push(Instruction::LocalGet(rhs_tmp));
                                    Self::emit_typed_coerce(rhs_vt, field_vt, out)?;
                                    out.push(Instruction::StructSet {
                                        struct_type_index: gc_type_idx,
                                        field_index,
                                    });
                                }
                                _ => {
                                    out.push(Instruction::LocalGet(ref_tmp));
                                    out.push(Instruction::StructGet {
                                        struct_type_index: gc_type_idx,
                                        field_index,
                                    });
                                    out.push(Instruction::LocalGet(rhs_tmp));
                                    Self::emit_typed_coerce(rhs_vt, field_vt, out)?;
                                    self.emit_compound_op(&assign.op, field_vt, false, out)?;
                                    let result_tmp = locals.add_local(
                                        &format!("__sel_res_{}", locals.locals.len()),
                                        field_vt,
                                    );
                                    out.push(Instruction::LocalSet(result_tmp));
                                    out.push(Instruction::LocalGet(ref_tmp));
                                    out.push(Instruction::LocalGet(result_tmp));
                                    out.push(Instruction::StructSet {
                                        struct_type_index: gc_type_idx,
                                        field_index,
                                    });
                                }
                            }
                        } else {
                            let field_tag = struct_type_name.as_ref().and_then(|name| {
                                self.struct_defs.get(name)?
                                    .find_field(&sel.sel.name)?
                                    .go_type_tag.as_deref()
                                    .map(|s| s.to_string())
                            });

                            if field_tag.as_deref() == Some("__string")
                                && self.gc_builtin_types.go_string.is_some()
                                && matches!(rhs_vt, ValType::Ref(_))
                            {
                                let go_string_idx = self.gc_builtin_types.go_string.unwrap();
                                let (offset, _field_vt) =
                                    self.compile_selector_store_addr(sel, out, locals)?;
                                let addr_tmp = locals.add_local(
                                    &format!("__sel_addr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(addr_tmp));

                                out.push(Instruction::LocalGet(rhs_tmp));
                                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                                let str_len = locals.add_local(
                                    &format!("__sel_str_len_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                let str_ptr = locals.add_local(
                                    &format!("__sel_str_ptr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(str_len));
                                out.push(Instruction::LocalSet(str_ptr));

                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(str_ptr));
                                out.push(Instruction::I32Store(MemArg {
                                    offset,
                                    align: 2,
                                    memory_index: 0,
                                }));
                                out.push(Instruction::LocalGet(addr_tmp));
                                out.push(Instruction::LocalGet(str_len));
                                out.push(Instruction::I32Store(MemArg {
                                    offset: offset + 4,
                                    align: 2,
                                    memory_index: 0,
                                }));
                            } else {
                                let (offset, field_vt) =
                                    self.compile_selector_store_addr(sel, out, locals)?;
                                let (_, align) = Self::elem_size_and_align(field_vt);

                                let addr_tmp = locals.add_local(
                                    &format!("__sel_addr_{}", locals.locals.len()),
                                    ValType::I32,
                                );
                                out.push(Instruction::LocalSet(addr_tmp));

                                match assign.op {
                                    Operator::Assign => {
                                        out.push(Instruction::LocalGet(addr_tmp));
                                        out.push(Instruction::LocalGet(rhs_tmp));
                                        Self::emit_typed_coerce(rhs_vt, field_vt, out)?;
                                        Self::emit_typed_store(field_vt, offset, align, out);
                                    }
                                    _ => {
                                        out.push(Instruction::LocalGet(addr_tmp));
                                        Self::emit_typed_load(field_vt, offset, align, out);

                                        out.push(Instruction::LocalGet(rhs_tmp));
                                        Self::emit_typed_coerce(rhs_vt, field_vt, out)?;

                                        self.emit_compound_op(&assign.op, field_vt, false, out)?;

                                        let result_tmp = locals.add_local(
                                            &format!("__sel_res_{}", locals.locals.len()),
                                            field_vt,
                                        );
                                        out.push(Instruction::LocalSet(result_tmp));
                                        out.push(Instruction::LocalGet(addr_tmp));
                                        out.push(Instruction::LocalGet(result_tmp));
                                        Self::emit_typed_store(field_vt, offset, align, out);
                                    }
                                }
                            }
                        }
                    }
                    ast::Expression::Star(star) => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__star_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        self.compile_expression(&star.right, out, locals)?;
                        let deref_vt = self.infer_deref_type(&star.right, locals);
                        let (_, align) = Self::elem_size_and_align(deref_vt);

                        out.push(Instruction::LocalGet(rhs_tmp));
                        Self::emit_typed_coerce(rhs_vt, deref_vt, out)?;
                        Self::emit_typed_store(deref_vt, 0, align, out);
                    }
                    ast::Expression::Operation(op) if op.op == Operator::Star && op.y.is_none() => {
                        let rhs_vt = if i < assign.right.len() {
                            self.infer_val_type(&assign.right[i], locals)
                        } else {
                            ValType::I64
                        };
                        let rhs_tmp = locals.add_local(
                            &format!("__deref_rhs_{}", locals.locals.len()),
                            rhs_vt,
                        );
                        out.push(Instruction::LocalSet(rhs_tmp));

                        self.compile_expression(&op.x, out, locals)?;
                        let deref_vt = self.infer_deref_type(&op.x, locals);
                        let (_, align) = Self::elem_size_and_align(deref_vt);

                        out.push(Instruction::LocalGet(rhs_tmp));
                        Self::emit_typed_coerce(rhs_vt, deref_vt, out)?;
                        Self::emit_typed_store(deref_vt, 0, align, out);
                    }
                    _ => {
                        return Err(Error::InternalError(
                            "assignment to unsupported target expression"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn typed_add(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Add,
            ValType::F32 => Instruction::F32Add,
            ValType::F64 => Instruction::F64Add,
            _ => Instruction::I64Add,
        }
    }

    pub(crate) fn typed_sub(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Sub,
            ValType::F32 => Instruction::F32Sub,
            ValType::F64 => Instruction::F64Sub,
            _ => Instruction::I64Sub,
        }
    }

    pub(crate) fn typed_mul(vt: ValType) -> Instruction<'static> {
        match vt {
            ValType::I32 => Instruction::I32Mul,
            ValType::F32 => Instruction::F32Mul,
            ValType::F64 => Instruction::F64Mul,
            _ => Instruction::I64Mul,
        }
    }

    pub(crate) fn typed_div(vt: ValType, is_unsigned: bool) -> Instruction<'static> {
        match vt {
            ValType::I32 => if is_unsigned { Instruction::I32DivU } else { Instruction::I32DivS },
            ValType::F32 => Instruction::F32Div,
            ValType::F64 => Instruction::F64Div,
            _ => if is_unsigned { Instruction::I64DivU } else { Instruction::I64DivS },
        }
    }

    pub(crate) fn compile_if(
        &mut self,
        if_stmt: &ast::IfStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        let has_init = if_stmt.init.is_some();
        if has_init {
            locals.push_scope();
            self.symbols.enter_scope();
        }

        if let Some(init) = &if_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        self.compile_expression(&if_stmt.cond, out, locals)?;

        out.push(Instruction::If(BlockType::Empty));
        if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
            *depth += 1;
        }
        self.compile_block(&if_stmt.body, out, locals, result_types)?;

        if let Some(else_) = &if_stmt.else_ {
            out.push(Instruction::Else);
            self.compile_statement(else_, out, locals, result_types)?;
        }

        if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
            *depth -= 1;
        }
        out.push(Instruction::End);

        if has_init {
            self.symbols.leave_scope();
            locals.pop_scope();
        }
        Ok(())
    }

    pub(crate) fn compile_for(
        &mut self,
        for_stmt: &ast::ForStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_for_labeled(for_stmt, out, locals, result_types, None)
    }

    pub(crate) fn compile_for_labeled(
        &mut self,
        for_stmt: &ast::ForStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let has_init = for_stmt.init.is_some();
        if has_init {
            locals.push_scope();
            self.symbols.enter_scope();
        }

        if let Some(init) = &for_stmt.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        let has_post = for_stmt.post.is_some();

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, has_post, true));

        if let Some(cond) = &for_stmt.cond {
            if let ast::Statement::Expr(expr_stmt) = cond.as_ref() {
                self.compile_expression(&expr_stmt.expr, out, locals)?;
                out.push(Instruction::I32Eqz);
                out.push(Instruction::BrIf(1));
            } else {
                return Err(Error::InternalError(format!(
                    "unsupported for-loop condition statement: {:?}",
                    cond
                )));
            }
        }

        if has_post {
            out.push(Instruction::Block(BlockType::Empty));
        }

        self.compile_block(&for_stmt.body, out, locals, result_types)?;

        if has_post {
            out.push(Instruction::End);
        }

        if let Some(post) = &for_stmt.post {
            self.compile_statement(post, out, locals, result_types)?;
        }

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        if has_init {
            self.symbols.leave_scope();
            locals.pop_scope();
        }

        Ok(())
    }

    pub(crate) fn compile_range(
        &mut self,
        range: &ast::RangeStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_range_labeled(range, out, locals, result_types, None)
    }

    pub(crate) fn compile_range_labeled(
        &mut self,
        range: &ast::RangeStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        // Check for range-over-function iterator pattern (Go 1.22+)
        if let ast::Expression::Ident(fn_ident) = &range.expr {
            let qualified = self.qualify_pkg_name(&fn_ident.name);
            if let Some(info) = self.iter_func_info.get(&qualified).cloned() {
                return self.compile_range_over_func(range, &info, out, locals, label);
            }
            if let Some(info) = self.iter_func_info.get(&fn_ident.name).cloned() {
                return self.compile_range_over_func(range, &info, out, locals, label);
            }
        }

        // Check for map range
        if let ast::Expression::Ident(map_ident) = &range.expr {
            if locals.get_var_struct_type(&map_ident.name) == Some("__map") {
                return self.compile_range_map(range, &map_ident.name.clone(), out, locals, result_types, label);
            }
        }
        if let ast::Expression::Selector(sel) = &range.expr {
            if self.is_selector_map_field(sel, locals) {
                if let Some(parent_type) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                    if let Some(mti) = self.struct_field_map_types.get(&(parent_type, sel.sel.name.clone())).cloned() {
                        self.compile_expression(&range.expr, out, locals)?;
                        let tmp_name = format!("__range_map_tmp_{}", locals.locals.len());
                        let tmp_local = locals.add_local(&tmp_name, ValType::I32);
                        out.push(Instruction::LocalSet(tmp_local));
                        locals.set_var_struct_type(&tmp_name, "__map");
                        locals.map_types.insert(tmp_name.clone(), mti);
                        return self.compile_range_map(range, &tmp_name, out, locals, result_types, label);
                    }
                }
            }
        }

        let idx_local = locals.add_local("__range_idx", ValType::I32);
        let len_local = locals.add_local("__range_len", ValType::I32);
        let base_ptr_local = locals.add_local("__range_base", ValType::I32);

        // Check if the range expression is a slice header variable
        let is_slice_header = if let ast::Expression::Ident(ident) = &range.expr {
            locals.get_var_struct_type(&ident.name) == Some("__slice")
        } else if let ast::Expression::Selector(sel) = &range.expr {
            self.is_selector_slice_field(sel, locals)
        } else if let ast::Expression::Call(_) = &range.expr {
            let go_types = self.call_return_go_types(&range.expr, locals);
            go_types.len() == 1 && go_types[0].starts_with("[]")
        } else {
            false
        };

        // Check if range is over an array variable
        let array_info = if let ast::Expression::Ident(ident) = &range.expr {
            locals.array_info.get(&ident.name).copied()
        } else {
            None
        };

        let is_string_range_early = self.is_string_expr(&range.expr, locals);
        let is_gc_string_range = is_string_range_early && self.gc_builtin_types.go_string.is_some();

        let gc_range_byte_arr: Option<u32> = if is_gc_string_range {
            let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
            Some(locals.add_local("__range_gc_ba", Self::gc_ref_val_type(byte_array_idx)))
        } else {
            None
        };

        self.compile_expression(&range.expr, out, locals)?;

        if let Some((_arr_elem_vt, arr_len, ..)) = array_info {
            // Array variable: pushes data pointer (1 value), length is compile-time
            out.push(Instruction::LocalSet(base_ptr_local));
            out.push(Instruction::I32Const(arr_len as i32));
            out.push(Instruction::LocalSet(len_local));
        } else if is_slice_header {
            // Slice header pointer: load data_ptr and len from header
            let hdr_tmp = locals.add_local(
                &format!("__range_hdr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(hdr_tmp));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg {
                offset: 4,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalSet(len_local));
            out.push(Instruction::LocalGet(hdr_tmp));
            out.push(Instruction::I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalSet(base_ptr_local));
        } else if is_gc_string_range {
            let go_string_idx = self.gc_builtin_types.go_string.unwrap();
            let gc_vt = Self::gc_ref_val_type(go_string_idx);
            let gc_str_tmp = locals.add_local("__range_gc_str", gc_vt);
            out.push(Instruction::LocalSet(gc_str_tmp));
            out.push(Instruction::LocalGet(gc_str_tmp));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 0 });
            out.push(Instruction::LocalSet(gc_range_byte_arr.unwrap()));
            out.push(Instruction::LocalGet(gc_str_tmp));
            out.push(Instruction::StructGet { struct_type_index: go_string_idx, field_index: 1 });
            out.push(Instruction::LocalSet(len_local));
        } else if is_string_range_early {
            out.push(Instruction::LocalSet(len_local));
            out.push(Instruction::LocalSet(base_ptr_local));
        } else {
            // Check how many values the expression pushed
            let expr_count = self.expression_result_count(&range.expr, Some(locals));
            if expr_count >= 3 {
                // Slice: (ptr, len, cap) -> store cap, len, ptr
                out.push(Instruction::Drop); // cap
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::LocalSet(base_ptr_local));
            } else if expr_count == 2 {
                // (ptr, len) or (something, something)
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::LocalSet(base_ptr_local));
            } else {
                // Single value: assume it's a count (integer range)
                // Wrap to i32 if the expression pushed i64
                let range_vt = self.infer_val_type(&range.expr, locals);
                if range_vt == ValType::I64 {
                    out.push(Instruction::I32WrapI64);
                }
                // Clamp negative values to 0 so the loop runs zero iterations
                let raw_count = locals.add_local(
                    &format!("__range_raw_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(raw_count));
                out.push(Instruction::LocalGet(raw_count));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalGet(raw_count));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::I32GtS);
                out.push(Instruction::Select);
                out.push(Instruction::LocalSet(len_local));
                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(base_ptr_local));
            }
        }

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(idx_local));

        out.push(Instruction::Block(BlockType::Empty));
        out.push(Instruction::Loop(BlockType::Empty));

        self.loop_depth.push((label, 0, false, true));

        out.push(Instruction::LocalGet(idx_local));
        out.push(Instruction::LocalGet(len_local));
        out.push(Instruction::I32GeU);
        out.push(Instruction::BrIf(1));

        // For string ranges, decode UTF-8 runes and compute rune width
        let mut rune_width_local: Option<u32> = None;
        let mut rune_val_local: Option<u32> = None;
        if is_string_range_early {
            let byte0 = locals.add_local("__utf8_byte0", ValType::I32);
            let addr = locals.add_local("__utf8_addr", ValType::I32);
            let rw = locals.add_local("__rune_width", ValType::I32);
            let rv = locals.add_local("__rune_val", ValType::I32);
            rune_width_local = Some(rw);
            rune_val_local = Some(rv);

            let mem0 = MemArg { offset: 0, align: 0, memory_index: 0 };
            let gc_ba = gc_range_byte_arr;
            let ba_type_idx = self.gc_builtin_types.byte_array;

            if gc_ba.is_some() {
                out.push(Instruction::LocalGet(idx_local));
                out.push(Instruction::LocalSet(addr));
            } else {
                out.push(Instruction::LocalGet(base_ptr_local));
                out.push(Instruction::LocalGet(idx_local));
                out.push(Instruction::I32Add);
                out.push(Instruction::LocalSet(addr));
            }

            if let (Some(ba), Some(ba_idx)) = (gc_ba, ba_type_idx) {
                out.push(Instruction::LocalGet(ba));
                out.push(Instruction::LocalGet(addr));
                out.push(Instruction::ArrayGetU(ba_idx));
            } else {
                out.push(Instruction::LocalGet(addr));
                out.push(Instruction::I32Load8U(mem0));
            }
            out.push(Instruction::LocalSet(byte0));

            // Default: width=1, rune=byte0
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(rw));
            out.push(Instruction::LocalGet(byte0));
            out.push(Instruction::LocalSet(rv));

            // Helper macro-style: emit byte read at addr+offset
            macro_rules! emit_byte_read {
                ($out:expr, $gc_ba:expr, $ba_type_idx:expr, $addr:expr, $offset:expr) => {
                    if let (Some(ba), Some(ba_idx)) = ($gc_ba, $ba_type_idx) {
                        $out.push(Instruction::LocalGet(ba));
                        $out.push(Instruction::LocalGet($addr));
                        if $offset > 0u32 {
                            $out.push(Instruction::I32Const($offset as i32));
                            $out.push(Instruction::I32Add);
                        }
                        $out.push(Instruction::ArrayGetU(ba_idx));
                    } else {
                        $out.push(Instruction::LocalGet($addr));
                        $out.push(Instruction::I32Load8U(MemArg { offset: $offset as u64, align: 0, memory_index: 0 }));
                    }
                };
            }

            // if byte0 >= 0x80 (multi-byte)
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
                    // Boundary check: need idx + 2 <= len
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::I32Const(2));
                    out.push(Instruction::I32Add);
                    out.push(Instruction::LocalGet(len_local));
                    out.push(Instruction::I32LeU);
                    out.push(Instruction::If(BlockType::Empty));
                    {
                        out.push(Instruction::I32Const(2));
                        out.push(Instruction::LocalSet(rw));
                        out.push(Instruction::LocalGet(byte0));
                        out.push(Instruction::I32Const(0x1F));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Const(6));
                        out.push(Instruction::I32Shl);
                        emit_byte_read!(out, gc_ba, ba_type_idx, addr, 1u32);
                        out.push(Instruction::I32Const(0x3F));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Or);
                        out.push(Instruction::LocalSet(rv));
                    }
                    out.push(Instruction::Else);
                    {
                        out.push(Instruction::I32Const(0xFFFD));
                        out.push(Instruction::LocalSet(rv));
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
                        // Boundary check: need idx + 3 <= len
                        out.push(Instruction::LocalGet(idx_local));
                        out.push(Instruction::I32Const(3));
                        out.push(Instruction::I32Add);
                        out.push(Instruction::LocalGet(len_local));
                        out.push(Instruction::I32LeU);
                        out.push(Instruction::If(BlockType::Empty));
                        {
                            out.push(Instruction::I32Const(3));
                            out.push(Instruction::LocalSet(rw));
                            out.push(Instruction::LocalGet(byte0));
                            out.push(Instruction::I32Const(0x0F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Const(12));
                            out.push(Instruction::I32Shl);
                            emit_byte_read!(out, gc_ba, ba_type_idx, addr, 1u32);
                            out.push(Instruction::I32Const(0x3F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Const(6));
                            out.push(Instruction::I32Shl);
                            out.push(Instruction::I32Or);
                            emit_byte_read!(out, gc_ba, ba_type_idx, addr, 2u32);
                            out.push(Instruction::I32Const(0x3F));
                            out.push(Instruction::I32And);
                            out.push(Instruction::I32Or);
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::Else);
                        {
                            out.push(Instruction::I32Const(0xFFFD));
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::End);
                    }
                    out.push(Instruction::Else);
                    {
                        // 4-byte: (byte0 & 0xF8) == 0xF0
                        out.push(Instruction::LocalGet(byte0));
                        out.push(Instruction::I32Const(0xF8));
                        out.push(Instruction::I32And);
                        out.push(Instruction::I32Const(0xF0));
                        out.push(Instruction::I32Eq);
                        out.push(Instruction::If(BlockType::Empty));
                        {
                            // Boundary check: need idx + 4 <= len
                            out.push(Instruction::LocalGet(idx_local));
                            out.push(Instruction::I32Const(4));
                            out.push(Instruction::I32Add);
                            out.push(Instruction::LocalGet(len_local));
                            out.push(Instruction::I32LeU);
                            out.push(Instruction::If(BlockType::Empty));
                            {
                                out.push(Instruction::I32Const(4));
                                out.push(Instruction::LocalSet(rw));
                                out.push(Instruction::LocalGet(byte0));
                                out.push(Instruction::I32Const(0x07));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(18));
                                out.push(Instruction::I32Shl);
                                emit_byte_read!(out, gc_ba, ba_type_idx, addr, 1u32);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(12));
                                out.push(Instruction::I32Shl);
                                out.push(Instruction::I32Or);
                                emit_byte_read!(out, gc_ba, ba_type_idx, addr, 2u32);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Const(6));
                                out.push(Instruction::I32Shl);
                                out.push(Instruction::I32Or);
                                emit_byte_read!(out, gc_ba, ba_type_idx, addr, 3u32);
                                out.push(Instruction::I32Const(0x3F));
                                out.push(Instruction::I32And);
                                out.push(Instruction::I32Or);
                                out.push(Instruction::LocalSet(rv));
                            }
                            out.push(Instruction::Else);
                            {
                                out.push(Instruction::I32Const(0xFFFD));
                                out.push(Instruction::LocalSet(rv));
                            }
                            out.push(Instruction::End);
                        }
                        out.push(Instruction::Else);
                        {
                            // Invalid leading byte: U+FFFD replacement, width=1
                            out.push(Instruction::I32Const(1));
                            out.push(Instruction::LocalSet(rw));
                            out.push(Instruction::I32Const(0xFFFD));
                            out.push(Instruction::LocalSet(rv));
                        }
                        out.push(Instruction::End); // end 4-byte check
                    }
                    out.push(Instruction::End); // end 3-byte check
                }
                out.push(Instruction::End); // end 2-byte check
            }
            out.push(Instruction::End); // end multi-byte check
        }

        if let Some(key) = &range.key {
            if let ast::Expression::Ident(ident) = key {
                if ident.name != "_" {
                    let key_local = if range
                        .op
                        .as_ref()
                        .map_or(false, |(_, op)| *op == Operator::Define)
                    {
                        locals.add_local(&ident.name, ValType::I64)
                    } else {
                        locals
                            .find(&ident.name)
                            .unwrap_or_else(|| locals.add_local(&ident.name, ValType::I64))
                    };
                    out.push(Instruction::LocalGet(idx_local));
                    out.push(Instruction::I64ExtendI32S);
                    out.push(Instruction::LocalSet(key_local));
                }
            }
        }

        if let Some(value) = &range.value {
            if let ast::Expression::Ident(ident) = value {
                if ident.name != "_" {
                    if is_string_range_early {
                        // String range: value is the decoded rune
                        let value_local = if range.op.as_ref().map_or(false, |(_, op)| *op == Operator::Define) {
                            locals.add_local(&ident.name, ValType::I32)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| locals.add_local(&ident.name, ValType::I32))
                        };
                        if let Some(rv) = rune_val_local {
                            out.push(Instruction::LocalGet(rv));
                            out.push(Instruction::LocalSet(value_local));
                        }
                    } else {
                        let elem_vt = if let ast::Expression::Ident(range_ident) = &range.expr {
                            if let Some(&(arr_evtype, _, ..)) = locals.array_info.get(&range_ident.name) {
                                arr_evtype
                            } else {
                                locals
                                    .slice_elem_types
                                    .get(&range_ident.name)
                                    .copied()
                                    .unwrap_or(ValType::I64)
                            }
                        } else {
                            ValType::I64
                        };

                        let value_local = if range
                            .op
                            .as_ref()
                            .map_or(false, |(_, op)| *op == Operator::Define)
                        {
                            locals.add_local(&ident.name, elem_vt)
                        } else {
                            locals.find(&ident.name).unwrap_or_else(|| {
                                locals.add_local(&ident.name, elem_vt)
                            })
                        };

                        let (elem_size, align) = match elem_vt {
                            ValType::I32 | ValType::F32 => (4i32, 2u32),
                            _ => (8i32, 3u32),
                        };

                        out.push(Instruction::LocalGet(base_ptr_local));
                        out.push(Instruction::LocalGet(idx_local));
                        out.push(Instruction::I32Const(elem_size));
                        out.push(Instruction::I32Mul);
                        out.push(Instruction::I32Add);
                        match elem_vt {
                            ValType::I32 => out.push(Instruction::I32Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            ValType::F32 => out.push(Instruction::F32Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            ValType::F64 => out.push(Instruction::F64Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                            _ => out.push(Instruction::I64Load(MemArg {
                                offset: 0,
                                align,
                                memory_index: 0,
                            })),
                        }
                        out.push(Instruction::LocalSet(value_local));
                    }
                }
            }
        }

        // Increment index BEFORE body so that `continue` (Br(0) to Loop start)
        // doesn't skip the increment and cause an infinite loop.
        if let Some(rw) = rune_width_local {
            // String range: advance by rune byte-width
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::LocalGet(rw));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
        } else {
            out.push(Instruction::LocalGet(idx_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::I32Add);
            out.push(Instruction::LocalSet(idx_local));
        }

        self.compile_block(&range.body, out, locals, result_types)?;

        out.push(Instruction::Br(0));
        out.push(Instruction::End);
        out.push(Instruction::End);

        self.loop_depth.pop();

        Ok(())
    }

    pub(crate) fn compile_block_stmts(
        &mut self,
        stmts: &[ast::Statement],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        for stmt in stmts {
            self.compile_statement(stmt, out, locals, result_types)?;
        }
        Ok(())
    }

    pub(crate) fn compile_switch_labeled(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        self.compile_switch_inner(switch, out, locals, result_types, label)
    }

    pub(crate) fn compile_switch(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        self.compile_switch_inner(switch, out, locals, result_types, None)
    }

    pub(crate) fn compile_switch_inner(
        &mut self,
        switch: &ast::SwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        let has_init = switch.init.is_some();
        if has_init {
            locals.push_scope();
            self.symbols.enter_scope();
        }

        if let Some(init) = &switch.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        let is_string_tag = switch.tag.as_ref().map_or(false, |t| self.is_string_expr(t, locals));

        let tag_str_ptr: Option<u32>;
        let tag_str_len: Option<u32>;

        let (tag_local, tag_vt) = if let Some(tag) = &switch.tag {
            if is_string_tag {
                if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                    let ref_l = locals.add_local("__switch_tag_ref", Self::gc_ref_val_type(go_string_idx));
                    self.compile_expression(tag, out, locals)?;
                    out.push(Instruction::LocalSet(ref_l));
                    tag_str_ptr = Some(ref_l);
                    tag_str_len = Some(ref_l);
                    (None, ValType::I32)
                } else {
                    let ptr_l = locals.add_local("__switch_tag_ptr", ValType::I32);
                    let len_l = locals.add_local("__switch_tag_len", ValType::I32);
                    self.compile_expression(tag, out, locals)?;
                    out.push(Instruction::LocalSet(len_l));
                    out.push(Instruction::LocalSet(ptr_l));
                    tag_str_ptr = Some(ptr_l);
                    tag_str_len = Some(len_l);
                    (None, ValType::I32)
                }
            } else {
                tag_str_ptr = None;
                tag_str_len = None;
                let vt = self.infer_val_type(tag, locals);
                let local = locals.add_local("__switch_tag", vt);
                self.compile_expression(tag, out, locals)?;
                out.push(Instruction::LocalSet(local));
                (Some(local), vt)
            }
        } else {
            tag_str_ptr = None;
            tag_str_len = None;
            (None, ValType::I32)
        };

        let mut non_default_cases: Vec<&ast::CaseClause> = Vec::new();
        let mut default_case: Option<&ast::CaseClause> = None;

        for case in &switch.block.body {
            if case.tok == Keyword::Default {
                default_case = Some(case);
            } else {
                non_default_cases.push(case);
            }
        }

        // Validate fallthrough: cannot appear in the last case clause when there is no
        // subsequent clause to fall into.
        let all_clauses = &switch.block.body;
        if let Some(last_clause) = all_clauses.last() {
            if last_clause.body.iter().any(|s| Self::is_fallthrough_stmt(s)) {
                return Err(Error::SyntaxError(
                    "cannot fallthrough final case of switch".to_string(),
                ));
            }
        }

        if non_default_cases.is_empty() {
            if let Some(def) = default_case {
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
            }
            if has_init {
                self.symbols.leave_scope();
                locals.pop_scope();
            }
            return Ok(());
        }

        let eq_instr = match tag_vt {
            ValType::I32 => Instruction::I32Eq,
            ValType::F32 => Instruction::F32Eq,
            ValType::F64 => Instruction::F64Eq,
            _ => Instruction::I64Eq,
        };

        let num_cases = non_default_cases.len();

        let has_fallthrough = non_default_cases.iter().any(|c| {
            c.body.iter().any(|s| Self::is_fallthrough_stmt(s))
        });

        out.push(Instruction::Block(BlockType::Empty));
        self.loop_depth.push((label, 0, false, false));

        if has_fallthrough {
            let matched = locals.add_local("__sw_matched", ValType::I32);
            let ft = locals.add_local("__sw_ft", ValType::I32);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(matched));
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(ft));

            for case in non_default_cases.iter() {
                let mut first = true;
                for expr in &case.list {
                    if is_string_tag {
                        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                            let cr = locals.add_local(
                                &format!("__sw_cr_{}", locals.locals.len()),
                                Self::gc_ref_val_type(go_string_idx),
                            );
                            self.compile_expression(expr, out, locals)?;
                            out.push(Instruction::LocalSet(cr));
                            let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                                "switch string tag pointer local missing".to_string(),
                            ))?;
                            self.emit_string_eq_from_locals(
                                sptr, sptr,
                                cr, cr, out, locals,
                            );
                        } else {
                            let cp = locals.add_local(
                                &format!("__sw_cp_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let cl = locals.add_local(
                                &format!("__sw_cl_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            self.compile_expression(expr, out, locals)?;
                            out.push(Instruction::LocalSet(cl));
                            out.push(Instruction::LocalSet(cp));
                            let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                                "switch string tag pointer local missing".to_string(),
                            ))?;
                            let slen = tag_str_len.ok_or_else(|| Error::InternalError(
                                "switch string tag length local missing".to_string(),
                            ))?;
                            self.emit_string_eq_from_locals(
                                sptr, slen,
                                cp, cl, out, locals,
                            );
                        }
                    } else if let Some(tag_l) = tag_local {
                        out.push(Instruction::LocalGet(tag_l));
                        self.compile_expression(expr, out, locals)?;
                        let case_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(case_vt, tag_vt, out)?;
                        out.push(eq_instr.clone());
                    } else {
                        self.compile_expression(expr, out, locals)?;
                    }
                    if !first {
                        out.push(Instruction::I32Or);
                    }
                    first = false;
                }
                out.push(Instruction::LocalGet(ft));
                out.push(Instruction::I32Or);

                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }

                out.push(Instruction::I32Const(0));
                out.push(Instruction::LocalSet(ft));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(matched));

                locals.push_scope();
                self.symbols.enter_scope();
                for stmt in case.body.iter() {
                    if Self::is_fallthrough_stmt(stmt) {
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::LocalSet(ft));
                        continue;
                    }
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
                self.symbols.leave_scope();
                locals.pop_scope();

                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End); // end if
            }

            if let Some(def) = default_case {
                out.push(Instruction::LocalGet(matched));
                out.push(Instruction::I32Eqz);
                out.push(Instruction::LocalGet(ft));
                out.push(Instruction::I32Or);
                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End);
            }
        } else {
            for (i, case) in non_default_cases.iter().enumerate() {
                let mut first = true;
                for expr in &case.list {
                    if is_string_tag {
                        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                            let cr = locals.add_local(
                                &format!("__sw_cr_{}", locals.locals.len()),
                                Self::gc_ref_val_type(go_string_idx),
                            );
                            self.compile_expression(expr, out, locals)?;
                            out.push(Instruction::LocalSet(cr));
                            let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                                "switch string tag pointer local missing".to_string(),
                            ))?;
                            self.emit_string_eq_from_locals(
                                sptr, sptr,
                                cr, cr, out, locals,
                            );
                        } else {
                            let cp = locals.add_local(
                                &format!("__sw_cp_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            let cl = locals.add_local(
                                &format!("__sw_cl_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            self.compile_expression(expr, out, locals)?;
                            out.push(Instruction::LocalSet(cl));
                            out.push(Instruction::LocalSet(cp));
                            let sptr = tag_str_ptr.ok_or_else(|| Error::InternalError(
                                "switch string tag pointer local missing".to_string(),
                            ))?;
                            let slen = tag_str_len.ok_or_else(|| Error::InternalError(
                                "switch string tag length local missing".to_string(),
                            ))?;
                            self.emit_string_eq_from_locals(
                                sptr, slen,
                                cp, cl, out, locals,
                            );
                        }
                    } else if let Some(tag_l) = tag_local {
                        out.push(Instruction::LocalGet(tag_l));
                        self.compile_expression(expr, out, locals)?;
                        let case_vt = self.infer_val_type(expr, locals);
                        Self::emit_typed_coerce(case_vt, tag_vt, out)?;
                        out.push(eq_instr.clone());
                    } else {
                        self.compile_expression(expr, out, locals)?;
                    }
                    if !first {
                        out.push(Instruction::I32Or);
                    }
                    first = false;
                }

                out.push(Instruction::If(BlockType::Empty));
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth += 1;
                }

                locals.push_scope();
                self.symbols.enter_scope();
                for stmt in case.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
                self.symbols.leave_scope();
                locals.pop_scope();

                let is_last = i == num_cases - 1;
                if !is_last || default_case.is_some() {
                    out.push(Instruction::Else);
                } else {
                    if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                        *depth -= 1;
                    }
                    out.push(Instruction::End);
                }
            }

            if let Some(def) = default_case {
                for stmt in def.body.iter() {
                    self.compile_statement(stmt, out, locals, result_types)?;
                }
            }

            let blocks_to_close = if default_case.is_some() {
                num_cases
            } else {
                num_cases.saturating_sub(1)
            };

            for _ in 0..blocks_to_close {
                if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                    *depth -= 1;
                }
                out.push(Instruction::End);
            }
        }

        self.loop_depth.pop();
        out.push(Instruction::End);

        if has_init {
            self.symbols.leave_scope();
            locals.pop_scope();
        }

        Ok(())
    }

    pub(crate) fn compile_branch(
        &mut self,
        branch: &ast::BranchStmt,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        if branch.key == Keyword::Goto {
            let label = branch
                .ident
                .as_ref()
                .ok_or_else(|| Error::InternalError("goto without label".to_string()))?
                .name
                .clone();
            let seg_idx = *self.goto_label_segments.get(&label).ok_or_else(|| {
                Error::InternalError(format!("undefined goto label: {}", label))
            })?;
            let target_local = self.goto_target_local.ok_or_else(|| {
                Error::InternalError("goto outside dispatch context".to_string())
            })?;

            out.push(Instruction::I32Const(seg_idx as i32));
            out.push(Instruction::LocalSet(target_local));

            let depth = self.calculate_goto_dispatch_depth()?;
            out.push(Instruction::Br(depth));
            return Ok(());
        }
        if branch.key == Keyword::FallThrough {
            // Fallthrough is handled by compile_switch; if we reach here,
            // it means fallthrough was used outside a switch — which is a Go error.
            return Err(Error::InternalError(
                "fallthrough statement outside switch".to_string(),
            ));
        }
        if let Some(ref label_ident) = branch.ident {
            return self.compile_labeled_branch(branch.key, &label_ident.name, out);
        }

        match branch.key {
            Keyword::Break => {
                if let Some((label, _, _, _)) = self.loop_depth.last() {
                    if label.as_deref() == Some("__range_over_func") {
                        out.push(Instruction::I32Const(0));
                        out.push(Instruction::Return);
                        return Ok(());
                    }
                }
                let (extra, has_post, is_loop) = self
                    .loop_depth
                    .last()
                    .map(|(_, d, hp, il)| (*d, *hp, *il))
                    .unwrap_or((0, false, true));
                if is_loop {
                    out.push(Instruction::Br(1 + extra + has_post as u32));
                } else {
                    out.push(Instruction::Br(extra));
                }
            }
            Keyword::Continue => {
                let mut depth: u32 = 0;
                let mut found = false;
                for entry in self.loop_depth.iter().rev() {
                    if entry.0.as_deref() == Some("__range_over_func") {
                        out.push(Instruction::I32Const(1));
                        out.push(Instruction::Return);
                        found = true;
                        break;
                    }
                    if entry.3 {
                        depth += entry.1;
                        out.push(Instruction::Br(depth));
                        found = true;
                        break;
                    } else {
                        depth += entry.1 + 1;
                    }
                }
                if !found {
                    return Err(Error::InternalError(
                        "continue statement outside loop".to_string(),
                    ));
                }
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported branch keyword: {:?}",
                    branch.key
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn compile_labeled_branch(
        &mut self,
        key: Keyword,
        label: &str,
        out: &mut Vec<Instruction<'static>>,
    ) -> Result<(), Error> {
        let target_idx = self
            .loop_depth
            .iter()
            .rposition(|(lbl, _, _, _)| lbl.as_deref() == Some(label))
            .ok_or_else(|| {
                Error::InternalError(format!("undefined label: {}", label))
            })?;

        let innermost = self.loop_depth.len() - 1;
        let inner_extra = self.loop_depth[innermost].1;

        let mut intermediate_depth: u32 = 0;

        if innermost > target_idx {
            // Add block levels for the innermost entry (its internal depth
            // is already accounted for by inner_extra, so exclude entry.1).
            let entry = &self.loop_depth[innermost];
            if entry.3 {
                intermediate_depth += 2 + entry.2 as u32;
            } else {
                intermediate_depth += 1;
            }

            // Add full contribution for entries between target and innermost
            for i in (target_idx + 1..innermost).rev() {
                let entry = &self.loop_depth[i];
                if entry.3 {
                    intermediate_depth += 2 + entry.2 as u32 + entry.1;
                } else {
                    intermediate_depth += 1 + entry.1;
                }
            }
        }

        match key {
            Keyword::Break => {
                let target = &self.loop_depth[target_idx];
                if target.3 {
                    let target_has_post = target.2 as u32;
                    let depth = inner_extra + intermediate_depth + target_has_post + 1;
                    out.push(Instruction::Br(depth));
                } else {
                    let depth = inner_extra + intermediate_depth;
                    out.push(Instruction::Br(depth));
                }
            }
            Keyword::Continue => {
                let depth = inner_extra + intermediate_depth;
                out.push(Instruction::Br(depth));
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported branch keyword: {:?}",
                    key
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn emit_incdec_op(op: Operator, vt: ValType, out: &mut Vec<Instruction<'static>>) -> Result<(), Error> {
        match (op, vt) {
            (Operator::Inc, ValType::I64) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Add);
            }
            (Operator::Dec, ValType::I64) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Sub);
            }
            (Operator::Inc, ValType::I32) => {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Add);
            }
            (Operator::Dec, ValType::I32) => {
                out.push(Instruction::I32Const(1));
                out.push(Instruction::I32Sub);
            }
            (Operator::Inc, ValType::F64) => {
                out.push(Instruction::F64Const(1.0_f64.into()));
                out.push(Instruction::F64Add);
            }
            (Operator::Dec, ValType::F64) => {
                out.push(Instruction::F64Const(1.0_f64.into()));
                out.push(Instruction::F64Sub);
            }
            (Operator::Inc, ValType::F32) => {
                out.push(Instruction::F32Const(1.0_f32.into()));
                out.push(Instruction::F32Add);
            }
            (Operator::Dec, ValType::F32) => {
                out.push(Instruction::F32Const(1.0_f32.into()));
                out.push(Instruction::F32Sub);
            }
            (Operator::Inc, _) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Add);
            }
            (Operator::Dec, _) => {
                out.push(Instruction::I64Const(1));
                out.push(Instruction::I64Sub);
            }
            _ => {
                return Err(Error::InternalError(format!(
                    "unsupported inc/dec operator: {:?}",
                    op
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn compile_incdec(
        &mut self,
        incdec: &ast::IncDecStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match &incdec.expr {
            ast::Expression::Ident(ident) => {
                if let Some(&(mb_offset, mb_vt)) = locals.memory_backed_vars.get(&ident.name) {
                    if let Some(sf) = &self.current_stack_frame {
                        if let Some(fb) = sf.frame_base_local {
                            out.push(Instruction::LocalGet(fb));
                            if mb_offset > 0 {
                                out.push(Instruction::I32Const(mb_offset as i32));
                                out.push(Instruction::I32Add);
                            }
                            let addr_tmp = locals.add_local(
                                &format!("__mb_incdec_addr_{}", locals.locals.len()),
                                ValType::I32,
                            );
                            out.push(Instruction::LocalTee(addr_tmp));
                            let (_, align) = Self::elem_size_and_align(mb_vt);
                            Self::emit_typed_load(mb_vt, 0, align, out);
                            Self::emit_incdec_op(incdec.op, mb_vt, out)?;
                            let result_tmp = locals.add_local(
                                &format!("__mb_incdec_res_{}", locals.locals.len()),
                                mb_vt,
                            );
                            out.push(Instruction::LocalSet(result_tmp));
                            out.push(Instruction::LocalGet(addr_tmp));
                            out.push(Instruction::LocalGet(result_tmp));
                            Self::emit_typed_store(mb_vt, 0, align, out);
                        }
                    }
                } else if let Some(idx) = locals.find(&ident.name) {
                    out.push(Instruction::LocalGet(idx));
                    let vt = self.infer_val_type(&incdec.expr, locals);
                    Self::emit_incdec_op(incdec.op, vt, out)?;
                    out.push(Instruction::LocalSet(idx));
                } else if let Some(&(global_idx, vt)) = self.resolve_global_var(&ident.name) {
                    out.push(Instruction::GlobalGet(global_idx));
                    Self::emit_incdec_op(incdec.op, vt, out)?;
                    out.push(Instruction::GlobalSet(global_idx));
                } else {
                    return Err(Error::InternalError(format!(
                        "undefined variable '{}' in increment/decrement",
                        ident.name
                    )));
                }
            }
            ast::Expression::Index(idx_expr) => {
                let (elem_vt, align) =
                    self.compile_index_store_addr(idx_expr, out, locals)?;
                let addr_tmp = locals.add_local(
                    &format!("__incdec_addr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(addr_tmp));

                out.push(Instruction::LocalGet(addr_tmp));
                Self::emit_typed_load(elem_vt, 0, align, out);

                Self::emit_incdec_op(incdec.op, elem_vt, out)?;

                let result_tmp = locals.add_local(
                    &format!("__incdec_res_{}", locals.locals.len()),
                    elem_vt,
                );
                out.push(Instruction::LocalSet(result_tmp));
                out.push(Instruction::LocalGet(addr_tmp));
                out.push(Instruction::LocalGet(result_tmp));
                Self::emit_typed_store(elem_vt, 0, align, out);
            }
            ast::Expression::Selector(sel) => {
                let (offset, field_vt) =
                    self.compile_selector_store_addr(sel, out, locals)?;
                let (_, align) = Self::elem_size_and_align(field_vt);
                let addr_tmp = locals.add_local(
                    &format!("__incdec_addr_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::LocalSet(addr_tmp));

                out.push(Instruction::LocalGet(addr_tmp));
                Self::emit_typed_load(field_vt, offset, align, out);

                Self::emit_incdec_op(incdec.op, field_vt, out)?;

                let result_tmp = locals.add_local(
                    &format!("__incdec_res_{}", locals.locals.len()),
                    field_vt,
                );
                out.push(Instruction::LocalSet(result_tmp));
                out.push(Instruction::LocalGet(addr_tmp));
                out.push(Instruction::LocalGet(result_tmp));
                Self::emit_typed_store(field_vt, offset, align, out);
            }
            _ => {
                return Err(Error::InternalError(
                    "increment/decrement on unsupported expression".to_string(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn compile_decl_stmt(
        &mut self,
        decl: &ast::DeclStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        match decl {
            ast::DeclStmt::Variable(var_decl) => {
                for spec in &var_decl.specs {
                    if let Some(ref typ) = spec.typ {
                        Self::reject_unsupported_type(typ)?;
                    }
                    for (i, ident) in spec.name.iter().enumerate() {
                        let vt = if let Some(ref typ) = spec.typ {
                            self.expr_to_val_type(typ)
                        } else if i < spec.values.len() {
                            self.infer_val_type(&spec.values[i], locals)
                        } else {
                            ValType::I64
                        };

                        let local_idx = locals.add_local(&ident.name, vt);

                        // Track struct, string, interface, and unsigned types
                        let mut is_string = false;
                        let mut is_iface = false;
                        let mut iface_type_name: Option<String> = None;
                        if let Some(ref typ) = spec.typ {
                            if let Some(dt) = self.expr_to_define_type(typ) {
                                self.define_var(&ident.name, dt);
                            }
                            if let ast::Expression::TypeInterface(_) = typ {
                                is_iface = true;
                                locals.set_var_struct_type(&ident.name, "__interface");
                                let tid_local = locals.add_local(
                                    &format!("{}__type_id", ident.name),
                                    ValType::I32,
                                );
                                locals.iface_type_id_locals.insert(ident.name.clone(), tid_local);
                            } else if let ast::Expression::TypeSlice(slice_type) = typ {
                                locals.set_var_struct_type(&ident.name, "__slice");
                                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                                locals.slice_elem_types.insert(ident.name.clone(), elem_vt);
                                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                                    locals.nested_slice_inner_elem_types.insert(ident.name.clone(), inner_vt);
                                }
                                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                                    let resolved_elem = self.resolve_struct_in_pkg(&el_id.name);
                                    if self.struct_defs.contains_key(&resolved_elem) {
                                        locals.slice_elem_struct_types.insert(ident.name.clone(), resolved_elem);
                                    }
                                }
                            } else if let ast::Expression::TypeMap(map_type) = typ {
                                locals.set_var_struct_type(&ident.name, "__map");
                                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                                let nested = self.build_nested_map_type_info(map_type);
                                locals.map_types.insert(
                                    ident.name.clone(),
                                    MapTypeInfo { key_vt: kv, val_vt: vv, key_size: ks, val_size: vs, is_string_key: sk, is_string_val: sv, val_struct_type: vst, nested_map_val_type: nested },
                                );
                            } else if let ast::Expression::TypeArray(arr_type) = typ {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                let (go_es, go_ea) = Self::go_type_elem_size_and_align(&arr_type.typ);
                                locals.set_var_struct_type(&ident.name, "__array");
                                locals.array_info.insert(ident.name.clone(), (elem_vt, arr_len, go_es, go_ea));
                                if let ast::Expression::TypeArray(inner_arr) = arr_type.typ.as_ref() {
                                    let inner_len = if let ast::Expression::BasicLit(lit) = inner_arr.len.as_ref() {
                                        Self::parse_go_int(&lit.value)
                                            .map_err(|e| Error::SyntaxError(e))? as u32
                                    } else { 0 };
                                    let inner_elem_vt = Self::infer_array_elem_vt(&inner_arr.typ);
                                    locals.nested_array_inner_info.insert(ident.name.clone(), (inner_elem_vt, inner_len));
                                }
                            } else if let ast::Expression::Ident(type_ident) = typ {
                                if type_ident.name == "string" {
                                    is_string = true;
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__string",
                                    );
                                } else if type_ident.name == "complex64" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__complex64",
                                    );
                                } else if type_ident.name == "complex128" {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        "__complex128",
                                    );
                                } else if self.iface_defs.contains_key(&type_ident.name)
                                    || type_ident.name == "error"
                                    || type_ident.name == "any"
                                {
                                    is_iface = true;
                                    iface_type_name = Some(type_ident.name.clone());
                                    let iface_tag = format!("__iface_{}", type_ident.name);
                                    locals.set_var_struct_type(&ident.name, &iface_tag);
                                    let tid_local = locals.add_local(
                                        &format!("{}__type_id", ident.name),
                                        ValType::I32,
                                    );
                                    locals.iface_type_id_locals.insert(ident.name.clone(), tid_local);
                                } else if self.struct_defs.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                } else {
                                    let resolved_struct = self.resolve_struct_in_pkg(&type_ident.name);
                                    if self.struct_defs.contains_key(&resolved_struct) {
                                        locals.set_var_struct_type(
                                            &ident.name,
                                            &resolved_struct,
                                        );
                                    }
                                }
                                if self.type_aliases.contains_key(&type_ident.name) {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                } else if let Some(underlying) = self.named_composite_types.get(&type_ident.name).cloned() {
                                    self.setup_named_composite_var(&ident.name, &underlying, locals);
                                }
                                if Self::is_unsigned_type_name(&type_ident.name) {
                                    locals.unsigned_vars.insert(ident.name.clone());
                                }
                            } else if let ast::Expression::Selector(sel) = typ {
                                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                                    let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                                    if self.struct_defs.contains_key(&qualified) {
                                        locals.set_var_struct_type(&ident.name, &qualified);
                                    } else if self.type_aliases.contains_key(&qualified) {
                                        locals.set_var_struct_type(&ident.name, &qualified);
                                    } else if self.iface_defs.contains_key(&qualified) {
                                        is_iface = true;
                                        iface_type_name = Some(qualified.clone());
                                        let iface_tag = format!("__iface_{}", qualified);
                                        locals.set_var_struct_type(&ident.name, &iface_tag);
                                        let tid_local = locals.add_local(
                                            &format!("{}__type_id", ident.name),
                                            ValType::I32,
                                        );
                                        locals.iface_type_id_locals.insert(ident.name.clone(), tid_local);
                                    }
                                }
                            } else if let ast::Expression::TypePointer(ptr) = typ {
                                if let ast::Expression::Ident(type_ident) = ptr.typ.as_ref() {
                                    let resolved_ptr_struct = self.resolve_struct_in_pkg(&type_ident.name);
                                    if self.struct_defs.contains_key(&resolved_ptr_struct) {
                                        locals.set_var_struct_type(&ident.name, &resolved_ptr_struct);
                                        locals.pointer_to_struct_vars.insert(ident.name.clone());
                                    }
                                }
                            }
                        }
                        if i < spec.values.len() && !is_iface {
                            if let ast::Expression::CompositeLit(comp) = &spec.values[i]
                            {
                                if let ast::Expression::Ident(type_ident) =
                                    comp.typ.as_ref()
                                {
                                    locals.set_var_struct_type(
                                        &ident.name,
                                        &type_ident.name,
                                    );
                                }
                            }
                            if let ast::Expression::Operation(addr_op) = &spec.values[i] {
                                if addr_op.op == Operator::And && addr_op.y.is_none() {
                                    if let ast::Expression::CompositeLit(comp) = &*addr_op.x {
                                        if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                                            locals.set_var_struct_type(
                                                &ident.name,
                                                &type_ident.name,
                                            );
                                        }
                                    }
                                }
                            }
                            if !is_string && self.is_string_expr(&spec.values[i], locals) {
                                is_string = true;
                                locals.set_var_struct_type(
                                    &ident.name,
                                    "__string",
                                );
                            }
                            if let ast::Expression::Ident(rhs_ident) = &spec.values[i] {
                                if let Some(struct_type) = locals.get_var_struct_type(&rhs_ident.name).map(|s| s.to_string()) {
                                    if locals.get_var_struct_type(&ident.name).is_none() {
                                        locals.set_var_struct_type(&ident.name, &struct_type);
                                    }
                                }
                                if let Some(&info) = locals.array_info.get(&rhs_ident.name) {
                                    if !locals.array_info.contains_key(&ident.name) {
                                        locals.array_info.insert(ident.name.clone(), info);
                                    }
                                }
                            }
                        }

                        if is_string {
                            if let Some(go_string_idx) = self.gc_builtin_types.go_string {
                                locals.gc_string_locals.insert(
                                    ident.name.clone(),
                                    local_idx,
                                );
                                if i >= spec.values.len() {
                                    let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::ArrayNew(byte_array_idx));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::StructNew(go_string_idx));
                                    out.push(Instruction::LocalSet(local_idx));
                                }
                            } else {
                                let len_local = locals.add_local(
                                    &format!("{}__str_len", ident.name),
                                    ValType::I32,
                                );
                                locals.string_locals.insert(
                                    ident.name.clone(),
                                    (local_idx, len_local),
                                );
                            }
                        }

                        if i < spec.values.len() {
                            if self.current_stack_frame.is_some() {
                                let is_struct_lit = matches!(
                                    &spec.values[i],
                                    ast::Expression::CompositeLit(comp)
                                        if matches!(comp.typ.as_ref(),
                                            ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name))
                                );
                                let is_addr_of_struct_lit = matches!(
                                    &spec.values[i],
                                    ast::Expression::Operation(op)
                                        if op.y.is_none()
                                            && matches!(op.op, Operator::And)
                                            && matches!(op.x.as_ref(),
                                                ast::Expression::CompositeLit(comp)
                                                    if matches!(comp.typ.as_ref(),
                                                        ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name)))
                                );
                                let is_new_struct = matches!(
                                    &spec.values[i],
                                    ast::Expression::Call(call)
                                        if matches!(call.func.as_ref(), ast::Expression::Ident(fn_id) if fn_id.name == "new")
                                            && call.args.first().map_or(false, |a|
                                                matches!(a, ast::Expression::Ident(ti) if self.struct_defs.contains_key(&ti.name)))
                                );
                                if is_struct_lit || is_addr_of_struct_lit || is_new_struct {
                                    self.stack_alloc_target = Some(ident.name.clone());
                                }
                            }

                            let compile_result = self.compile_expression(&spec.values[i], out, locals);
                            self.stack_alloc_target = None;
                            let expr_type = compile_result?;
                            if is_iface {
                                let rhs_vt = self.infer_val_type(&spec.values[i], locals);
                                let rhs_type_name = self.infer_concrete_type_name(&spec.values[i], locals);
                                if let Some(ref iname) = iface_type_name {
                                    if !self.is_interface_var_expr(&spec.values[i], locals) {
                                        self.check_iface_satisfaction(iname, &rhs_type_name)?;
                                    }
                                }
                                let type_id = self.get_or_create_type_id(&rhs_type_name);
                                let tid_local = self.get_iface_type_id_local(&ident.name, locals).ok_or_else(|| {
                                    Error::InternalError(format!(
                                        "interface type-id local not found for '{}'", ident.name
                                    ))
                                })?;
                                let (elem_size, _) = Self::elem_size_and_align(rhs_vt);
                                self.emit_box_value(rhs_vt, elem_size, type_id, tid_local, local_idx, out, locals)?;
                            } else if is_string {
                                if let Some(&gc_ref_idx) = locals.gc_string_locals.get(&ident.name) {
                                    out.push(Instruction::LocalSet(gc_ref_idx));
                                } else {
                                    let (ptr_local, len_local) = locals.string_locals[&ident.name];
                                    out.push(Instruction::LocalSet(len_local));
                                    out.push(Instruction::LocalSet(ptr_local));
                                }
                            } else if let Some(&(mb_offset, mb_vt)) = locals.memory_backed_vars.get(&ident.name) {
                                if let Some(sf) = &self.current_stack_frame {
                                    if let Some(fb) = sf.frame_base_local {
                                        let val_tmp = locals.add_local(
                                            &format!("__mb_decl_{}", locals.locals.len()),
                                            mb_vt,
                                        );
                                        let expr_vt = expr_type.wasm_type();
                                        if expr_vt != mb_vt {
                                            Self::emit_typed_coerce(expr_vt, mb_vt, out)?;
                                        }
                                        out.push(Instruction::LocalSet(val_tmp));
                                        out.push(Instruction::LocalGet(fb));
                                        if mb_offset > 0 {
                                            out.push(Instruction::I32Const(mb_offset as i32));
                                            out.push(Instruction::I32Add);
                                        }
                                        out.push(Instruction::LocalGet(val_tmp));
                                        let (_, align) = Self::elem_size_and_align(mb_vt);
                                        Self::emit_typed_store(mb_vt, 0, align, out);
                                    }
                                }
                            } else {
                                let gc_copy = if i < spec.values.len() {
                                    if let ast::Expression::Ident(rhs_ident) = &spec.values[i] {
                                        self.get_gc_copy_info(&rhs_ident.name, locals)
                                    } else { None }
                                } else { None };
                                if let Some((gc_idx, gc_sd)) = gc_copy {
                                    Self::emit_gc_value_deep_copy(gc_idx, &gc_sd, out, locals);
                                } else {
                                    let needs_deep_copy = if i < spec.values.len() {
                                        if let ast::Expression::Ident(rhs_ident) = &spec.values[i] {
                                            self.get_value_copy_size(&rhs_ident.name, locals).is_some()
                                        } else { false }
                                    } else { false };
                                    if needs_deep_copy {
                                        if let ast::Expression::Ident(rhs_ident) = &spec.values[i] {
                                            let size = self.get_value_copy_size(&rhs_ident.name, locals).unwrap();
                                            self.emit_value_deep_copy(size, out, locals)?;
                                        }
                                    } else {
                                        if spec.typ.is_some() {
                                            let expr_vt = expr_type.wasm_type();
                                            if expr_vt != vt {
                                                Self::emit_typed_coerce(expr_vt, vt, out)?;
                                            }
                                        }
                                    }
                                }
                                out.push(Instruction::LocalSet(local_idx));
                            }
                        } else if let Some(ref typ) = spec.typ {
                            if let ast::Expression::Ident(type_ident) = typ {
                                if let Some(sd) = self.struct_defs.get(&type_ident.name) {
                                    if let Some(gc_idx) = sd.gc_type_idx {
                                        out.push(Instruction::StructNewDefault(gc_idx));
                                        out.push(Instruction::LocalSet(local_idx));
                                    } else {
                                        let size = sd.total_size as i32;
                                        let used_stack = if let Some(sf) = &self.current_stack_frame {
                                            if let Some(sl) = sf.find(&ident.name) {
                                                if let Some(fb) = sf.frame_base_local {
                                                    out.push(Instruction::LocalGet(fb));
                                                    if sl.offset > 0 {
                                                        out.push(Instruction::I32Const(sl.offset as i32));
                                                        out.push(Instruction::I32Add);
                                                    }
                                                    true
                                                } else { false }
                                            } else { false }
                                        } else { false };
                                        if !used_stack {
                                            out.push(Instruction::I32Const(size));
                                            out.push(Instruction::Call(self.alloc_func_idx()?));
                                        }
                                        out.push(Instruction::LocalTee(local_idx));
                                        out.push(Instruction::I32Const(0));
                                        out.push(Instruction::I32Const(size));
                                        out.push(Instruction::MemoryFill(0));
                                    }
                                }
                            } else if let ast::Expression::TypeStruct(_) = typ {
                                let field_count = if let ast::Expression::TypeStruct(st) = typ {
                                    st.fields.len()
                                } else {
                                    0
                                };
                                let size = ((field_count * 8) as i32).max(8);
                                out.push(Instruction::I32Const(size));
                                out.push(Instruction::Call(self.alloc_func_idx()?));
                                out.push(Instruction::LocalTee(local_idx));
                                out.push(Instruction::I32Const(0));
                                out.push(Instruction::I32Const(size));
                                out.push(Instruction::MemoryFill(0));
                            } else if let ast::Expression::TypeArray(arr_type) = typ {
                                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                                    Self::parse_go_int(&lit.value)
                                        .map_err(|e| Error::SyntaxError(e))? as u32
                                } else { 0 };
                                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                                let (elem_size, _) = Self::elem_size_and_align(elem_vt);
                                let total_bytes = if let Some(&(inner_elem_vt, inner_len)) = locals.nested_array_inner_info.get(&ident.name) {
                                    let (inner_elem_size, _) = Self::elem_size_and_align(inner_elem_vt);
                                    (arr_len * inner_len * inner_elem_size as u32) as i32
                                } else {
                                    (arr_len as i32) * elem_size
                                };
                                if total_bytes > 0 {
                                    out.push(Instruction::I32Const(total_bytes));
                                    out.push(Instruction::Call(self.alloc_func_idx()?));
                                    out.push(Instruction::LocalTee(local_idx));
                                    out.push(Instruction::I32Const(0));
                                    out.push(Instruction::I32Const(total_bytes));
                                    out.push(Instruction::MemoryFill(0));
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            ast::DeclStmt::Const(const_decl) => {
                let old_iota = self.current_iota;
                let mut last_exprs: Vec<ast::Expression> = Vec::new();
                for (iota_val, spec) in const_decl.specs.iter().enumerate() {
                    self.current_iota = Some(iota_val as i128);
                    let exprs_to_use: &[ast::Expression] = if spec.values.is_empty() {
                        last_exprs.as_slice()
                    } else {
                        last_exprs = spec.values.clone();
                        spec.values.as_slice()
                    };
                    for (i, ident) in spec.name.iter().enumerate() {
                        let expr = exprs_to_use.get(i).or(exprs_to_use.first());
                        if let Some(expr) = expr {
                            let cv = self.eval_const_expr(expr, &ident.name)?;
                            self.constants.insert(ident.name.clone(), cv);
                        }
                    }
                }
                self.current_iota = old_iota;
                Ok(())
            }
            ast::DeclStmt::Type(type_decl) => {
                for spec in &type_decl.specs {
                    Self::reject_unsupported_type(&spec.typ)?;
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(spec.name.name.clone(), struct_def);
                        self.register_struct_field_map_types(&spec.name.name, &struct_type.fields);
                    }
                }
                Ok(())
            }
        }
    }

    pub(crate) fn block_always_returns(stmts: &[ast::Statement]) -> bool {
        stmts
            .iter()
            .rev()
            .find(|s| !matches!(s, ast::Statement::Empty(_)))
            .map_or(false, |s| Self::stmt_always_returns(s))
    }

    pub(crate) fn stmt_always_returns(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Return(_) => true,
            ast::Statement::Expr(expr_stmt) => Self::expr_is_panic_call(&expr_stmt.expr),
            ast::Statement::If(if_stmt) => {
                let then_returns = Self::block_always_returns(&if_stmt.body.list);
                let else_returns =
                    if_stmt
                        .else_
                        .as_ref()
                        .map_or(false, |els| match els.as_ref() {
                            ast::Statement::Block(block) => {
                                Self::block_always_returns(&block.list)
                            }
                            other => Self::stmt_always_returns(other),
                        });
                then_returns && else_returns
            }
            ast::Statement::Block(block) => Self::block_always_returns(&block.list),
            ast::Statement::For(for_stmt) => {
                for_stmt.cond.is_none()
                    && !Self::block_contains_break(&for_stmt.body.list, None)
            }
            ast::Statement::Switch(sw) => {
                Self::switch_is_terminating(&sw.block.body, None)
            }
            ast::Statement::TypeSwitch(ts) => {
                let has_default = ts.block.body.iter().any(|c| c.list.is_empty());
                if !has_default {
                    return false;
                }
                let cases_terminate = ts.block.body.iter().all(|c| Self::case_body_terminates(&c.body));
                if !cases_terminate {
                    return false;
                }
                !Self::switch_cases_contain_break(
                    ts.block.body.iter().map(|c| c.body.as_slice()),
                    None,
                )
            }
            ast::Statement::Range(_) => false,
            ast::Statement::Label(labeled) => {
                match labeled.stmt.as_ref() {
                    ast::Statement::For(for_stmt) => {
                        for_stmt.cond.is_none()
                            && !Self::block_contains_break(
                                &for_stmt.body.list,
                                Some(&labeled.name.name),
                            )
                    }
                    ast::Statement::Range(_) => false,
                    ast::Statement::Switch(sw) => {
                        Self::switch_is_terminating(&sw.block.body, Some(&labeled.name.name))
                    }
                    ast::Statement::TypeSwitch(ts) => {
                        let has_default = ts.block.body.iter().any(|c| c.list.is_empty());
                        if !has_default {
                            return false;
                        }
                        let cases_terminate = ts.block.body.iter().all(|c| Self::case_body_terminates(&c.body));
                        if !cases_terminate {
                            return false;
                        }
                        !Self::switch_cases_contain_break(
                            ts.block.body.iter().map(|c| c.body.as_slice()),
                            Some(&labeled.name.name),
                        )
                    }
                    other => Self::stmt_always_returns(other),
                }
            }
            _ => false,
        }
    }

    pub(crate) fn expr_is_panic_call(expr: &ast::Expression) -> bool {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                return ident.name == "panic";
            }
        }
        false
    }

    pub(crate) fn block_contains_break(stmts: &[ast::Statement], for_label: Option<&str>) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b) if b.key == Keyword::Break => {
                    if b.ident.is_none() {
                        return true;
                    }
                    if let (Some(brk_label), Some(fl)) = (&b.ident, for_label) {
                        if brk_label.name == fl {
                            return true;
                        }
                    }
                }
                ast::Statement::If(if_stmt) => {
                    if Self::block_contains_break(&if_stmt.body.list, for_label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::block_contains_break(&block.list, for_label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::block_contains_break(&block.list, for_label) {
                        return true;
                    }
                }
                ast::Statement::Switch(sw) if for_label.is_some() => {
                    for case in &sw.block.body {
                        if Self::block_contains_labeled_break(&case.body, for_label.unwrap()) {
                            return true;
                        }
                    }
                }
                ast::Statement::TypeSwitch(ts) if for_label.is_some() => {
                    for case in &ts.block.body {
                        if Self::block_contains_labeled_break(&case.body, for_label.unwrap()) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub(crate) fn block_contains_labeled_break(stmts: &[ast::Statement], label: &str) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b)
                    if b.key == Keyword::Break
                        && b.ident.as_ref().map_or(false, |id| id.name == label) =>
                {
                    return true;
                }
                ast::Statement::If(if_stmt) => {
                    if Self::block_contains_labeled_break(&if_stmt.body.list, label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::block_contains_labeled_break(&block.list, label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::block_contains_labeled_break(&block.list, label) {
                        return true;
                    }
                }
                ast::Statement::Switch(sw) => {
                    for case in &sw.block.body {
                        if Self::block_contains_labeled_break(&case.body, label) {
                            return true;
                        }
                    }
                }
                ast::Statement::TypeSwitch(ts) => {
                    for case in &ts.block.body {
                        if Self::block_contains_labeled_break(&case.body, label) {
                            return true;
                        }
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub(crate) fn switch_is_terminating(cases: &[ast::CaseClause], label: Option<&str>) -> bool {
        let has_default = cases.iter().any(|c| c.tok == Keyword::Default);
        if !has_default {
            return false;
        }
        let cases_terminate = cases.iter().all(|c| Self::case_body_terminates(&c.body));
        if !cases_terminate {
            return false;
        }
        !Self::switch_cases_contain_break(
            cases.iter().map(|c| c.body.as_slice()),
            label,
        )
    }

    pub(crate) fn switch_cases_contain_break<'a>(
        cases: impl Iterator<Item = &'a [ast::Statement]>,
        label: Option<&str>,
    ) -> bool {
        for case_body in cases {
            if Self::case_stmts_contain_unlabeled_break(case_body, label) {
                return true;
            }
        }
        false
    }

    pub(crate) fn case_stmts_contain_unlabeled_break(stmts: &[ast::Statement], switch_label: Option<&str>) -> bool {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b) if b.key == Keyword::Break => {
                    if b.ident.is_none() {
                        return true;
                    }
                    if let Some(ref brk_label) = b.ident {
                        if let Some(sw_label) = switch_label {
                            if brk_label.name == sw_label {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::If(if_stmt) => {
                    if Self::case_stmts_contain_unlabeled_break(&if_stmt.body.list, switch_label) {
                        return true;
                    }
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            if Self::case_stmts_contain_unlabeled_break(&block.list, switch_label) {
                                return true;
                            }
                        }
                    }
                }
                ast::Statement::Block(block) => {
                    if Self::case_stmts_contain_unlabeled_break(&block.list, switch_label) {
                        return true;
                    }
                }
                // Don't recurse into for/switch/select - break inside those refers to them, not the outer switch
                _ => {}
            }
        }
        false
    }

    pub(crate) fn case_body_terminates(stmts: &[ast::Statement]) -> bool {
        let last = stmts
            .iter()
            .rev()
            .find(|s| !matches!(s, ast::Statement::Empty(_)));
        if let Some(last) = last {
            if Self::stmt_always_returns(last) {
                return true;
            }
            if Self::is_fallthrough_stmt(last) {
                return true;
            }
        }
        false
    }

    pub(crate) fn is_fallthrough_stmt(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Branch(b) => b.key == Keyword::FallThrough,
            ast::Statement::Label(labeled) => Self::is_fallthrough_stmt(&labeled.stmt),
            _ => false,
        }
    }

    pub(crate) fn contains_fallthrough(stmt: &ast::Statement) -> bool {
        match stmt {
            ast::Statement::Branch(b) => b.key == Keyword::FallThrough,
            ast::Statement::Label(labeled) => Self::contains_fallthrough(&labeled.stmt),
            ast::Statement::Block(block) => block.list.iter().any(|s| Self::contains_fallthrough(s)),
            ast::Statement::If(if_stmt) => {
                if_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
                    || if_stmt.else_.as_ref().map_or(false, |e| Self::contains_fallthrough(e))
            }
            ast::Statement::For(for_stmt) => {
                for_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
            }
            ast::Statement::Range(range_stmt) => {
                range_stmt.body.list.iter().any(|s| Self::contains_fallthrough(s))
            }
            _ => false,
        }
    }

    /// Shared type-tracking for a local variable based on its known Go type.
    /// Returns `true` if the Go type is an interface type.
    pub(crate) fn track_local_var_type(
        &mut self,
        name: &str,
        go_type: &str,
        local_idx: u32,
        locals: &mut LocalAlloc,
    ) -> bool {
        let dt = self.go_type_name_to_define_type(go_type);
        self.define_var(name, dt);

        if go_type == "string" {
            locals.set_var_struct_type(name, "__string");
            if self.gc_builtin_types.go_string.is_some() {
                locals.gc_string_locals.insert(name.to_string(), local_idx);
            } else if !locals.string_locals.contains_key(name) {
                let len_local = locals.add_local(
                    &format!("{}__str_len", name),
                    ValType::I32,
                );
                locals.string_locals.insert(name.to_string(), (local_idx, len_local));
            }
            return false;
        }

        if self.is_iface_go_type(go_type) {
            let iface_tag = format!("__iface_{}", go_type);
            locals.set_var_struct_type(name, &iface_tag);
            if !locals.iface_type_id_locals.contains_key(name) {
                let tid_local = locals.add_local(
                    &format!("{}__type_id", name),
                    ValType::I32,
                );
                locals.iface_type_id_locals.insert(name.to_string(), tid_local);
            }
            return true;
        }

        let base_type = go_type.strip_prefix('*').unwrap_or(go_type);
        let resolved = self.resolve_struct_in_pkg(base_type);
        if self.struct_defs.contains_key(&resolved) {
            locals.set_var_struct_type(name, &resolved);
            if go_type.starts_with('*') {
                locals.pointer_to_struct_vars.insert(name.to_string());
            }
        } else if go_type.starts_with("[]") {
            locals.set_var_struct_type(name, "__slice");
        } else if self.type_aliases.contains_key(go_type)
            || self.current_package.as_ref().map_or(false, |pkg| {
                self.type_aliases.contains_key(&format!("{}.{}", pkg, go_type))
            })
        {
            locals.set_var_struct_type(name, go_type);
        }

        false
    }

    /// Define or find a local variable, track its Go type, and emit assignment
    /// from temp local(s). For non-GC strings, `temp_locals` has two entries
    /// (ptr, len); for everything else it has one.
    /// Returns the primary local index.
    pub(crate) fn assign_local_from_temp(
        &mut self,
        name: &str,
        go_type: &str,
        wasm_vt: ValType,
        temp_locals: &[(u32, ValType)],
        is_define: bool,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> u32 {
        let local_idx = if is_define {
            if let Some(existing) = locals.find_at_current_scope(name) {
                existing
            } else {
                locals.add_local(name, wasm_vt)
            }
        } else {
            locals.find(name).unwrap_or_else(|| locals.add_local(name, wasm_vt))
        };

        let is_iface = self.track_local_var_type(name, go_type, local_idx, locals);

        if temp_locals.len() == 2 {
            if let Some(&(ptr_local, len_local)) = locals.string_locals.get(name) {
                out.push(Instruction::LocalGet(temp_locals[0].0));
                out.push(Instruction::LocalSet(ptr_local));
                out.push(Instruction::LocalGet(temp_locals[1].0));
                out.push(Instruction::LocalSet(len_local));
                return local_idx;
            }
        }

        if is_iface && temp_locals.len() == 1 {
            if let Some(&tid_local) = locals.iface_type_id_locals.get(name) {
                let box_ptr = temp_locals[0].0;
                out.push(Instruction::LocalGet(box_ptr));
                out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalSet(local_idx));
                out.push(Instruction::LocalGet(box_ptr));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalSet(tid_local));
                return local_idx;
            }
        }

        out.push(Instruction::LocalGet(temp_locals[0].0));
        out.push(Instruction::LocalSet(local_idx));
        local_idx
    }

    pub(crate) fn compile_multi_return_define(
        &mut self,
        assign: &ast::AssignStmt,
        ret_types: &[ValType],
        go_types: &[String],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(&assign.right[0], out, locals)?;

        let mut all_temps: Vec<(u32, ValType)> = Vec::new();
        for i in (0..ret_types.len()).rev() {
            let vt = ret_types[i];
            let tmp = locals.add_local(&format!("__mret_tmp_{}", i), vt);
            out.push(Instruction::LocalSet(tmp));
            all_temps.push((tmp, vt));
        }
        all_temps.reverse();

        let gc_string = self.gc_builtin_types.go_string.is_some();
        let mut go_to_wasm: Vec<(usize, usize)> = Vec::new();
        let mut wasm_idx = 0;
        for go_type in go_types.iter() {
            let slot_count = if !gc_string && go_type == "string" { 2 } else { 1 };
            go_to_wasm.push((wasm_idx, slot_count));
            wasm_idx += slot_count;
        }

        let is_define = assign.op == Operator::Define;
        for (i, left) in assign.left.iter().enumerate() {
            if let ast::Expression::Ident(ident) = left {
                if ident.name == "_" {
                    continue;
                }
                let (slot_start, slot_count) = go_to_wasm.get(i).copied().unwrap_or((i, 1));
                let vt = ret_types[slot_start];
                let go_type = go_types.get(i).map(|s| s.as_str()).unwrap_or("");
                let temps = &all_temps[slot_start..slot_start + slot_count];

                self.assign_local_from_temp(&ident.name, go_type, vt, temps, is_define, out, locals);
            }
        }

        Ok(())
    }

    pub(crate) fn scan_goto_targets(body: &ast::BlockStmt) -> HashSet<String> {
        let mut targets = HashSet::new();
        Self::collect_goto_targets(&body.list, &mut targets);
        targets
    }

    fn collect_goto_targets(stmts: &[ast::Statement], targets: &mut HashSet<String>) {
        for stmt in stmts {
            match stmt {
                ast::Statement::Branch(b) if b.key == Keyword::Goto => {
                    if let Some(ref ident) = b.ident {
                        targets.insert(ident.name.clone());
                    }
                }
                ast::Statement::If(if_stmt) => {
                    Self::collect_goto_targets(&if_stmt.body.list, targets);
                    if let Some(ref els) = if_stmt.else_ {
                        if let ast::Statement::Block(block) = els.as_ref() {
                            Self::collect_goto_targets(&block.list, targets);
                        } else {
                            Self::collect_goto_targets(std::slice::from_ref(els.as_ref()), targets);
                        }
                    }
                }
                ast::Statement::For(for_stmt) => {
                    Self::collect_goto_targets(&for_stmt.body.list, targets);
                }
                ast::Statement::Range(range) => {
                    Self::collect_goto_targets(&range.body.list, targets);
                }
                ast::Statement::Block(block) => {
                    Self::collect_goto_targets(&block.list, targets);
                }
                ast::Statement::Switch(sw) => {
                    for case in &sw.block.body {
                        Self::collect_goto_targets(&case.body, targets);
                    }
                }
                ast::Statement::TypeSwitch(ts) => {
                    for case in &ts.block.body {
                        Self::collect_goto_targets(&case.body, targets);
                    }
                }
                ast::Statement::Label(labeled) => {
                    Self::collect_goto_targets(
                        std::slice::from_ref(labeled.stmt.as_ref()),
                        targets,
                    );
                }
                _ => {}
            }
        }
    }

    fn split_goto_segments<'a>(
        stmts: &'a [ast::Statement],
        goto_targets: &HashSet<String>,
    ) -> (Vec<&'a [ast::Statement]>, HashMap<String, u32>) {
        let mut segments: Vec<&'a [ast::Statement]> = Vec::new();
        let mut label_to_segment: HashMap<String, u32> = HashMap::new();
        let mut seg_start: usize = 0;

        for (i, stmt) in stmts.iter().enumerate() {
            if let ast::Statement::Label(labeled) = stmt {
                if goto_targets.contains(&labeled.name.name) {
                    segments.push(&stmts[seg_start..i]);
                    label_to_segment.insert(labeled.name.name.clone(), segments.len() as u32);
                    seg_start = i;
                }
            }
        }
        segments.push(&stmts[seg_start..]);
        (segments, label_to_segment)
    }

    pub(crate) fn compile_block_with_goto_dispatch(
        &mut self,
        body: &ast::BlockStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        goto_targets: HashSet<String>,
    ) -> Result<(), Error> {
        let (segments, label_to_segment) =
            Self::split_goto_segments(&body.list, &goto_targets);
        let num_segments = segments.len();

        let target_local = locals.add_local("__goto_target", ValType::I32);

        let saved_target_local = self.goto_target_local.take();
        let saved_label_segments = std::mem::take(&mut self.goto_label_segments);
        let saved_segment_depth = self.goto_segment_depth;

        self.goto_target_local = Some(target_local);
        self.goto_label_segments = label_to_segment;

        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(target_local));

        out.push(Instruction::Block(BlockType::Empty)); // $exit
        out.push(Instruction::Loop(BlockType::Empty));   // $dispatch

        for _ in 0..num_segments {
            out.push(Instruction::Block(BlockType::Empty));
        }

        out.push(Instruction::LocalGet(target_local));
        let br_targets: Vec<u32> = (0..num_segments as u32).collect();
        let default_target = (num_segments - 1) as u32;
        out.push(Instruction::BrTable(
            br_targets.into(),
            default_target,
        ));

        out.push(Instruction::End); // close innermost block (segment 0 dispatch target)

        self.loop_depth
            .push((Some("__goto_dispatch".to_string()), 0, false, true));

        locals.push_scope();
        self.symbols.enter_scope();

        for k in 0..num_segments {
            self.goto_segment_depth = (num_segments - 1 - k) as u32;

            if let Some((_, depth, _, _)) = self.loop_depth.last_mut() {
                *depth = self.goto_segment_depth;
            }

            for stmt in segments[k] {
                self.compile_statement(stmt, out, locals, result_types)?;
            }

            if k < num_segments - 1 {
                out.push(Instruction::End); // close next segment's block
            }
        }

        out.push(Instruction::Br(1)); // exit past $exit block

        self.symbols.leave_scope();
        locals.pop_scope();

        self.loop_depth.pop();

        out.push(Instruction::End); // $dispatch loop
        out.push(Instruction::End); // $exit block

        self.goto_target_local = saved_target_local;
        self.goto_label_segments = saved_label_segments;
        self.goto_segment_depth = saved_segment_depth;

        Ok(())
    }

    fn calculate_goto_dispatch_depth(&self) -> Result<u32, Error> {
        let dispatch_idx = self
            .loop_depth
            .iter()
            .rposition(|(lbl, _, _, _)| lbl.as_deref() == Some("__goto_dispatch"))
            .ok_or_else(|| {
                Error::InternalError("goto outside dispatch context".to_string())
            })?;

        let innermost = self.loop_depth.len() - 1;
        let inner_extra = self.loop_depth[innermost].1;

        if innermost == dispatch_idx {
            return Ok(inner_extra);
        }

        let mut intermediate_depth: u32 = 0;

        let entry = &self.loop_depth[innermost];
        if entry.3 {
            intermediate_depth += 2 + entry.2 as u32;
        } else {
            intermediate_depth += 1;
        }

        for i in (dispatch_idx + 1..innermost).rev() {
            let entry = &self.loop_depth[i];
            if entry.3 {
                intermediate_depth += 2 + entry.2 as u32 + entry.1;
            } else {
                intermediate_depth += 1 + entry.1;
            }
        }

        let dispatch_depth = self.loop_depth[dispatch_idx].1;

        Ok(inner_extra + intermediate_depth + dispatch_depth)
    }

    pub(crate) fn compile_range_over_func(
        &mut self,
        range: &ast::RangeStmt,
        iter_info: &IterFuncInfo,
        out: &mut Vec<Instruction<'static>>,
        outer_locals: &mut LocalAlloc,
        _label: Option<String>,
    ) -> Result<(), Error> {
        let yield_param_types = iter_info.yield_param_types.clone();
        let iter_func_idx = iter_info.func_idx;

        // Build the yield closure as a separate WASM function.
        // Signature: (env_ptr: i32, [key: K, [value: V]]) -> i32  (bool)
        let func_idx = self.next_func_idx;

        let mut go_param_names: Vec<String> = Vec::new();
        let mut go_param_types: Vec<ValType> = Vec::new();

        // Key parameter
        if let Some(key_expr) = &range.key {
            if let ast::Expression::Ident(id) = key_expr {
                if id.name != "_" {
                    go_param_names.push(id.name.clone());
                } else {
                    go_param_names.push(format!("_yield_param_{}", go_param_names.len()));
                }
            } else {
                go_param_names.push(format!("_yield_param_{}", go_param_names.len()));
            }
            if !yield_param_types.is_empty() {
                go_param_types.push(yield_param_types[0]);
            }
        } else if !yield_param_types.is_empty() {
            go_param_names.push("_yield_k".to_string());
            go_param_types.push(yield_param_types[0]);
        }

        // Value parameter
        if let Some(val_expr) = &range.value {
            if let ast::Expression::Ident(id) = val_expr {
                if id.name != "_" {
                    go_param_names.push(id.name.clone());
                } else {
                    go_param_names.push(format!("_yield_param_{}", go_param_names.len()));
                }
            } else {
                go_param_names.push(format!("_yield_param_{}", go_param_names.len()));
            }
            if yield_param_types.len() > 1 {
                go_param_types.push(yield_param_types[1]);
            }
        } else if yield_param_types.len() > 1 {
            go_param_names.push("_yield_v".to_string());
            go_param_types.push(yield_param_types[1]);
        }

        let result_types: Vec<ValType> = vec![ValType::I32]; // bool return

        let mut full_param_types: Vec<ValType> = vec![ValType::I32]; // env_ptr
        full_param_types.extend_from_slice(&go_param_types);

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(full_param_types.clone(), result_types.clone());
        self.next_type_idx += 1;
        self.needs_func_table = true;

        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let closure_name = format!("__yield_closure_{}", func_idx);
        let wasm_params: Vec<(String, WasmType)> = go_param_names
            .iter()
            .zip(go_param_types.iter())
            .map(|(n, vt)| (n.clone(), match vt {
                ValType::I32 => WasmType::I32,
                ValType::I64 => WasmType::I64,
                ValType::F32 => WasmType::F32,
                ValType::F64 => WasmType::F64,
                _ => WasmType::I32,
            }))
            .collect();

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: closure_name,
            params: wasm_params,
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });

        // Build inner locals: env_ptr + yield params
        let mut inner_params: Vec<(String, ValType)> =
            vec![("__env_ptr".to_string(), ValType::I32)];
        for (name, vt) in go_param_names.iter().zip(go_param_types.iter()) {
            inner_params.push((name.clone(), *vt));
        }
        let mut inner_locals = LocalAlloc::new(inner_params);

        // Set up capture state
        let outer_snapshot = outer_locals.all_entries();
        self.closure_captures = Some(ClosureCaptureState {
            outer_locals: outer_snapshot,
            captures: Vec::new(),
            outer_closure_info: outer_locals.closure_info.clone(),
            outer_closure_env_captures: outer_locals.closure_env_captures.clone(),
        });

        let saved_named_returns = std::mem::replace(&mut self.named_returns, vec![]);
        let saved_result_types = std::mem::replace(&mut self.current_result_types, result_types.clone());
        let saved_result_go_types = std::mem::replace(&mut self.current_result_go_types, vec![]);
        let saved_stack_frame = self.current_stack_frame.take();
        let saved_stack_alloc_target = self.stack_alloc_target.take();
        let saved_loop_depth = std::mem::take(&mut self.loop_depth);

        // Push a sentinel so break/continue inside the body emit return 0/1
        // Using is_loop=true with a special label we can detect
        self.loop_depth.push((Some("__range_over_func".to_string()), 0, false, true));

        let mut body: Vec<Instruction<'static>> = Vec::new();
        self.deferred_calls.push(Vec::new());

        self.compile_block(&range.body, &mut body, &mut inner_locals, &result_types)?;

        self.emit_deferred_calls(&mut body);
        self.deferred_calls.pop();

        // Default: return true (continue iterating)
        body.push(Instruction::I32Const(1));
        body.push(Instruction::Return);
        body.push(Instruction::End);

        self.loop_depth = saved_loop_depth;
        self.named_returns = saved_named_returns;
        self.current_result_types = saved_result_types;
        self.current_result_go_types = saved_result_go_types;
        self.current_stack_frame = saved_stack_frame;
        self.stack_alloc_target = saved_stack_alloc_target;

        // Extract captures
        let captures = if let Some(cc) = self.closure_captures.take() {
            cc.captures
        } else {
            Vec::new()
        };

        let mut func = Function::new(inner_locals.local_types());
        for instr in &body {
            func.instruction(instr);
        }
        self.pending_closures.push((func_idx, func));

        // Allocate env and store captures in the outer function
        let env_local = if let Some(last) = captures.last() {
            let env_size = (last.env_offset + val_type_byte_size(last.val_type)) as i32;
            out.push(Instruction::I32Const(env_size));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let env_local = outer_locals.add_local("__yield_env_ptr", ValType::I32);
            out.push(Instruction::LocalSet(env_local));

            for cap in &captures {
                out.push(Instruction::LocalGet(env_local));
                out.push(Instruction::LocalGet(cap.outer_local_idx));
                let (_, align) = Self::elem_size_and_align(cap.val_type);
                Self::emit_typed_store(cap.val_type, cap.env_offset as u64, align, out);
            }

            env_local
        } else {
            let env_local = outer_locals.add_local("__yield_env_ptr", ValType::I32);
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(env_local));
            env_local
        };

        // Call the iterator function: iter(func_table_idx, env_ptr)
        out.push(Instruction::I32Const(func_idx as i32));
        out.push(Instruction::LocalGet(env_local));
        out.push(Instruction::Call(iter_func_idx));

        // Writeback captures after iterator returns
        for cap in &captures {
            if let Some(outer_local) = outer_locals.find(&cap.name) {
                out.push(Instruction::LocalGet(env_local));
                let (_, align) = Self::elem_size_and_align(cap.val_type);
                Self::emit_typed_load(cap.val_type, cap.env_offset as u64, align, out);
                out.push(Instruction::LocalSet(outer_local));
            }
        }

        Ok(())
    }
}
