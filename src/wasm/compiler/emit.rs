use super::*;

impl WasmCompiler {
    pub(crate) fn prescan_stdlib_package(&mut self, pkg: &str, sources: &[&str]) -> Result<(), Error> {
        let short = Self::pkg_short_name(pkg);
        if self.compiled_packages.contains(short) {
            return Ok(());
        }
        self.compiled_packages.insert(short.to_string());

        let mut files = Vec::new();
        for source in sources {
            let file = crate::parser::parse_source(source)
                .map_err(|e| Error::SyntaxError(e.to_string()))?;
            files.push(file);
        }

        // Recursively prescan dependencies
        for file in &files {
            for imp in &file.imports {
                let path = imp.path.value.trim_matches('"');
                if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(dep)) =
                    crate::wasm::stdlib::resolve_import(path)
                {
                    if let Some(dep_sources) = crate::wasm::stdlib::get_stdlib_sources(&dep) {
                        self.prescan_stdlib_package(&dep, dep_sources)?;
                    }
                }
            }
        }

        self.current_package = Some(short.to_string());

        for file in &files {
            self.prescan_type_declarations(file);
        }
        for file in &files {
            let sorted = Self::sort_declarations_by_deps(&file.decl);
            self.forward_declare_functions(&sorted);
        }

        self.current_package = None;
        Ok(())
    }

    pub(crate) fn compile_stdlib_bodies(&mut self, pkg: &str, sources: &[&str]) -> Result<(), Error> {
        let short = Self::pkg_short_name(pkg);
        let compiled_key = format!("__compiled_{}", short);
        if self.compiled_packages.contains(&compiled_key) {
            return Ok(());
        }
        self.compiled_packages.insert(compiled_key);

        let mut files = Vec::new();
        for source in sources {
            let file = crate::parser::parse_source(source)
                .map_err(|e| Error::SyntaxError(e.to_string()))?;
            files.push(file);
        }

        // Recursively compile dependencies first
        for file in &files {
            for imp in &file.imports {
                let path = imp.path.value.trim_matches('"');
                if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(dep)) =
                    crate::wasm::stdlib::resolve_import(path)
                {
                    if let Some(dep_sources) = crate::wasm::stdlib::get_stdlib_sources(&dep) {
                        self.compile_stdlib_bodies(&dep, dep_sources)?;
                    }
                }
            }
        }

        self.current_package = Some(short.to_string());

        // Collect all declarations from all files
        let mut all_const_var_type: Vec<ast::Declaration> = Vec::new();
        let mut all_functions: Vec<ast::Declaration> = Vec::new();
        for file in &files {
            let sorted = Self::sort_declarations_by_deps(&file.decl);
            for decl in &sorted {
                match decl {
                    ast::Declaration::Const(_) | ast::Declaration::Variable(_) | ast::Declaration::Type(_) => {
                        all_const_var_type.push(decl.clone());
                    }
                    ast::Declaration::Function(_) => {
                        all_functions.push(decl.clone());
                    }
                    _ => {}
                }
            }
        }

        // Pass 1: compile constants, variables, and types with multi-pass for forward refs
        {
            let mut resolved = vec![false; all_const_var_type.len()];
            loop {
                let mut progress = false;
                for (i, decl) in all_const_var_type.iter().enumerate() {
                    if resolved[i] { continue; }
                    match self.compile_declaration(decl) {
                        Ok(()) => {
                            resolved[i] = true;
                            progress = true;
                        }
                        Err(_) => {}
                    }
                }
                if resolved.iter().all(|&r| r) { break; }
                if !progress {
                    for (i, decl) in all_const_var_type.iter().enumerate() {
                        if !resolved[i] {
                            self.compile_declaration(decl)?;
                        }
                    }
                    break;
                }
            }
        }

        // Pass 2a: register generic function templates first
        for decl in &all_functions {
            if let ast::Declaration::Function(func_decl) = decl {
                if !func_decl.typ.typ_params.list.is_empty() {
                    self.generic_funcs.insert(func_decl.name.name.clone(), func_decl.clone());
                }
            }
        }

        // Pass 2b: compile function bodies
        for decl in &all_functions {
            self.compile_declaration(decl)?;
        }

        self.current_package = None;
        Ok(())
    }

    pub(crate) fn emit_memory(&mut self) {
        self.memory_section.memory(MemoryType {
            minimum: 2,
            maximum: Some(256),
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        self.export_section.export("memory", ExportKind::Memory, 0);
    }

    pub(crate) const HEAP_BASE: i32 = 65536;
    pub(crate) const STACK_BASE: i32 = 1024;
    pub(crate) const TYPE_DESC_BASE: i32 = 2048;
    pub(crate) const TYPE_DESC_ENTRY_SIZE: i32 = 12;

    pub(crate) fn emit_heap_globals(&mut self) {
        self.heap_ptr_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(Self::HEAP_BASE),
        );
        self.next_global_idx += 1;

        self.stack_ptr_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(Self::STACK_BASE),
        );
        self.next_global_idx += 1;

        self.panicking_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.panic_value_ptr_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.panic_value_len_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;

        self.map_iter_counter_global = self.next_global_idx;
        self.global_section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        self.next_global_idx += 1;
    }

    const NATIVE_FUNCTIONS: &'static [&'static str] = &[
        "nowUnixNano",
        "monotonicNano",
        "Float32bits",
        "Float32frombits",
        "Float64bits",
        "Float64frombits",
    ];

    pub(crate) const INLINED_NATIVE_FUNCTIONS: &'static [&'static str] = &[
        "Float32bits",
        "Float32frombits",
        "Float64bits",
        "Float64frombits",
    ];

    pub(crate) fn emit_host_imports(&mut self) {
        let pairs: &[(&str, &[ValType], &[ValType])] = &[
            ("ctx_log", &[ValType::I32, ValType::I32], &[]),
            ("ctx_query_id", &[ValType::I32], &[ValType::I32]),
            ("ctx_database", &[ValType::I32], &[ValType::I32]),
            ("ctx_schema", &[ValType::I32], &[ValType::I32]),
            ("ctx_user", &[ValType::I32], &[ValType::I32]),
            (
                "ctx_config",
                &[ValType::I32, ValType::I32, ValType::I32],
                &[ValType::I32],
            ),
            ("ctx_oom", &[], &[]),
        ];

        for (name, params, results) in pairs {
            let type_idx = self.next_type_idx;
            self.type_section.ty().function(
                params.iter().copied().collect::<Vec<_>>(),
                results.iter().copied().collect::<Vec<_>>(),
            );
            self.next_type_idx += 1;

            if *name == "ctx_oom" {
                self.oom_func_idx = self.next_func_idx;
            }

            self.import_section.import(
                "env",
                *name,
                wasm_encoder::EntityType::Function(type_idx),
            );
            self.next_func_idx += 1;
            self.import_func_count += 1;
        }
    }

    pub(crate) fn emit_native_imports(&mut self, file: &ast::File) -> Result<(), Error> {
        let mut all_files: Vec<ast::File> = Vec::new();
        let mut visited = HashSet::new();

        for imp in &file.imports {
            let path = imp.path.value.trim_matches('"');
            if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(pkg)) =
                crate::wasm::stdlib::resolve_import(path)
            {
                if let Some(sources) = crate::wasm::stdlib::get_stdlib_sources(&pkg) {
                    self.collect_stdlib_files_recursive(&pkg, sources, &mut all_files, &mut visited)?;
                }
            }
        }

        for parsed_file in &all_files {
            for decl in &parsed_file.decl {
                let func_decl = match decl {
                    ast::Declaration::Function(f) => f,
                    _ => continue,
                };
                if func_decl.body.is_some() {
                    continue;
                }
                if func_decl.recv.is_some() {
                    continue;
                }
                if !func_decl.typ.typ_params.list.is_empty() {
                    continue;
                }

                let name = &func_decl.name.name;

                if !Self::NATIVE_FUNCTIONS.contains(&name.as_str()) {
                    return Err(Error::SyntaxError(format!(
                        "function \"{}\" has no body and no native implementation is registered for it",
                        name
                    )));
                }

                let mut param_types: Vec<ValType> = Vec::new();
                for field in &func_decl.typ.params.list {
                    let field_wasm_types = self.field_to_wasm_types(field);
                    if field.name.is_empty() {
                        for wt in &field_wasm_types {
                            param_types.push(wt.to_val_type());
                        }
                    } else {
                        for _ in &field.name {
                            if !field_wasm_types.is_empty() {
                                param_types.push(field_wasm_types[0].to_val_type());
                            } else {
                                param_types.push(ValType::I32);
                            }
                        }
                    }
                }

                let mut result_types: Vec<ValType> = Vec::new();
                for field in &func_decl.typ.result.list {
                    let field_wasm_types = self.field_to_wasm_types(field);
                    for wt in &field_wasm_types {
                        result_types.push(wt.to_val_type());
                    }
                }

                let is_inlined = matches!(name.as_str(),
                    "Float64frombits" | "Float64bits" | "Float32frombits" | "Float32bits"
                );
                if is_inlined {
                    continue;
                }

                let type_idx = self.next_type_idx;
                self.type_section
                    .ty()
                    .function(param_types.clone(), result_types.clone());
                self.next_type_idx += 1;

                let func_idx = self.next_func_idx;
                self.import_section.import(
                    "env",
                    name.as_str(),
                    wasm_encoder::EntityType::Function(type_idx),
                );
                self.next_func_idx += 1;
                self.import_func_count += 1;

                self.wasm_imports.insert(name.clone(), func_idx);
            }
        }

        Ok(())
    }

    pub(crate) fn collect_stdlib_files_recursive(
        &self,
        pkg: &str,
        sources: &[&str],
        out: &mut Vec<ast::File>,
        visited: &mut HashSet<String>,
    ) -> Result<(), Error> {
        let short = Self::pkg_short_name(pkg).to_string();
        if !visited.insert(short) {
            return Ok(());
        }

        for source in sources {
            let file = crate::parser::parse_source(source)
                .map_err(|e| Error::SyntaxError(e.to_string()))?;

            for imp in &file.imports {
                let path = imp.path.value.trim_matches('"');
                if let Ok(crate::wasm::stdlib::ImportKind::Stdlib(dep)) =
                    crate::wasm::stdlib::resolve_import(path)
                {
                    if let Some(dep_sources) = crate::wasm::stdlib::get_stdlib_sources(&dep) {
                        self.collect_stdlib_files_recursive(&dep, dep_sources, out, visited)?;
                    }
                }
            }

            out.push(file);
        }

        Ok(())
    }

    pub(crate) fn emit_alloc_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(vec![ValType::I32], vec![ValType::I32]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![(1, ValType::I32)]);

        // Align size to 8: size = (size + 7) & ~7
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Const(7));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Const(!7));
        func.instruction(&Instruction::I32And);
        func.instruction(&Instruction::LocalSet(0));

        // Save current pointer
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalSet(1));

        // Bump heap pointer
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global));

        // Overflow check: if new_ptr < old_ptr, the add wrapped around
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::I32LtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::Call(self.oom_func_idx));
        func.instruction(&Instruction::Unreachable);
        func.instruction(&Instruction::End);

        // Bounds check: if new heap_ptr >= memory size in bytes, grow
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::MemorySize(0));
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32Shl);
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::If(BlockType::Empty));
        // Compute pages needed: (heap_ptr - current_bytes + 65535) >> 16
        func.instruction(&Instruction::GlobalGet(self.heap_ptr_global));
        func.instruction(&Instruction::MemorySize(0));
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32Shl);
        func.instruction(&Instruction::I32Sub);
        func.instruction(&Instruction::I32Const(65535));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Const(16));
        func.instruction(&Instruction::I32ShrU);
        func.instruction(&Instruction::MemoryGrow(0));
        func.instruction(&Instruction::I32Const(-1));
        func.instruction(&Instruction::I32Eq);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::Call(self.oom_func_idx));
        func.instruction(&Instruction::Unreachable);
        func.instruction(&Instruction::End);
        func.instruction(&Instruction::End);

        // Return old pointer
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.export_section
            .export("alloc", ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__wasm_alloc".to_string(),
            params: vec![("size".to_string(), WasmType::I32)],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    pub(crate) fn emit_reset_function(&mut self) {
        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        func.instruction(&Instruction::I32Const(Self::HEAP_BASE));
        func.instruction(&Instruction::GlobalSet(self.heap_ptr_global));
        func.instruction(&Instruction::I32Const(Self::STACK_BASE));
        func.instruction(&Instruction::GlobalSet(self.stack_ptr_global));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::GlobalSet(self.panicking_global));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::GlobalSet(self.panic_value_ptr_global));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::GlobalSet(self.panic_value_len_global));
        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.export_section
            .export("reset", ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__wasm_reset".to_string(),
            params: vec![],
            results: vec![],
            result_go_types: vec![],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    pub(crate) fn emit_gc_string_bridge_function(&mut self) {
        let go_string_idx = match self.gc_builtin_types.go_string {
            Some(idx) => idx,
            None => return,
        };
        let byte_array_idx = self.gc_builtin_types.byte_array.unwrap();

        let gc_string_vt = Self::gc_ref_val_type(go_string_idx);
        let gc_arr_vt = Self::gc_ref_val_type(byte_array_idx);

        let type_idx = self.next_type_idx;
        self.type_section
            .ty()
            .function(vec![ValType::I32, ValType::I32], vec![gc_string_vt]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        // params: 0=ptr, 1=len; locals: 2=gc_arr, 3=i
        let mut func = Function::new(vec![(1, gc_arr_vt), (1, ValType::I32)]);

        // gc_arr = ArrayNew(byte_array_idx, 0, len)
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalGet(1)); // len
        func.instruction(&Instruction::ArrayNew(byte_array_idx));
        func.instruction(&Instruction::LocalSet(2)); // gc_arr

        // i = 0
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(3));

        // loop: copy bytes from linear memory to GC array
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));

        func.instruction(&Instruction::LocalGet(3)); // i
        func.instruction(&Instruction::LocalGet(1)); // len
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1)); // break if i >= len

        func.instruction(&Instruction::LocalGet(2)); // gc_arr
        func.instruction(&Instruction::LocalGet(3)); // i
        func.instruction(&Instruction::LocalGet(0)); // ptr
        func.instruction(&Instruction::LocalGet(3)); // i
        func.instruction(&Instruction::I32Add);       // ptr + i
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::ArraySet(byte_array_idx));

        func.instruction(&Instruction::LocalGet(3)); // i
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(3)); // i++

        func.instruction(&Instruction::Br(0)); // continue loop
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block

        // StructNew(go_string_idx, gc_arr, len)
        func.instruction(&Instruction::LocalGet(2)); // gc_arr
        func.instruction(&Instruction::LocalGet(1)); // len
        func.instruction(&Instruction::StructNew(go_string_idx));

        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.export_section
            .export("__make_gc_string", ExportKind::Func, func_idx);

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__make_gc_string".to_string(),
            params: vec![
                ("ptr".to_string(), WasmType::I32),
                ("len".to_string(), WasmType::I32),
            ],
            results: vec![WasmType::Ref(go_string_idx)],
            result_go_types: vec!["string".to_string()],
            is_exported: true,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    pub(crate) fn emit_global_var_init_function(&mut self) -> Result<(), Error> {
        let inits = std::mem::take(&mut self.global_var_inits);
        if inits.is_empty() {
            return Ok(());
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let param_entries: Vec<(String, ValType)> = Vec::new();
        let mut locals = LocalAlloc::new(param_entries);
        let mut body: Vec<Instruction<'static>> = Vec::new();

        for (var_name, init_expr, _vt, pkg_ctx) in &inits {
            let prev_pkg = self.current_package.clone();
            self.current_package = pkg_ctx.clone();

            let is_gc_string_global = self.global_vars.get(var_name).map_or(false, |&(_, vt)| matches!(vt, ValType::Ref(_)));
            let is_string_global = !is_gc_string_global && self.global_vars.contains_key(&format!("{}_1", var_name));
            let is_iface_global = self.global_vars.contains_key(&format!("{}_tid", var_name));
            if is_gc_string_global {
                self.compile_expression(init_expr, &mut body, &mut locals)?;
                let (global_idx, _) = self.global_vars[var_name];
                body.push(Instruction::GlobalSet(global_idx));
            } else if is_string_global || is_iface_global {
                let expr_results = self.expression_result_count(init_expr, Some(&locals));
                self.compile_expression(init_expr, &mut body, &mut locals)?;

                if is_iface_global && expr_results == 1 {
                    let box_ptr = locals.add_local(
                        &format!("__ginit_box_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    body.push(Instruction::LocalSet(box_ptr));

                    let (first_global, _) = self.global_vars[var_name];
                    let (tid_global, _) = self.global_vars[&format!("{}_tid", var_name)];

                    body.push(Instruction::LocalGet(box_ptr));
                    body.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                    body.push(Instruction::GlobalSet(first_global));

                    body.push(Instruction::LocalGet(box_ptr));
                    body.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    body.push(Instruction::GlobalSet(tid_global));
                } else {
                    let second_tmp = locals.add_local(
                        &format!("__ginit_v2_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    let first_tmp = locals.add_local(
                        &format!("__ginit_v1_{}", locals.locals.len()),
                        ValType::I32,
                    );
                    body.push(Instruction::LocalSet(second_tmp));
                    body.push(Instruction::LocalSet(first_tmp));

                    let (first_global, _) = self.global_vars[var_name];
                    let suffix = if is_iface_global { "_tid" } else { "_1" };
                    let (second_global, _) = self.global_vars[&format!("{}{}", var_name, suffix)];
                    body.push(Instruction::LocalGet(first_tmp));
                    body.push(Instruction::GlobalSet(first_global));
                    body.push(Instruction::LocalGet(second_tmp));
                    body.push(Instruction::GlobalSet(second_global));
                }
            } else {
                self.compile_expression(init_expr, &mut body, &mut locals)?;
                let (global_idx, _) = self.global_vars[var_name];
                body.push(Instruction::GlobalSet(global_idx));
            }
            self.current_package = prev_pkg;
        }

        body.push(Instruction::End);

        let mut func = Function::new(locals.local_types());
        for instr in &body {
            func.instruction(instr);
        }
        self.code_buffer.push((func_idx, func));

        // Insert before user init() functions
        self.init_func_indices.insert(0, func_idx);

        Ok(())
    }

    pub(crate) fn emit_init_function(&mut self) {
        if self.init_func_indices.is_empty() {
            return;
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![], vec![]);
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        let mut func = Function::new(vec![]);
        for &init_idx in &self.init_func_indices {
            func.instruction(&Instruction::Call(init_idx));
        }
        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.start_func_idx = Some(func_idx);
    }

    pub(crate) fn alloc_func_idx(&self) -> Result<u32, Error> {
        self.functions
            .iter()
            .find(|f| f.name == "__wasm_alloc")
            .map(|f| f.wasm_func_idx)
            .ok_or_else(|| {
                Error::InternalError(
                    "alloc function not registered; emit_alloc_function must be called before compilation".to_string(),
                )
            })
    }

    /// Phase 2: Emit __rt_streq(ptr1, len1, ptr2, len2) -> i32
    pub(crate) fn emit_rt_streq(&mut self) {
        if self.rt_streq_func_idx.is_some() {
            return;
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
        );
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        // params: 0=ptr1, 1=len1, 2=ptr2, 3=len2; locals: 4=result, 5=idx
        let mut func = Function::new(vec![(1, ValType::I32), (1, ValType::I32)]);

        // result = 1 (assume equal)
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::LocalSet(4));

        // if len1 != len2: return 0
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::I32Ne);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::Else);

        // idx = 0
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(5));
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));

        // if idx >= len1: break
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));

        // if ptr1[idx] != ptr2[idx]: result=0, break
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::LocalGet(2));
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::I32Ne);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::Br(2));
        func.instruction(&Instruction::End);

        // idx++
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(5));
        func.instruction(&Instruction::Br(0));
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block
        func.instruction(&Instruction::End); // else

        func.instruction(&Instruction::LocalGet(4));
        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.rt_streq_func_idx = Some(func_idx);
        self.needs_func_table = true;

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__rt_streq".to_string(),
            params: vec![
                ("ptr1".to_string(), WasmType::I32),
                ("len1".to_string(), WasmType::I32),
                ("ptr2".to_string(), WasmType::I32),
                ("len2".to_string(), WasmType::I32),
            ],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    /// Phase 2: Emit __rt_strcmp(ptr1, len1, ptr2, len2) -> i32 (returns -1, 0, 1)
    pub(crate) fn emit_rt_strcmp(&mut self) {
        if self.rt_strcmp_func_idx.is_some() {
            return;
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(
            vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
        );
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        // params: 0=ptr1, 1=len1, 2=ptr2, 3=len2
        // locals: 4=cmp, 5=idx, 6=min_len, 7=b1, 8=b2
        let mut func = Function::new(vec![
            (1, ValType::I32), (1, ValType::I32), (1, ValType::I32),
            (1, ValType::I32), (1, ValType::I32),
        ]);

        // min_len = len1 <= len2 ? len1 : len2
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::I32LeU);
        func.instruction(&Instruction::Select);
        func.instruction(&Instruction::LocalSet(6));

        // cmp = 0, idx = 0
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::LocalSet(5));

        // byte-by-byte comparison loop
        func.instruction(&Instruction::Block(BlockType::Empty));
        func.instruction(&Instruction::Loop(BlockType::Empty));
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::LocalGet(6));
        func.instruction(&Instruction::I32GeU);
        func.instruction(&Instruction::BrIf(1));

        // b1 = ptr1[idx], b2 = ptr2[idx]
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::LocalSet(7));
        func.instruction(&Instruction::LocalGet(2));
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
        func.instruction(&Instruction::LocalSet(8));

        // if b1 < b2: cmp = -1, break
        func.instruction(&Instruction::LocalGet(7));
        func.instruction(&Instruction::LocalGet(8));
        func.instruction(&Instruction::I32LtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(-1i32));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::Br(2));
        func.instruction(&Instruction::End);

        // if b1 > b2: cmp = 1, break
        func.instruction(&Instruction::LocalGet(7));
        func.instruction(&Instruction::LocalGet(8));
        func.instruction(&Instruction::I32GtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::Br(2));
        func.instruction(&Instruction::End);

        // idx++
        func.instruction(&Instruction::LocalGet(5));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(5));
        func.instruction(&Instruction::Br(0));
        func.instruction(&Instruction::End); // loop
        func.instruction(&Instruction::End); // block

        // if cmp == 0: compare lengths
        func.instruction(&Instruction::LocalGet(4));
        func.instruction(&Instruction::I32Eqz);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::I32LtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(-1i32));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::Else);
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::I32GtU);
        func.instruction(&Instruction::If(BlockType::Empty));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::LocalSet(4));
        func.instruction(&Instruction::End);
        func.instruction(&Instruction::End);
        func.instruction(&Instruction::End);

        func.instruction(&Instruction::LocalGet(4));
        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.rt_strcmp_func_idx = Some(func_idx);
        self.needs_func_table = true;

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__rt_strcmp".to_string(),
            params: vec![
                ("ptr1".to_string(), WasmType::I32),
                ("len1".to_string(), WasmType::I32),
                ("ptr2".to_string(), WasmType::I32),
                ("len2".to_string(), WasmType::I32),
            ],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    /// Phase 3: Emit per-type comparison functions and register in function table.
    pub(crate) fn emit_type_cmp_functions(&mut self) {
        let type_names: Vec<(String, u32)> = self.type_registry.iter()
            .map(|(name, &id)| (name.clone(), id))
            .collect();

        // Signature: (ptr_a: i32, ptr_b: i32) -> i32
        let cmp_type_idx = self.next_type_idx;
        self.type_section.ty().function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
        self.next_type_idx += 1;

        for (type_name, _type_id) in &type_names {
            if self.type_cmp_funcs.contains_key(type_name) {
                continue;
            }

            let func_idx = self.next_func_idx;
            self.function_section.function(cmp_type_idx);
            self.next_func_idx += 1;

            let mut func = Function::new(vec![]);

            match type_name.as_str() {
                "int" | "int64" | "uint" | "uint64" => {
                    // i64.load(ptr_a) == i64.load(ptr_b)
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::I64Eq);
                }
                "float64" => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::F64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::F64Load(MemArg { offset: 0, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::F64Eq);
                }
                "float32" => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::F32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::F32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::F32Eq);
                }
                "int32" | "uint32" | "rune" | "uintptr" => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::I32Eq);
                }
                "int16" | "uint16" => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I32Load16U(MemArg { offset: 0, align: 1, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I32Load16U(MemArg { offset: 0, align: 1, memory_index: 0 }));
                    func.instruction(&Instruction::I32Eq);
                }
                "int8" | "uint8" | "byte" | "bool" => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I32Load8U(MemArg { offset: 0, align: 0, memory_index: 0 }));
                    func.instruction(&Instruction::I32Eq);
                }
                "string" => {
                    // String: ptr_a -> (data_ptr, len), ptr_b -> (data_ptr, len)
                    // Call __rt_streq(data_ptr_a, len_a, data_ptr_b, len_b)
                    if let Some(streq_idx) = self.rt_streq_func_idx {
                        func.instruction(&Instruction::LocalGet(0));
                        func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        func.instruction(&Instruction::LocalGet(0));
                        func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                        func.instruction(&Instruction::LocalGet(1));
                        func.instruction(&Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        func.instruction(&Instruction::LocalGet(1));
                        func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
                        func.instruction(&Instruction::Call(streq_idx));
                    } else {
                        func.instruction(&Instruction::I32Const(0));
                    }
                }
                _ => {
                    // Struct types: field-by-field, or fallback to pointer equality
                    if let Some(sdef) = self.struct_defs.get(type_name).cloned() {
                        self.emit_struct_cmp_func_body(&sdef, &mut func);
                    } else {
                        // Pointer equality fallback for unknown types
                        func.instruction(&Instruction::LocalGet(0));
                        func.instruction(&Instruction::LocalGet(1));
                        func.instruction(&Instruction::I32Eq);
                    }
                }
            }

            func.instruction(&Instruction::End);

            self.code_buffer.push((func_idx, func));
            self.type_cmp_funcs.insert(type_name.clone(), func_idx);

            self.functions.push(FuncInfo {
                wasm_func_idx: func_idx,
                type_idx: cmp_type_idx,
                name: format!("__rt_cmp_{}", type_name),
                params: vec![
                    ("ptr_a".to_string(), WasmType::I32),
                    ("ptr_b".to_string(), WasmType::I32),
                ],
                results: vec![WasmType::I32],
                result_go_types: vec![],
                is_exported: false,
                recv_type: None,
                is_variadic: false,
                variadic_elem_vt: None,
                iface_param_indices: vec![],
            });
        }
    }

    fn emit_struct_cmp_func_body(&self, sdef: &StructDef, func: &mut Function) {
        if sdef.fields.is_empty() {
            func.instruction(&Instruction::I32Const(1));
            return;
        }

        // Compare field by field. For each field, load from ptr_a and ptr_b and compare.
        // If any field differs, return 0. If all match, return 1.
        // Uses a "result" local at index 2.
        // We add one local for the result.
        // Since Function::new was called with vec![], we can't easily add locals here.
        // Instead, use a single-expression approach: AND all field comparisons.
        let mut first = true;
        for field in &sdef.fields {
            let offset = field.offset as u64;
            match field.wasm_type {
                WasmType::I64 => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I64Load(MemArg { offset, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I64Load(MemArg { offset, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::I64Eq);
                }
                WasmType::F64 => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::F64Load(MemArg { offset, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::F64Load(MemArg { offset, align: 3, memory_index: 0 }));
                    func.instruction(&Instruction::F64Eq);
                }
                WasmType::F32 => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::F32Load(MemArg { offset, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::F32Load(MemArg { offset, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::F32Eq);
                }
                _ => {
                    func.instruction(&Instruction::LocalGet(0));
                    func.instruction(&Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::LocalGet(1));
                    func.instruction(&Instruction::I32Load(MemArg { offset, align: 2, memory_index: 0 }));
                    func.instruction(&Instruction::I32Eq);
                }
            }
            if !first {
                func.instruction(&Instruction::I32And);
            }
            first = false;
        }
    }

    /// Phase 1: Emit the type descriptor table into the data section.
    pub(crate) fn emit_type_descriptor_table(&mut self) {
        let max_type_id = self.next_type_id;
        if max_type_id <= 1 {
            return;
        }

        let table_size = max_type_id as usize * Self::TYPE_DESC_ENTRY_SIZE as usize;
        let mut data = vec![0u8; table_size];

        for (type_name, &type_id) in &self.type_registry {
            let entry_offset = type_id as usize * Self::TYPE_DESC_ENTRY_SIZE as usize;
            if entry_offset + 12 > data.len() {
                continue;
            }

            let size = self.type_byte_size(type_name);
            let cmp_func_idx = self.type_cmp_funcs.get(type_name).copied().unwrap_or(0);
            let flags: u32 = if self.type_cmp_funcs.contains_key(type_name) { 1 } else { 0 };

            data[entry_offset..entry_offset + 4].copy_from_slice(&(size as u32).to_le_bytes());
            data[entry_offset + 4..entry_offset + 8].copy_from_slice(&cmp_func_idx.to_le_bytes());
            data[entry_offset + 8..entry_offset + 12].copy_from_slice(&flags.to_le_bytes());
        }

        self.data_section.active(
            0,
            &ConstExpr::i32_const(Self::TYPE_DESC_BASE),
            data.into_iter(),
        );

        self.data_offset = Self::TYPE_DESC_BASE as u32 + table_size as u32;
    }

    fn type_byte_size(&self, type_name: &str) -> usize {
        match type_name {
            "int" | "int64" | "uint" | "uint64" | "float64" => 8,
            "float32" => 4,
            "int32" | "uint32" | "rune" | "uintptr" => 4,
            "int16" | "uint16" => 2,
            "int8" | "uint8" | "byte" | "bool" => 1,
            "string" => 8, // ptr + len
            _ => {
                if let Some(sdef) = self.struct_defs.get(type_name) {
                    sdef.total_size as usize
                } else {
                    4
                }
            }
        }
    }

    /// Phase 4: Emit __rt_eq(tid, ptr_a, ptr_b) -> i32
    pub(crate) fn emit_rt_eq(&mut self) {
        if self.rt_eq_func_idx.is_some() {
            return;
        }

        let type_idx = self.next_type_idx;
        self.type_section.ty().function(
            vec![ValType::I32, ValType::I32, ValType::I32],
            vec![ValType::I32],
        );
        self.next_type_idx += 1;

        let func_idx = self.next_func_idx;
        self.function_section.function(type_idx);
        self.next_func_idx += 1;

        // Signature for the comparison functions: (i32, i32) -> i32
        let cmp_type_idx = self.functions.iter()
            .find(|f| f.name.starts_with("__rt_cmp_"))
            .map(|f| f.type_idx)
            .unwrap_or_else(|| {
                let idx = self.next_type_idx;
                self.type_section.ty().function(vec![ValType::I32, ValType::I32], vec![ValType::I32]);
                self.next_type_idx += 1;
                idx
            });

        // params: 0=tid, 1=ptr_a, 2=ptr_b; locals: 3=desc_ptr, 4=cmp_func_idx
        let mut func = Function::new(vec![(1, ValType::I32), (1, ValType::I32)]);

        // Fast path: if ptr_a == ptr_b, return 1
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(2));
        func.instruction(&Instruction::I32Eq);
        func.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        func.instruction(&Instruction::I32Const(1));
        func.instruction(&Instruction::Else);

        // desc_ptr = TYPE_DESC_BASE + tid * 12
        func.instruction(&Instruction::I32Const(Self::TYPE_DESC_BASE));
        func.instruction(&Instruction::LocalGet(0));
        func.instruction(&Instruction::I32Const(Self::TYPE_DESC_ENTRY_SIZE));
        func.instruction(&Instruction::I32Mul);
        func.instruction(&Instruction::I32Add);
        func.instruction(&Instruction::LocalSet(3));

        // cmp_func_idx = i32.load(desc_ptr + 4)
        func.instruction(&Instruction::LocalGet(3));
        func.instruction(&Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
        func.instruction(&Instruction::LocalSet(4));

        // if cmp_func_idx == 0: return 0 (not comparable)
        func.instruction(&Instruction::LocalGet(4));
        func.instruction(&Instruction::I32Eqz);
        func.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
        func.instruction(&Instruction::I32Const(0));
        func.instruction(&Instruction::Else);

        // call_indirect(cmp_func_idx, ptr_a, ptr_b)
        func.instruction(&Instruction::LocalGet(1));
        func.instruction(&Instruction::LocalGet(2));
        func.instruction(&Instruction::LocalGet(4));
        func.instruction(&Instruction::CallIndirect { type_index: cmp_type_idx, table_index: 0 });

        func.instruction(&Instruction::End); // inner if/else
        func.instruction(&Instruction::End); // outer if/else

        func.instruction(&Instruction::End);

        self.code_buffer.push((func_idx, func));
        self.rt_eq_func_idx = Some(func_idx);
        self.needs_func_table = true;

        self.functions.push(FuncInfo {
            wasm_func_idx: func_idx,
            type_idx,
            name: "__rt_eq".to_string(),
            params: vec![
                ("tid".to_string(), WasmType::I32),
                ("ptr_a".to_string(), WasmType::I32),
                ("ptr_b".to_string(), WasmType::I32),
            ],
            results: vec![WasmType::I32],
            result_go_types: vec![],
            is_exported: false,
            recv_type: None,
            is_variadic: false,
            variadic_elem_vt: None,
            iface_param_indices: vec![],
        });
    }

    /// Phase 6: Emit the itab (interface table) into the data section.
    pub(crate) fn emit_itab_table(&mut self) {
        // Assign interface IDs
        let iface_names: Vec<String> = self.iface_defs.keys().cloned().collect();
        for name in &iface_names {
            if !self.iface_ids.contains_key(name) {
                let id = self.next_iface_id;
                self.next_iface_id += 1;
                self.iface_ids.insert(name.clone(), id);
            }
        }

        if self.next_iface_id == 0 || self.type_registry.is_empty() {
            return;
        }

        // Find max methods across all interfaces
        let mut max_methods: u32 = 0;
        for methods in self.iface_defs.values() {
            max_methods = max_methods.max(methods.len() as u32);
        }
        if max_methods == 0 {
            return;
        }
        self.max_iface_methods = max_methods;

        let max_type_id = self.next_type_id;
        let max_iface_id = self.next_iface_id;

        // itab entry size = max_methods * 4 bytes
        let entry_size = max_methods as usize * 4;
        let table_size = max_type_id as usize * max_iface_id as usize * entry_size;

        // Place itab after type descriptor table
        let itab_base = if self.data_offset > 0 {
            ((self.data_offset + 7) / 8) * 8 // align to 8
        } else {
            (Self::TYPE_DESC_BASE as u32 + max_type_id * Self::TYPE_DESC_ENTRY_SIZE as u32 + 7) & !7
        };
        self.itab_base = itab_base;

        let mut data = vec![0u8; table_size];

        // For each (concrete_type, interface) pair, populate the vtable
        let type_entries: Vec<(String, u32)> = self.type_registry.iter()
            .map(|(n, &id)| (n.clone(), id))
            .collect();

        for (type_name, type_id) in &type_entries {
            for (iface_name, &iface_id) in &self.iface_ids {
                let methods = match self.iface_defs.get(iface_name) {
                    Some(m) => m.clone(),
                    None => continue,
                };

                // Check if this type implements this interface
                let mut all_found = true;
                let mut method_indices: Vec<u32> = Vec::new();
                for method_name in &methods {
                    let qualified = format!("{}.{}", type_name, method_name);
                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                        method_indices.push(fi.wasm_func_idx);
                    } else {
                        all_found = false;
                        break;
                    }
                }

                if all_found && !method_indices.is_empty() {
                    let base_offset = (*type_id as usize * max_iface_id as usize + iface_id as usize) * entry_size;
                    for (i, &func_idx) in method_indices.iter().enumerate() {
                        let off = base_offset + i * 4;
                        if off + 4 <= data.len() {
                            data[off..off + 4].copy_from_slice(&func_idx.to_le_bytes());
                        }
                    }
                    self.itab_entries.push((*type_id, iface_id, method_indices));
                }
            }
        }

        if data.iter().any(|&b| b != 0) {
            self.data_section.active(
                0,
                &ConstExpr::i32_const(itab_base as i32),
                data.into_iter(),
            );
        }
    }

    pub(crate) fn get_iface_id(&mut self, iface_name: &str) -> u32 {
        if let Some(&id) = self.iface_ids.get(iface_name) {
            return id;
        }
        let id = self.next_iface_id;
        self.next_iface_id += 1;
        self.iface_ids.insert(iface_name.to_string(), id);
        id
    }
}
