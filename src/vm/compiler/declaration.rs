use crate::parser::ast::{self, ConstSpec, Decl, Expression, FuncDecl, VarSpec};
use crate::parser::token::Operator;
use crate::vm::compiler::analysis;
use crate::vm::compiler::compiler::{Compiler, MemVar};
use crate::vm::compiler::{make_ident_name, make_method_name, FuncContext};
use crate::vm::symbols::{ContextType, DefineType, Qualifier, Scope, WasmBinding};
use crate::vm::Error;
use crate::wasm::layout::field_byte_size;
use wasm_encoder::{Instruction, ValType};

pub fn compile_variable(pkg: &str, v: &Decl<VarSpec>, c: &mut Compiler) -> Result<(), Error> {
    for spec in &v.specs {
        let declared_tp = c
            .expression_to_define_type(
                pkg,
                spec.typ
                    .as_ref()
                    .expect("no declared values requires a declared type"),
            )
            .ok_or_else(|| Error::TypeError("failed to resolve declared type".to_string()))?;

        let values = if spec.values.is_empty() {
            let mut defaults = Vec::with_capacity(spec.name.len());
            for _ in 0..spec.name.len() {
                defaults.push(c.make_type_default_val(declared_tp.clone()));
            }
            defaults
        } else {
            spec.values.clone()
        };

        for (name, value) in spec.name.iter().zip(values.iter()) {
            let mut rt = c.compile_expression(pkg, value)?;

            if rt.is_nil() {
                if Compiler::is_interface_type(&declared_tp) {
                    c.wasm.active().i32_const(0);
                }
                rt = declared_tp.clone();
            }

            // Box concrete values into interfaces BEFORE defining the symbol
            if Compiler::is_interface_type(&declared_tp) && !Compiler::is_interface_type(&rt) && !rt.is_nil() {
                let tname = Compiler::type_name_for_tag(&rt);
                c.box_to_interface(&rt, &tname);
                rt = declared_tp.clone();
            }

            let symbol = if c.symbols.current_context().scope == Scope::Global {
                let updated = c.symbols.update_dt(
                    pkg,
                    name.name.as_str(),
                    DefineType::Qualified(Qualifier::Var, Box::new(rt.clone())),
                );
                if !updated {
                    return Err(Error::InternalError(format!(
                        "failed to update symbol: {}",
                        name.name
                    )));
                }
                c.symbols
                    .resolve(pkg, name.name.as_str())
                    .ok_or_else(|| {
                        Error::ReferenceError(format!("undefined variable: {}", name.name))
                    })?
                    .get_symbol()
            } else {
                c.symbols.define(
                    pkg,
                    name.name.as_str(),
                    DefineType::Qualified(Qualifier::Var, Box::new(rt.clone())),
                    rt.is_invar(),
                )
            };

            if let Some(mv) = c.func_ctx().mem_vars.get(&name.name).cloned() {
                let tmp = c.func_ctx().next_wasm_local;
                c.wasm.active().local_set(tmp);
                c.wasm.active().local_get(mv.addr_local);
                c.wasm.active().local_get(tmp);
                c.wasm.active().i32_store(0);
            } else if c.func_ctx().escaped_vars.contains(&name.name) {
                let size = field_byte_size(&rt);
                let rt_alloc_idx = c.alloc_func_idx(true)
                    .expect("RtAllocPersistent not registered");
                let val_tmp = c.func_ctx().next_wasm_local;
                c.wasm.active().local_set(val_tmp);
                c.wasm.active().i32_const(size as i32);
                c.wasm.active().call(rt_alloc_idx);
                let addr_local = c.func_ctx().next_wasm_local + 1;
                c.wasm.active().local_tee(addr_local);
                c.wasm.active().local_get(val_tmp);
                c.wasm.active().i32_store(0);
                c.func_ctx().mem_vars.insert(name.name.clone(), MemVar { addr_local, size });
                c.func_ctx().next_wasm_local = addr_local + 1;
            } else if Compiler::is_interface_type(&rt) {
                let base = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 2;
                c.func_ctx().locals.insert(symbol.index, base);
                c.wasm.active().local_set(base + 1); // data_ptr
                c.wasm.active().local_set(base);      // type_tag
            } else if Compiler::is_string_type(&rt) {
                let base = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 2;
                c.func_ctx().locals.insert(symbol.index, base);
                c.wasm.active().local_set(base + 1);
                c.wasm.active().local_set(base);
            } else {
                let local_idx = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 1;
                c.func_ctx().locals.insert(symbol.index, local_idx);
                c.wasm.active().local_set(local_idx);
            }
        }
    }
    Ok(())
}

/// Pre-register a function's type and index in the WASM module without compiling its body.
/// Used by stdlib compilation to resolve intra-package calls.
pub fn forward_declare_function(pkg: &str, f: &FuncDecl, c: &mut Compiler) -> Result<(), Error> {
    let f_name = if let Some(recv) = f.recv.as_ref() {
        let recv_field = recv
            .list
            .first()
            .ok_or_else(|| Error::SyntaxError("receiver field list is empty".to_string()))?;
        let t = c
            .expression_to_define_type(pkg, &recv_field.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve receiver type".to_string()))?;
        make_method_name(pkg, t.strip_ref(), &f.name.name)
    } else {
        f.name.name.clone()
    };

    if c.wasm_func_map.contains_key(&f_name) {
        return Ok(());
    }

    let mut wasm_params: Vec<ValType> = Vec::new();
    if let Some(recv) = f.recv.as_ref() {
        let recv_field = recv.list.first().unwrap();
        let t = c.expression_to_define_type(pkg, &recv_field.typ).unwrap();
        wasm_params.push(Compiler::define_type_to_wasm(&t));
    }
    for p in &f.typ.params.list {
        let t = c
            .expression_to_define_type(pkg, &p.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve parameter type".to_string()))?;
        for _ in &p.name {
            if Compiler::is_fat_type(&t) {
                wasm_params.push(ValType::I32);
                wasm_params.push(ValType::I32);
            } else {
                wasm_params.push(Compiler::define_type_to_wasm(&t));
            }
        }
    }

    let mut wasm_results: Vec<ValType> = Vec::new();
    for el in &f.typ.result.list {
        let t = c
            .expression_to_define_type(pkg, &el.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve return type".to_string()))?;
        wasm_results.push(Compiler::define_type_to_wasm(&t));
    }

    let type_idx = c.wasm.add_func_type(wasm_params, wasm_results);
    let func_idx = c.wasm.define_function(type_idx);

    let mangled = make_ident_name(pkg, &f.name.name);
    c.wasm_func_map.insert(f_name, func_idx);
    c.wasm_func_map.insert(mangled, func_idx);
    c.symbols.set_wasm_binding(pkg, &f.name.name, WasmBinding::Func { func_idx });

    Ok(())
}

pub fn compile_function(pkg: &str, f: &FuncDecl, c: &mut Compiler) -> Result<(), Error> {
    let (f_name, recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
        let recv_field = recv
            .list
            .first()
            .ok_or_else(|| Error::SyntaxError("receiver field list is empty".to_string()))?;
        let t = c
            .expression_to_define_type(pkg, &recv_field.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve receiver type".to_string()))?;

        (
            make_method_name(pkg, t.strip_ref(), &f.name.name),
            Some(recv_field),
            Some(Box::new(t)),
        )
    } else {
        (f.name.name.clone(), None, None)
    };

    let symbol = match c.symbols.resolve(pkg, &f_name) {
        Some(s) => {
            c.symbols.update_dt(
                pkg,
                &f_name,
                DefineType::Func {
                    name: f.name.name.clone(),
                    recv: recv_t.clone(),
                    args: vec![],
                    rt: Box::new(DefineType::Null),
                },
            );
            s.get_symbol()
        }
        None => c.symbols.define(
            pkg,
            &f_name,
            DefineType::Func {
                name: f.name.name.clone(),
                recv: recv_t.clone(),
                args: vec![],
                rt: Box::new(DefineType::Null),
            },
            false,
        ),
    };

    c.symbols.new_context(false);
    c.func_contexts.push(FuncContext::new(0));

    let mut wasm_params: Vec<ValType> = Vec::new();

    if let Some(recv) = recv {
        let t = c
            .expression_to_define_type(pkg, &recv.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve receiver type".to_string()))?;

        let recv_name = &recv
            .name
            .first()
            .ok_or_else(|| Error::SyntaxError("receiver name is empty".to_string()))?
            .name;

        let sym = c.symbols.define(
            pkg,
            recv_name,
            DefineType::Qualified(Qualifier::Var, Box::new(t.clone())),
            t.is_invar(),
        );

        let local_idx = c.func_ctx().next_wasm_local;
        c.func_ctx().next_wasm_local += 1;
        c.func_ctx().locals.insert(sym.index, local_idx);
        wasm_params.push(Compiler::define_type_to_wasm(&t));
    }

    let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

    for p in &f.typ.params.list {
        let t = c
            .expression_to_define_type(pkg, &p.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve parameter type".to_string()))?;
        for name in &p.name {
            decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

            let sym = c.symbols.define(
                pkg,
                &name.name,
                DefineType::Qualified(Qualifier::Var, Box::new(t.clone())),
                t.is_invar(),
            );

            if Compiler::is_fat_type(&t) {
                let base = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 2;
                c.func_ctx().locals.insert(sym.index, base);
                wasm_params.push(ValType::I32);
                wasm_params.push(ValType::I32);
            } else {
                let local_idx = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 1;
                c.func_ctx().locals.insert(sym.index, local_idx);
                wasm_params.push(Compiler::define_type_to_wasm(&t));
            }
        }
    }

    let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());
    for el in &f.typ.result.list {
        let t = c
            .expression_to_define_type(pkg, &el.typ)
            .ok_or_else(|| Error::TypeError("failed to resolve return type".to_string()))?;
        decl_r_types.push(t);
    }

    let r_t = if decl_r_types.is_empty() {
        DefineType::Null
    } else if decl_r_types.len() == 1 {
        decl_r_types[0].clone()
    } else {
        DefineType::Tuple(decl_r_types.clone())
    };

    let func_def = DefineType::Func {
        name: f.name.name.clone(),
        recv: recv_t,
        args: decl_arg_types,
        rt: Box::new(r_t.clone()),
    };
    c.symbols.update_dt(pkg, &f_name, func_def.clone());

    let wasm_results: Vec<ValType> = if decl_r_types.is_empty() {
        vec![]
    } else {
        decl_r_types
            .iter()
            .map(|dt| Compiler::define_type_to_wasm(dt))
            .collect()
    };

    let func_idx = if let Some(&existing) = c.wasm_func_map.get(&f_name) {
        existing
    } else {
        let type_idx = c
            .wasm
            .add_func_type(wasm_params.clone(), wasm_results.clone());
        let idx = c.wasm.define_function(type_idx);
        c.wasm_func_map.insert(f_name.clone(), idx);
        idx
    };
    let mangled = crate::vm::compiler::make_ident_name(pkg, &f.name.name);
    c.wasm_func_map.insert(mangled, func_idx);
    c.symbols.set_wasm_binding(pkg, &f.name.name, WasmBinding::Func { func_idx });

    let num_params = wasm_params.len() as u32;

    c.func_ctx().wasm_func_idx = func_idx;
    c.func_ctx().expected_ret = if r_t == DefineType::Null {
        None
    } else {
        Some(r_t)
    };

    let estimated_locals = count_locals_in_body(f);
    // Each local might need 2 slots (string = ptr+len). Slice operations use many scratch
    // locals (hdr, data_ptr, len, cap, addr, etc.), so pad generously.
    let padded = estimated_locals * 3 + 16;
    let mut body_locals: Vec<(u32, ValType)> = Vec::new();
    if padded > 0 {
        body_locals.push((padded as u32, ValType::I32));
    }

    c.wasm.begin_func_body(func_idx, body_locals);
    c.func_ctx().next_wasm_local = num_params;

    if f.body.is_none() {
        if let Some(emitter) = crate::stdlib::get_native_func(&f.name.name) {
            emitter(c);
            c.wasm.end_func_body();
            c.symbols.leave_context();
            c.func_contexts.pop();
            return Ok(());
        }
        return Err(Error::SyntaxError(format!(
            "function \"{}\" has no body and no native implementation",
            f.name.name
        )));
    }

    // Save $sp for stack-allocated arrays; restore on return.
    let sp_idx = c.wasm.sp_global_idx()
        .expect("$sp global not registered");
    let saved_sp = c.func_ctx().next_wasm_local;
    c.func_ctx().next_wasm_local += 1;
    c.func_ctx().saved_sp_local = Some(saved_sp);
    c.wasm.active().global_get(sp_idx);
    c.wasm.active().local_set(saved_sp);

    // Save heap watermark for scope-based freeing (skip for runtime allocator functions).
    let is_runtime_func = crate::stdlib::is_intrinsic(&f.name.name)
        || matches!(f.name.name.as_str(), "RtAlloc" | "RtAllocPersistent" | "RtFree"
            | "RtWatermark" | "RtScopeReset");
    if !is_runtime_func {
        if let Some(&wm_idx) = c.wasm_func_map.get("RtWatermark") {
            let saved_wm = c.func_ctx().next_wasm_local;
            c.func_ctx().next_wasm_local += 1;
            c.func_ctx().saved_heap_wm_local = Some(saved_wm);
            c.wasm.active().call(wm_idx);
            c.wasm.active().local_set(saved_wm);
        }
    }

    // Escape analysis: determine which variables need linear memory allocation.
    if let Some(body) = &f.body {
        let addr_taken = analysis::analyze_address_taken_vars(&body.list);
        if !addr_taken.is_empty() {
            let escaping = analysis::analyze_function_escapes(&body.list);
            let var_infos = collect_addr_taken_var_types(&body.list, &addr_taken, c, pkg);

            // Split into non-escaping (stack frame) and escaping (heap, deferred to decl site)
            let mut frame_vars: Vec<(String, u32)> = Vec::new();
            for (name, size) in &var_infos {
                if escaping.contains(name.as_str()) {
                    c.func_ctx().escaped_vars.insert(name.clone());
                } else {
                    frame_vars.push((name.clone(), *size));
                }
            }

            if !frame_vars.is_empty() {
                // Compute frame layout
                let mut total_frame: u32 = 0;
                let mut offsets: Vec<(String, u32, u32)> = Vec::new();
                for (name, size) in &frame_vars {
                    let aligned = (total_frame + 3) & !3;
                    offsets.push((name.clone(), aligned, *size));
                    total_frame = aligned + size;
                }
                total_frame = (total_frame + 3) & !3;

                // Allocate frame: $sp -= total_frame
                c.wasm.active().global_get(sp_idx);
                c.wasm.active().i32_const(total_frame as i32);
                c.wasm.active().emit(&Instruction::I32Sub);
                c.wasm.active().global_set(sp_idx);

                let fb_local = c.func_ctx().next_wasm_local;
                c.func_ctx().next_wasm_local += 1;
                c.wasm.active().global_get(sp_idx);
                c.wasm.active().local_set(fb_local);
                c.func_ctx().frame_base_local = Some(fb_local);

                // Compute each variable's address = frame_base + offset
                for (name, offset, size) in &offsets {
                    let addr_local = c.func_ctx().next_wasm_local;
                    c.func_ctx().next_wasm_local += 1;
                    c.wasm.active().local_get(fb_local);
                    if *offset > 0 {
                        c.wasm.active().i32_const(*offset as i32);
                        c.wasm.active().emit(&Instruction::I32Add);
                    }
                    c.wasm.active().local_set(addr_local);
                    c.func_ctx().mem_vars.insert(name.clone(), MemVar { addr_local, size: *size });
                }
            }
        }
    }

    let mut terminates = None;
    if let Some(body) = &f.body {
        terminates = c.compile_block_statement(pkg, &body.list)?;
    }

    // Restore heap watermark before implicit return.
    if let (Some(saved_wm), Some(&scope_reset_idx)) = (
        c.func_ctx().saved_heap_wm_local,
        c.wasm_func_map.get("RtScopeReset"),
    ) {
        c.wasm.active().local_get(saved_wm);
        c.wasm.active().call(scope_reset_idx);
    }

    // Restore $sp before implicit return (fallthrough for void functions or unreachable).
    let saved_sp = c.func_ctx().saved_sp_local.expect("saved_sp_local not set");
    c.wasm.active().local_get(saved_sp);
    c.wasm.active().global_set(sp_idx);

    if !decl_r_types.is_empty() {
        c.wasm.active().emit(&wasm_encoder::Instruction::Unreachable);
    }

    c.wasm.end_func_body();

    c.symbols.leave_context();
    c.func_contexts.pop();

    let _ = symbol;

    Ok(())
}

fn count_locals_in_body(f: &FuncDecl) -> usize {
    if let Some(body) = &f.body {
        count_locals_in_stmts(&body.list)
    } else {
        0
    }
}

fn count_locals_in_stmts(stmts: &[crate::parser::ast::Statement]) -> usize {
    let mut count = 0;
    for stmt in stmts {
        count += count_locals_in_stmt(stmt);
    }
    count
}

fn count_locals_in_stmt(stmt: &crate::parser::ast::Statement) -> usize {
    use crate::parser::ast::Statement;
    match stmt {
        Statement::Assign(a) => {
            if a.op == crate::parser::token::Operator::Define {
                a.left.len()
            } else {
                0
            }
        }
        Statement::Declaration(d) => match d {
            crate::parser::ast::DeclStmt::Variable(v) => {
                v.specs.iter().map(|s| s.name.len()).sum()
            }
            crate::parser::ast::DeclStmt::Const(c) => {
                c.specs.iter().map(|s| s.name.len()).sum()
            }
            _ => 0,
        },
        Statement::If(ifstmt) => {
            let mut n = count_locals_in_stmts(&ifstmt.body.list);
            if let Some(init) = &ifstmt.init {
                n += count_locals_in_stmt(init);
            }
            if let Some(alt) = &ifstmt.else_ {
                n += count_locals_in_stmt(alt);
            }
            n
        }
        Statement::For(forstmt) => {
            let mut n = count_locals_in_stmts(&forstmt.body.list);
            if let Some(init) = &forstmt.init {
                n += count_locals_in_stmt(init);
            }
            if let Some(post) = &forstmt.post {
                n += count_locals_in_stmt(post);
            }
            n
        }
        Statement::Block(b) => count_locals_in_stmts(&b.list),
        Statement::Range(r) => {
            let mut n = count_locals_in_stmts(&r.body.list);
            if r.key.is_some() { n += 1; }
            if r.value.is_some() { n += 1; }
            n
        }
        _ => 0,
    }
}

pub fn compile_const(
    pkg: &str,
    c: &Decl<ConstSpec>,
    compiler: &mut Compiler,
) -> Result<(), Error> {
    let mut c_iter = c.specs.iter();
    let mut grouped_constants = vec![];
    let mut current_group = vec![];

    while let Some(spec) = c_iter.next() {
        if current_group.is_empty() {
            assert!(!spec.values.is_empty());
            assert_eq!(spec.name.len(), spec.values.len());
            current_group.push(spec);
            continue;
        }

        let value_spec = current_group.first().expect("should not happen");
        if spec.name.len() != value_spec.name.len() {
            assert!(!spec.values.is_empty());
            assert_eq!(spec.name.len(), spec.values.len());
            grouped_constants.push(current_group.clone());
            current_group = vec![spec];
            continue;
        }

        if spec.values.is_empty() {
            current_group.push(spec);
        } else {
            assert_eq!(spec.name.len(), spec.values.len());
            grouped_constants.push(current_group.clone());
            current_group = vec![spec];
        }
    }

    grouped_constants.push(current_group);

    compiler.iota = 0;

    for grouped_constant in grouped_constants {
        for cnst in grouped_constant.clone() {
            for (name, value) in cnst.name.iter().zip(
                grouped_constant
                    .first()
                    .expect("not to happen")
                    .values
                    .clone(),
            ) {
                let rt = compiler.compile_expression(pkg, &value)?;

                let symbol = compiler.symbols.define(
                    pkg,
                    name.name.as_str(),
                    DefineType::Qualified(Qualifier::Const, Box::new(rt.clone())),
                    false,
                );

                let local_idx = compiler.func_ctx().next_wasm_local;
                compiler.func_ctx().next_wasm_local += 1;
                compiler.func_ctx().locals.insert(symbol.index, local_idx);

                compiler.wasm.active().local_set(local_idx);
            }
            compiler.iota += 1;
        }
    }

    Ok(())
}

/// Walk function body to find address-taken variables and their byte sizes.
fn collect_addr_taken_var_types(
    stmts: &[ast::Statement],
    addr_taken: &std::collections::HashSet<String>,
    c: &mut Compiler,
    pkg: &str,
) -> Vec<(String, u32)> {
    let mut result = Vec::new();
    collect_var_types_from_stmts(stmts, addr_taken, c, pkg, &mut result);
    result
}

fn collect_var_types_from_stmts(
    stmts: &[ast::Statement],
    addr_taken: &std::collections::HashSet<String>,
    c: &mut Compiler,
    pkg: &str,
    out: &mut Vec<(String, u32)>,
) {
    for stmt in stmts {
        match stmt {
            ast::Statement::Assign(assign) if matches!(assign.op, Operator::Define) => {
                for lhs in &assign.left {
                    if let Expression::Ident(id) = lhs {
                        if addr_taken.contains(&id.name) && !out.iter().any(|(n, _)| n == &id.name) {
                            out.push((id.name.clone(), 4));
                        }
                    }
                }
            }
            ast::Statement::Declaration(ast::DeclStmt::Variable(var_decl)) => {
                for spec in &var_decl.specs {
                    for name in &spec.name {
                        if addr_taken.contains(&name.name) && !out.iter().any(|(n, _)| n == &name.name) {
                            let size = if let Some(ref typ) = spec.typ {
                                c.expression_to_define_type(pkg, typ)
                                    .map(|dt| field_byte_size(&dt))
                                    .unwrap_or(4)
                            } else {
                                4
                            };
                            out.push((name.name.clone(), size));
                        }
                    }
                }
            }
            ast::Statement::If(if_stmt) => {
                if let Some(init) = &if_stmt.init {
                    collect_var_types_from_stmts(std::slice::from_ref(init.as_ref()), addr_taken, c, pkg, out);
                }
                collect_var_types_from_stmts(&if_stmt.body.list, addr_taken, c, pkg, out);
                if let Some(else_) = &if_stmt.else_ {
                    collect_var_types_from_stmts(std::slice::from_ref(else_.as_ref()), addr_taken, c, pkg, out);
                }
            }
            ast::Statement::For(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    collect_var_types_from_stmts(std::slice::from_ref(init.as_ref()), addr_taken, c, pkg, out);
                }
                collect_var_types_from_stmts(&for_stmt.body.list, addr_taken, c, pkg, out);
            }
            ast::Statement::Range(range_stmt) => {
                collect_var_types_from_stmts(&range_stmt.body.list, addr_taken, c, pkg, out);
            }
            ast::Statement::Block(block) => {
                collect_var_types_from_stmts(&block.list, addr_taken, c, pkg, out);
            }
            ast::Statement::Switch(sw) => {
                if let Some(init) = &sw.init {
                    collect_var_types_from_stmts(std::slice::from_ref(init.as_ref()), addr_taken, c, pkg, out);
                }
                for clause in &sw.block.body {
                    collect_var_types_from_stmts(&clause.body, addr_taken, c, pkg, out);
                }
            }
            _ => {}
        }
    }
}
