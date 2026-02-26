use super::*;

impl WasmCompiler {
    pub(crate) fn compile_func_decl(&mut self, decl: &ast::FuncDecl, defer_code: bool) -> Result<(), Error> {
        let name = &decl.name.name;

        // Generic functions: store for later monomorphization instead of compiling now
        if !decl.typ.typ_params.list.is_empty() {
            self.generic_funcs.insert(name.clone(), decl.clone());
            return Ok(());
        }

        // Bodyless functions are native imports, already handled in forward_declare_functions
        if decl.body.is_none() && decl.recv.is_none() {
            return Ok(());
        }

        // Methods on generic types: store for later monomorphization
        if let Some(recv) = &decl.recv {
            if self.is_generic_recv(recv) {
                let recv_type_name = self.extract_recv_type_name(recv);
                if let Some(type_name) = recv_type_name {
                    let key = format!("{}.{}", type_name, name);
                    self.generic_funcs.insert(key, decl.clone());
                    return Ok(());
                }
            }
        }

        let is_method = decl.recv.is_some();

        let recv_type_name = if let Some(recv) = &decl.recv {
            self.extract_recv_type_name(recv)
                .map(|rtn| self.qualify_pkg_name(&rtn))
        } else {
            None
        };

        let internal_name = if let Some(ref rtn) = recv_type_name {
            format!("{}.{}", rtn, name)
        } else {
            self.qualify_pkg_name(name)
        };

        let is_forward_declared = self.forward_declared.contains(&internal_name);

        let is_exported =
            name.chars().next().map_or(false, |c| c.is_uppercase())
            && !is_method
            && self.current_package.is_none();

        let mut param_types: Vec<ValType> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();

        if let Some(recv) = &decl.recv {
            for field in &recv.list {
                let recv_vt = match &field.typ {
                    ast::Expression::TypePointer(_) => ValType::I32,
                    ast::Expression::Ident(id) => {
                        let resolved_struct = self.resolve_struct_in_pkg(&id.name);
                        if self.struct_defs.contains_key(&resolved_struct) {
                            ValType::I32
                        } else {
                            let resolved = self.resolve_type_name(&id.name);
                            match resolved {
                                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                                "float64" => ValType::F64,
                                "float32" => ValType::F32,
                                "int32" | "uint32" | "byte" | "bool" | "rune" => ValType::I32,
                                _ => ValType::I32,
                            }
                        }
                    }
                    _ => ValType::I32,
                };
                param_types.push(recv_vt);
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                param_names.push(recv_name.to_string());
            }
        }

        let mut is_variadic = false;
        let mut variadic_elem_vt: Option<ValType> = None;
        let mut variadic_param_name: Option<String> = None;

        for field in &decl.typ.params.list {
            // Detect variadic parameter: ...T
            if let ast::Expression::Ellipsis(ellipsis) = &field.typ {
                is_variadic = true;
                let elem_vt = if let Some(ref elt) = ellipsis.elt {
                    Self::infer_array_elem_vt(elt)
                } else {
                    ValType::I64
                };
                variadic_elem_vt = Some(elem_vt);
                param_types.push(ValType::I32); // slice header pointer
                let vname = field.name.first().map_or(
                    format!("_param{}", param_names.len()),
                    |id| id.name.clone()
                );
                variadic_param_name = Some(vname.clone());
                param_names.push(vname);
                continue;
            }

            let is_func_param = matches!(&field.typ, ast::Expression::TypeFunction(_));

            if is_func_param {
                for ident in field.name.iter() {
                    param_types.push(ValType::I32); // table index
                    param_names.push(ident.name.clone());
                    param_types.push(ValType::I32); // env_ptr
                    param_names.push(format!("{}__env_ptr", ident.name));
                }
                continue;
            }

            let is_iface_param = match &field.typ {
                ast::Expression::Ident(id) => {
                    self.iface_defs.contains_key(&id.name) || id.name == "error" || id.name == "any"
                }
                ast::Expression::TypeInterface(_) => true,
                _ => false,
            };

            let is_string_param = matches!(&field.typ, ast::Expression::Ident(id) if id.name == "string");

            let field_wasm_types = self.field_to_wasm_types(field);
            if field.name.is_empty() {
                for wt in &field_wasm_types {
                    param_types.push(wt.to_val_type());
                    param_names.push(format!("_param{}", param_names.len()));
                }
            } else {
                for ident in field.name.iter() {
                    if is_string_param && self.gc_builtin_types.go_string.is_none() {
                        param_types.push(ValType::I32);
                        param_names.push(ident.name.clone());
                        param_types.push(ValType::I32);
                        param_names.push(format!("{}__str_len", ident.name));
                    } else {
                        if !field_wasm_types.is_empty() {
                            param_types.push(field_wasm_types[0].to_val_type());
                        } else {
                            param_types.push(ValType::I32);
                        }
                        param_names.push(ident.name.clone());
                    }
                    if is_iface_param {
                        param_types.push(ValType::I32);
                        param_names.push(format!("{}__type_id", ident.name));
                    }
                }
            }
        }

        let mut result_types: Vec<ValType> = Vec::new();
        let mut result_go_types: Vec<String> = Vec::new();
        for field in &decl.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            let go_type_name = self.expr_type_name(&field.typ);
            let count = if field.name.len() > 1 { field.name.len() } else { 1 };
            for _ in 0..count {
                for wt in &field_wasm_types {
                    result_types.push(wt.to_val_type());
                    result_go_types.push(go_type_name.clone());
                }
            }
        }

        let (type_idx, func_idx) = if is_forward_declared {
            // Already registered in type/function sections during forward declaration
            let fi = self.functions.iter().find(|f| f.name == internal_name).ok_or_else(|| {
                Error::InternalError(format!("forward-declared function '{}' not found", internal_name))
            })?;
            (fi.type_idx, fi.wasm_func_idx)
        } else {
            let type_idx = self.next_type_idx;
            self.type_section
                .ty()
                .function(param_types.clone(), result_types.clone());
            self.next_type_idx += 1;

            let func_idx = self.next_func_idx;
            self.function_section.function(type_idx);
            self.next_func_idx += 1;

            if is_exported {
                self.export_section
                    .export(name, ExportKind::Func, func_idx);
            }

            (type_idx, func_idx)
        };

        let is_init = name == "init"
            && !is_method
            && decl.typ.params.list.is_empty()
            && decl.typ.result.list.is_empty();

        if is_init {
            self.init_func_indices.push(func_idx);
        }

        let wasm_params: Vec<(String, WasmType)> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, vt)| {
                (n.clone(), Self::val_type_to_wasm_type(*vt))
            })
            .collect();

        let wasm_results: Vec<WasmType> = result_types
            .iter()
            .map(|vt| Self::val_type_to_wasm_type(*vt))
            .collect();

        let recv_count = if decl.recv.is_some() { 1 } else { 0 };
        let mut iface_param_indices: Vec<usize> = Vec::new();
        {
            let mut go_arg_idx: usize = 0;
            for field in &decl.typ.params.list {
                let is_iface_field = match &field.typ {
                    ast::Expression::Ident(id) => {
                        self.iface_defs.contains_key(&id.name) || id.name == "error" || id.name == "any"
                    }
                    ast::Expression::TypeInterface(_) => true,
                    _ => false,
                };
                let name_count = if field.name.is_empty() { 1 } else { field.name.len() };
                for _ in 0..name_count {
                    if is_iface_field {
                        iface_param_indices.push(go_arg_idx);
                    }
                    go_arg_idx += 1;
                }
            }
        }
        let _ = recv_count;

        if self.forward_declared.contains(&internal_name) {
            if let Some(fi) = self.functions.iter_mut().find(|f| f.name == internal_name) {
                fi.wasm_func_idx = func_idx;
                fi.type_idx = type_idx;
            }
        } else {
            self.functions.push(FuncInfo {
                wasm_func_idx: func_idx,
                type_idx,
                name: internal_name,
                params: wasm_params,
                results: wasm_results,
                result_go_types: result_go_types.clone(),
                is_exported,
                recv_type: recv_type_name.clone(),
                is_variadic,
                variadic_elem_vt,
                iface_param_indices,
            });
        }

        let param_entries: Vec<(String, ValType)> = param_names
            .iter()
            .zip(param_types.iter())
            .map(|(n, vt)| (n.clone(), *vt))
            .collect();
        let mut locals = LocalAlloc::new(param_entries);

        self.symbols.new_context(false);

        // #region agent log
        if func_idx >= 248 && func_idx <= 290 {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                let _ = writeln!(f, r#"{{"hypothesisId":"A","location":"functions.rs:compile_func_decl","message":"compiling function","data":{{"func_idx":{},"name":"{}","pkg":"{}","param_count":{},"result_count":{}}},"timestamp":{}}}"#,
                    func_idx, decl.name.name, self.current_package.as_deref().unwrap_or(""), param_types.len(), result_types.len(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
            }
        }
        // #endregion

        // Track variadic parameter as slice
        if let Some(ref vp_name) = variadic_param_name {
            locals.set_var_struct_type(vp_name, "__slice");
            self.define_var(vp_name, DefineType::Slice(Box::new(DefineType::Null)));
            if let Some(evtype) = variadic_elem_vt {
                locals.slice_elem_types.insert(vp_name.clone(), evtype);
            }
        }

        // Track struct type for receiver
        if let Some(recv) = &decl.recv {
            for field in &recv.list {
                let recv_name = field.name.first().map_or("self", |id| &id.name);
                if let Some(ref rtn) = recv_type_name {
                    locals.set_var_struct_type(recv_name, rtn);
                    self.define_var(recv_name, self.go_type_name_to_define_type(rtn));
                }
            }
        }

        // Track struct types and signedness for parameters
        for field in &decl.typ.params.list {
            let param_dt = self.expr_to_define_type(&field.typ);
            for name_ident in &field.name {
                if let Some(ref dt) = param_dt {
                    self.define_var(&name_ident.name, dt.clone());
                }
            }

            if let ast::Expression::TypeSlice(slice_type) = &field.typ {
                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                for name_ident in &field.name {
                    locals.set_var_struct_type(&name_ident.name, "__slice");
                    locals.slice_elem_types.insert(name_ident.name.clone(), elem_vt);
                    if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                        let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                        locals.nested_slice_inner_elem_types.insert(name_ident.name.clone(), inner_vt);
                    }
                    if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                        let resolved_elem = self.resolve_struct_in_pkg(&el_id.name);
                        if self.struct_defs.contains_key(&resolved_elem) {
                            locals.slice_elem_struct_types.insert(name_ident.name.clone(), resolved_elem);
                        }
                    }
                }
            }
            if let ast::Expression::Ident(type_ident) = &field.typ {
                let resolved_struct = self.resolve_struct_in_pkg(&type_ident.name);
                if self.struct_defs.contains_key(&resolved_struct) {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, &resolved_struct);
                    }
                }
                if type_ident.name == "Context" {
                    for name_ident in &field.name {
                        locals.set_var_struct_type(&name_ident.name, "__context");
                    }
                }
                if type_ident.name == "string" {
                    for name_ident in &field.name {
                        if self.gc_builtin_types.go_string.is_some() {
                            locals.set_var_struct_type(&name_ident.name, "__string");
                            let ref_idx = locals.find(&name_ident.name).unwrap_or(0);
                            locals.gc_string_locals.insert(name_ident.name.clone(), ref_idx);
                        } else {
                            locals.set_var_struct_type(&name_ident.name, "__string");
                            let ptr_idx = locals.find(&name_ident.name).unwrap_or(0);
                            let len_name = format!("{}__str_len", name_ident.name);
                            let len_idx = locals.find(&len_name).unwrap_or_else(|| {
                                locals.add_local(&len_name, ValType::I32)
                            });
                            locals.string_locals.insert(name_ident.name.clone(), (ptr_idx, len_idx));
                        }
                    }
                }
                if Self::is_unsigned_type_name(&type_ident.name) {
                    for name_ident in &field.name {
                        locals.unsigned_vars.insert(name_ident.name.clone());
                    }
                }
                if self.iface_defs.contains_key(&type_ident.name)
                    || type_ident.name == "error"
                    || type_ident.name == "any"
                {
                    for name_ident in &field.name {
                        let iface_tag = format!("__iface_{}", type_ident.name);
                        locals.set_var_struct_type(&name_ident.name, &iface_tag);
                        let tid_param_name = format!("{}__type_id", name_ident.name);
                        let tid_local = locals.find(&tid_param_name).unwrap_or_else(|| {
                            locals.add_local(&tid_param_name, ValType::I32)
                        });
                        locals.iface_type_id_locals.insert(name_ident.name.clone(), tid_local);
                    }
                }
            }
            if let ast::Expression::TypePointer(ptr) = &field.typ {
                if let ast::Expression::Ident(type_ident) = ptr.typ.as_ref() {
                    let resolved_ptr_struct = self.resolve_struct_in_pkg(&type_ident.name);
                    if self.struct_defs.contains_key(&resolved_ptr_struct) {
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, &resolved_ptr_struct);
                            locals.pointer_to_struct_vars.insert(name_ident.name.clone());
                        }
                    } else {
                        let ptr_tag = match type_ident.name.as_str() {
                            "int" | "int64" | "uint" | "uint64" => "__ptr_i64",
                            "float32" => "__ptr_f32",
                            "float64" => "__ptr_f64",
                            _ => "__ptr_i32",
                        };
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, ptr_tag);
                        }
                    }
                }
            }
            if let ast::Expression::Selector(sel) = &field.typ {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    if pkg.name == "context" && sel.sel.name == "Context" {
                        for name_ident in &field.name {
                            locals.set_var_struct_type(&name_ident.name, "__context");
                        }
                    }
                }
            }
            if let ast::Expression::TypeFunction(ft) = &field.typ {
                for name_ident in &field.name {
                    let func_idx_local = locals.find(&name_ident.name).unwrap_or(0);
                    let env_ptr_name = format!("{}__env_ptr", name_ident.name);
                    let env_ptr_local = locals.find(&env_ptr_name).unwrap_or(0);

                    let mut call_param_types: Vec<ValType> = vec![ValType::I32]; // env_ptr hidden
                    for p in &ft.params.list {
                        let wts = self.field_to_wasm_types(p);
                        for wt in &wts {
                            call_param_types.push(wt.to_val_type());
                        }
                    }
                    let mut call_result_types: Vec<ValType> = Vec::new();
                    for r in &ft.result.list {
                        let wts = self.field_to_wasm_types(r);
                        for wt in &wts {
                            call_result_types.push(wt.to_val_type());
                        }
                    }
                    let call_type_idx = self.next_type_idx;
                    self.type_section.ty().function(
                        call_param_types,
                        call_result_types.clone(),
                    );
                    self.next_type_idx += 1;
                    self.needs_func_table = true;

                    locals.func_typed_params.insert(name_ident.name.clone(), FuncTypedParamInfo {
                        func_idx_local,
                        env_ptr_local,
                        call_type_idx,
                        result_count: call_result_types.len(),
                    });
                }
            }
        }

        // Collect named return variables
        let mut named_returns: Vec<(String, ValType)> = Vec::new();
        let mut named_return_tracking: Vec<(String, String, u32)> = Vec::new();
        for field in &decl.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            let go_type_name = self.expr_type_name(&field.typ);
            for (i, ident) in field.name.iter().enumerate() {
                let vt = if i < field_wasm_types.len() {
                    field_wasm_types[i].to_val_type()
                } else if !field_wasm_types.is_empty() {
                    field_wasm_types[0].to_val_type()
                } else {
                    ValType::I64
                };
                let local_idx = locals.add_local(&ident.name, vt);
                named_return_tracking.push((ident.name.clone(), go_type_name.clone(), local_idx));
                named_returns.push((ident.name.clone(), vt));
            }
        }

        self.deferred_calls.push(Vec::new());

        let mut func_body: Vec<Instruction<'static>> = Vec::new();

        let stack_frame = if let Some(body) = &decl.body {
            let escaping = Self::analyze_function_escapes(body);
            let mut sf = self.compute_stack_frame(decl, &escaping);
            // #region agent log
            if decl.name.name == "genericFtoa" || decl.name.name == "bigFtoa" {
                use std::io::Write;
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                    let frame_locals: Vec<String> = sf.locals.iter().map(|sl| format!("{}:{}:{}", sl.name, sl.offset, sl.size)).collect();
                    let esc_list: Vec<String> = escaping.iter().cloned().collect();
                    let ds_keys: Vec<String> = self.struct_defs.keys().filter(|k| k.contains("decimal")).cloned().collect();
                    let _ = writeln!(f, r#"{{"hypothesisId":"H","location":"functions.rs:stack_frame","message":"stack frame info","data":{{"func":"{}","total_size":{},"frame_locals":"{}","escaping":"{}","decimal_struct_keys":"{}"}},"timestamp":{}}}"#,
                        decl.name.name, sf.total_size, frame_locals.join("|"), esc_list.join("|"), ds_keys.join("|"),
                        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
                }
            }
            // #endregion
            if sf.total_size > 0 {
                let fb = locals.add_local("__frame_base", ValType::I32);
                sf.frame_base_local = Some(fb);

                func_body.push(Instruction::GlobalGet(self.stack_ptr_global));
                func_body.push(Instruction::LocalSet(fb));

                // Check for overflow before bumping the stack pointer
                func_body.push(Instruction::LocalGet(fb));
                func_body.push(Instruction::I32Const(sf.total_size as i32));
                func_body.push(Instruction::I32Add);
                func_body.push(Instruction::I32Const(Self::TYPE_DESC_BASE));
                func_body.push(Instruction::I32GeU);
                func_body.push(Instruction::If(BlockType::Empty));
                func_body.push(Instruction::Call(self.oom_func_idx));
                func_body.push(Instruction::Unreachable);
                func_body.push(Instruction::End);

                func_body.push(Instruction::LocalGet(fb));
                func_body.push(Instruction::I32Const(sf.total_size as i32));
                func_body.push(Instruction::I32Add);
                func_body.push(Instruction::GlobalSet(self.stack_ptr_global));
            }
            sf
        } else {
            StackFrameInfo::default()
        };

        if let Some(body) = &decl.body {
            let addr_taken_set = Self::analyze_address_taken_vars(body);
            for sl in &stack_frame.locals {
                if addr_taken_set.contains(&sl.name) {
                    let vt = {
                        let mut found_vt: Option<ValType> = None;
                        for field in &decl.typ.params.list {
                            for name in &field.name {
                                if name.name == sl.name {
                                    if let ast::Expression::Ident(ti) = &field.typ {
                                        found_vt = Some(match ti.name.as_str() {
                                            "float32" => ValType::F32,
                                            "float64" => ValType::F64,
                                            "int32" | "uint32" | "int16" | "uint16" |
                                            "int8" | "uint8" | "byte" | "bool" => ValType::I32,
                                            _ => ValType::I64,
                                        });
                                    }
                                }
                            }
                        }
                        if found_vt.is_none() {
                            if let Some(body) = &decl.body {
                                for stmt in &body.list {
                                    if let ast::Statement::Declaration(ast::DeclStmt::Variable(vd)) = stmt {
                                        for spec in &vd.specs {
                                            for nm in &spec.name {
                                                if nm.name == sl.name {
                                                    if let Some(ref typ) = spec.typ {
                                                        if let ast::Expression::Ident(ti) = typ {
                                                            found_vt = Some(match ti.name.as_str() {
                                                                "float32" => ValType::F32,
                                                                "float64" => ValType::F64,
                                                                "int32" | "uint32" | "int16" | "uint16" |
                                                                "int8" | "uint8" | "byte" | "bool" => ValType::I32,
                                                                _ => {
                                                                    if Self::struct_defs_contains(&ti.name, &self.struct_defs) {
                                                                        ValType::I32
                                                                    } else {
                                                                        ValType::I64
                                                                    }
                                                                }
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        found_vt.unwrap_or(ValType::I64)
                    };
                    locals.memory_backed_vars.insert(sl.name.clone(), (sl.offset, vt));
                }
            }
        }

        if let Some(fb) = stack_frame.frame_base_local {
            for (name, &(offset, vt)) in &locals.memory_backed_vars {
                if let Some(param_idx) = locals.find(name) {
                    func_body.push(Instruction::LocalGet(fb));
                    if offset > 0 {
                        func_body.push(Instruction::I32Const(offset as i32));
                        func_body.push(Instruction::I32Add);
                    }
                    func_body.push(Instruction::LocalGet(param_idx));
                    let (_, align) = Self::elem_size_and_align(vt);
                    Self::emit_typed_store(vt, 0, align, &mut func_body);
                }
            }
        }

        let saved_stack_frame = std::mem::replace(
            &mut self.current_stack_frame,
            if stack_frame.total_size > 0 {
                Some(stack_frame.clone())
            } else {
                None
            },
        );
        let saved_stack_alloc_target = self.stack_alloc_target.take();

        if let Some(body) = &decl.body {
            let saved_constants = self.constants.clone();
            for (name, go_type, local_idx) in &named_return_tracking {
                self.track_local_var_type(name, go_type, *local_idx, &mut locals);
            }
            self.named_returns = named_returns.clone();
            self.current_result_types = result_types.clone();
            self.current_result_go_types = result_go_types.clone();
            let goto_targets = Self::scan_goto_targets(body);
            if goto_targets.is_empty() {
                self.compile_block(&body, &mut func_body, &mut locals, &result_types)?;
            } else {
                self.compile_block_with_goto_dispatch(
                    body, &mut func_body, &mut locals, &result_types, goto_targets,
                )?;
            }
            self.named_returns = Vec::new();
            self.current_result_types = Vec::new();
            self.current_result_go_types = Vec::new();
            self.constants = saved_constants;
        }

        // #region agent log
        if decl.name.name == "genericFtoa" || decl.name.name == "bigFtoa" {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/Users/petardambovaliev/GolandProjects/govm/.cursor/debug.log") {
                let locals_info: Vec<String> = locals.locals.iter().enumerate().map(|(i, (name, vt, _scope))| {
                    format!("{}:{}:{:?}", locals.param_count() as usize + i, name, vt)
                }).collect();
                let _ = writeln!(f, r#"{{"hypothesisId":"G","location":"functions.rs:compile_func_decl","message":"locals dump","data":{{"func":"{}","func_idx":{},"param_count":{},"locals_count":{},"locals":"{}"}},"timestamp":{}}}"#,
                    decl.name.name, func_idx, locals.param_count(), locals.locals.len(),
                    locals_info.join("|"),
                    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
            }
        }
        // #endregion

        self.current_stack_frame = saved_stack_frame;
        self.stack_alloc_target = saved_stack_alloc_target;

        self.emit_deferred_calls(&mut func_body);
        self.deferred_calls.pop();

        if stack_frame.total_size > 0 {
            if let Some(fb) = stack_frame.frame_base_local {
                let mut patched = Vec::with_capacity(func_body.len());
                for instr in func_body {
                    if matches!(instr, Instruction::Return) {
                        patched.push(Instruction::LocalGet(fb));
                        patched.push(Instruction::GlobalSet(self.stack_ptr_global));
                    }
                    patched.push(instr);
                }
                func_body = patched;
            }
        }

        let body_always_returns = decl
            .body
            .as_ref()
            .map_or(false, |b| Self::block_always_returns(&b.list));

        if body_always_returns {
            if !result_types.is_empty()
                && func_body
                    .last()
                    .map_or(true, |i| !matches!(i, Instruction::Return))
            {
                func_body.push(Instruction::Unreachable);
            }
        } else if result_types.is_empty()
            || func_body
                .last()
                .map_or(true, |i| !matches!(i, Instruction::Return))
        {
            if !named_returns.is_empty() {
                for (name, _vt) in &named_returns {
                    if let Some(idx) = locals.find(name) {
                        func_body.push(Instruction::LocalGet(idx));
                    }
                }
            } else if !result_types.is_empty() {
                return Err(Error::SyntaxError(format!(
                    "missing return at end of function '{}'",
                    decl.name.name
                )));
            }
        }

        if stack_frame.total_size > 0 {
            if let Some(fb) = stack_frame.frame_base_local {
                func_body.push(Instruction::LocalGet(fb));
                func_body.push(Instruction::GlobalSet(self.stack_ptr_global));
            }
        }

        func_body.push(Instruction::End);

        let mut func = Function::new(locals.local_types());
        for instr in &func_body {
            func.instruction(instr);
        }

        if defer_code {
            self.pending_closures.push((func_idx, func));
        } else {
            self.code_buffer.push((func_idx, func));
            for (closure_idx, closure_func) in self.pending_closures.drain(..) {
                self.code_buffer.push((closure_idx, closure_func));
            }
        }

        self.symbols.leave_context();

        Ok(())
    }

    pub(crate) fn compile_func_lit(
        &mut self,
        func_lit: &ast::FuncLit,
        out: &mut Vec<Instruction<'static>>,
        outer_locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        let func_idx = self.next_func_idx;

        let mut go_param_names: Vec<String> = Vec::new();
        let mut go_param_types: Vec<ValType> = Vec::new();
        for field in &func_lit.typ.params.list {
            let wts = self.field_to_wasm_types(field);
            if field.name.is_empty() {
                for wt in &wts {
                    go_param_types.push(wt.to_val_type());
                    go_param_names.push(format!("_param{}", go_param_names.len()));
                }
            } else {
                for (j, ident) in field.name.iter().enumerate() {
                    if j < wts.len() {
                        go_param_types.push(wts[j].to_val_type());
                    } else if !wts.is_empty() {
                        go_param_types.push(wts[0].to_val_type());
                    }
                    go_param_names.push(ident.name.clone());
                }
            }
        }

        let mut result_types: Vec<ValType> = Vec::new();
        for field in &func_lit.typ.result.list {
            let wts = self.field_to_wasm_types(field);
            for wt in wts {
                result_types.push(wt.to_val_type());
            }
        }

        // Always include env_ptr as hidden first parameter
        let mut full_param_types: Vec<ValType> = vec![ValType::I32];
        full_param_types.extend_from_slice(&go_param_types);

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(full_param_types.clone(), result_types.clone());
        self.next_type_idx += 1;

        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let closure_name = format!("__closure_{}", func_idx);
        let wasm_params: Vec<(String, WasmType)> = go_param_names
            .iter()
            .zip(go_param_types.iter())
            .map(|(n, vt)| {
                (
                    n.clone(),
                    match vt {
                        ValType::I32 => WasmType::I32,
                        ValType::I64 => WasmType::I64,
                        ValType::F32 => WasmType::F32,
                        ValType::F64 => WasmType::F64,
                        _ => WasmType::I32,
                    },
                )
            })
            .collect();
        let wasm_results: Vec<WasmType> = result_types
            .iter()
            .map(|vt| match vt {
                ValType::I32 => WasmType::I32,
                ValType::I64 => WasmType::I64,
                ValType::F32 => WasmType::F32,
                ValType::F64 => WasmType::F64,
                _ => WasmType::I32,
            })
            .collect();
        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: closure_name,
            params: wasm_params,
            results: wasm_results,
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });

        // Build inner locals: env_ptr + go params
        let mut inner_params: Vec<(String, ValType)> =
            vec![("__env_ptr".to_string(), ValType::I32)];
        for (name, vt) in go_param_names.iter().zip(go_param_types.iter()) {
            inner_params.push((name.clone(), *vt));
        }
        let mut inner_locals = LocalAlloc::new(inner_params);

        // Collect named return variables (same logic as compile_func_decl)
        let mut named_returns: Vec<(String, ValType)> = Vec::new();
        for field in &func_lit.typ.result.list {
            let field_wasm_types = self.field_to_wasm_types(field);
            for (i, ident) in field.name.iter().enumerate() {
                let vt = if i < field_wasm_types.len() {
                    field_wasm_types[i].to_val_type()
                } else if !field_wasm_types.is_empty() {
                    field_wasm_types[0].to_val_type()
                } else {
                    ValType::I64
                };
                let _local_idx = inner_locals.add_local(&ident.name, vt);
                named_returns.push((ident.name.clone(), vt));
            }
        }

        // Set up capture state
        let outer_snapshot = outer_locals.all_entries();
        self.closure_captures = Some(ClosureCaptureState {
            outer_locals: outer_snapshot,
            captures: Vec::new(),
            outer_closure_info: outer_locals.closure_info.clone(),
            outer_closure_env_captures: outer_locals.closure_env_captures.clone(),
        });

        let saved_named_returns = std::mem::replace(&mut self.named_returns, named_returns.clone());
        let saved_result_types = std::mem::replace(&mut self.current_result_types, result_types.clone());
        let saved_result_go_types = std::mem::replace(&mut self.current_result_go_types, vec![]);
        let saved_stack_frame = self.current_stack_frame.take();
        let saved_stack_alloc_target = self.stack_alloc_target.take();

        let saved_constants = self.constants.clone();
        self.symbols.new_context(true);
        let mut body: Vec<Instruction<'static>> = Vec::new();
        self.deferred_calls.push(Vec::new());
        let goto_targets = Self::scan_goto_targets(&func_lit.body);
        if goto_targets.is_empty() {
            self.compile_block(&func_lit.body, &mut body, &mut inner_locals, &result_types)?;
        } else {
            self.compile_block_with_goto_dispatch(
                &func_lit.body, &mut body, &mut inner_locals, &result_types, goto_targets,
            )?;
        }
        self.emit_deferred_calls(&mut body);
        self.deferred_calls.pop();
        self.symbols.leave_context();

        self.constants = saved_constants;
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

        self.last_closure_captures = captures.clone();

        // In outer function: allocate env and store captures
        if let Some(last) = captures.last() {
            let env_size: i32 =
                (last.env_offset + val_type_byte_size(last.val_type)) as i32;
            out.push(Instruction::I32Const(env_size));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let env_local = outer_locals.add_local("__env_ptr_outer", ValType::I32);
            out.push(Instruction::LocalSet(env_local));

            for cap in &captures {
                out.push(Instruction::LocalGet(env_local));
                out.push(Instruction::LocalGet(cap.outer_local_idx));
                match cap.val_type {
                    ValType::I64 => out.push(Instruction::I64Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F64 => out.push(Instruction::F64Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 3,
                        memory_index: 0,
                    })),
                    ValType::F32 => out.push(Instruction::F32Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                    _ => out.push(Instruction::I32Store(MemArg {
                        offset: cap.env_offset as u64,
                        align: 2,
                        memory_index: 0,
                    })),
                }
            }

            self.last_closure_env = Some(env_local);
        } else {
            self.last_closure_env = None;
        }

        self.last_closure_func_idx = Some(func_idx);

        // Termination analysis (mirrors compile_func_decl logic)
        let body_always_returns = Self::block_always_returns(&func_lit.body.list);

        if body_always_returns {
            if !result_types.is_empty()
                && body
                    .last()
                    .map_or(true, |i| !matches!(i, Instruction::Return))
            {
                body.push(Instruction::Unreachable);
            }
        } else if result_types.is_empty()
            || body
                .last()
                .map_or(true, |i| !matches!(i, Instruction::Return))
        {
            if !named_returns.is_empty() {
                for (name, _vt) in &named_returns {
                    if let Some(idx) = inner_locals.find(name) {
                        body.push(Instruction::LocalGet(idx));
                    }
                }
            } else if !result_types.is_empty() {
                return Err(Error::SyntaxError(
                    "missing return in function literal".to_string(),
                ));
            }
        }
        body.push(Instruction::End);

        let mut func = Function::new(inner_locals.local_types());
        for instr in &body {
            func.instruction(instr);
        }
        self.pending_closures.push((func_idx, func));

        // Push func_idx as the closure value
        out.push(Instruction::I32Const(func_idx as i32));

        Ok(GoType::Func)
    }
}
