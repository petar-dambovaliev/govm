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
                self.compile_expression(init_expr, &mut body, &mut locals)?;
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
}
