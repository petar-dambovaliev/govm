use super::*;

impl WasmCompiler {
    pub(crate) fn prescan_type_declarations(&mut self, file: &ast::File) {
        for decl in &file.decl {
            if let ast::Declaration::Type(type_decl) = decl {
                for spec in &type_decl.specs {
                    let raw_name = &spec.name.name;
                    let name = self.qualify_pkg_name(raw_name);
                    if let ast::Expression::TypeInterface(iface) = &spec.typ {
                        let mut methods = Vec::new();
                        let mut embedded_ifaces = Vec::new();
                        let mut method_sigs: HashMap<String, (Vec<WasmType>, Vec<WasmType>)> = HashMap::new();
                        for field in &iface.methods.list {
                            if field.name.is_empty() {
                                if let ast::Expression::Ident(embedded_id) = &field.typ {
                                    embedded_ifaces.push(embedded_id.name.clone());
                                }
                            } else {
                                if let ast::Expression::TypeFunction(ft) = &field.typ {
                                    let param_types: Vec<WasmType> = ft.params.list.iter()
                                        .flat_map(|p| self.field_to_wasm_types(p))
                                        .collect();
                                    let result_types: Vec<WasmType> = ft.result.list.iter()
                                        .flat_map(|r| self.field_to_wasm_types(r))
                                        .collect();
                                    for ident in &field.name {
                                        methods.push(ident.name.clone());
                                        method_sigs.insert(ident.name.clone(), (param_types.clone(), result_types.clone()));
                                    }
                                } else {
                                    for ident in &field.name {
                                        methods.push(ident.name.clone());
                                    }
                                }
                            }
                        }
                        for embedded_name in &embedded_ifaces {
                            if let Some(embedded_methods) = self.iface_defs.get(embedded_name) {
                                methods.extend(embedded_methods.clone());
                            }
                            if let Some(embedded_sigs) = self.iface_method_sigs.get(embedded_name) {
                                method_sigs.extend(embedded_sigs.clone());
                            }
                        }
                        self.iface_defs.insert(name.clone(), methods);
                        self.iface_method_sigs.insert(name.clone(), method_sigs);
                    } else if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        self.get_or_create_type_id(&name);
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs.insert(name.clone(), struct_def);
                    } else if let ast::Expression::Ident(base_type) = &spec.typ {
                        self.type_aliases.insert(
                            name.clone(),
                            (base_type.name.clone(), spec.alias),
                        );
                    }
                }
            }
            // Register methods from function declarations
            if let ast::Declaration::Function(func_decl) = decl {
                if let Some(recv) = &func_decl.recv {
                    if let Some(type_name) = self.extract_recv_type_name(recv) {
                        let qualified = self.qualify_pkg_name(&type_name);
                        self.get_or_create_type_id(&qualified);
                    }
                }
            }
        }

        // Resolve forward-declared embedded interfaces
        let iface_names: Vec<String> = self.iface_defs.keys().cloned().collect();
        for _ in 0..iface_names.len() {
            let mut changed = false;
            for name in &iface_names {
                let methods = self.iface_defs.get(name).cloned().unwrap_or_default();
                for decl in &file.decl {
                    if let ast::Declaration::Type(type_decl) = decl {
                        for spec in &type_decl.specs {
                            if spec.name.name == *name {
                                if let ast::Expression::TypeInterface(iface) = &spec.typ {
                                    for field in &iface.methods.list {
                                        if field.name.is_empty() {
                                            if let ast::Expression::Ident(embedded_id) = &field.typ {
                                                if let Some(embedded_methods) = self.iface_defs.get(&embedded_id.name).cloned() {
                                                    for m in &embedded_methods {
                                                        if !methods.contains(m) {
                                                            changed = true;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
            for name in &iface_names {
                let mut methods = self.iface_defs.get(name).cloned().unwrap_or_default();
                let mut sigs = self.iface_method_sigs.get(name).cloned().unwrap_or_default();
                for decl in &file.decl {
                    if let ast::Declaration::Type(type_decl) = decl {
                        for spec in &type_decl.specs {
                            if spec.name.name == *name {
                                if let ast::Expression::TypeInterface(iface) = &spec.typ {
                                    for field in &iface.methods.list {
                                        if field.name.is_empty() {
                                            if let ast::Expression::Ident(embedded_id) = &field.typ {
                                                if let Some(embedded_methods) = self.iface_defs.get(&embedded_id.name).cloned() {
                                                    for m in embedded_methods {
                                                        if !methods.contains(&m) {
                                                            methods.push(m);
                                                        }
                                                    }
                                                }
                                                if let Some(embedded_sigs) = self.iface_method_sigs.get(&embedded_id.name).cloned() {
                                                    for (k, v) in embedded_sigs {
                                                        sigs.entry(k).or_insert(v);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                self.iface_defs.insert(name.clone(), methods);
                self.iface_method_sigs.insert(name.clone(), sigs);
            }
        }
    }

    pub(crate) fn forward_declare_functions(&mut self, decls: &[ast::Declaration]) {
        for decl in decls {
            let func_decl = match decl {
                ast::Declaration::Function(f) => f,
                _ => continue,
            };

            if !func_decl.typ.typ_params.list.is_empty() {
                continue;
            }
            if let Some(recv) = &func_decl.recv {
                if self.is_generic_recv(recv) {
                    continue;
                }
            }

            let is_init = func_decl.name.name == "init"
                && func_decl.recv.is_none()
                && func_decl.typ.params.list.is_empty()
                && func_decl.typ.result.list.is_empty();
            if is_init {
                continue;
            }

            let name = &func_decl.name.name;
            let is_method = func_decl.recv.is_some();

            if func_decl.body.is_none() && !is_method {
                let is_inlined_native = matches!(name.as_str(),
                    "Float64frombits" | "Float64bits" | "Float32frombits" | "Float32bits"
                );
                if is_inlined_native {
                    continue;
                }
                if let Some(&import_idx) = self.wasm_imports.get(name.as_str()) {
                    let internal_name = self.qualify_pkg_name(name);

                    let mut param_types: Vec<ValType> = Vec::new();
                    let mut param_names: Vec<String> = Vec::new();
                    for field in &func_decl.typ.params.list {
                        let field_wasm_types = self.field_to_wasm_types(field);
                        if field.name.is_empty() {
                            for wt in &field_wasm_types {
                                param_types.push(wt.to_val_type());
                                param_names.push(format!("_param{}", param_names.len()));
                            }
                        } else {
                            for ident in &field.name {
                                if !field_wasm_types.is_empty() {
                                    param_types.push(field_wasm_types[0].to_val_type());
                                } else {
                                    param_types.push(ValType::I32);
                                }
                                param_names.push(ident.name.clone());
                            }
                        }
                    }
                    let mut result_types: Vec<ValType> = Vec::new();
                    let mut result_go_types: Vec<String> = Vec::new();
                    for field in &func_decl.typ.result.list {
                        let go_type_name = self.expr_type_name(&field.typ);
                        let field_wasm_types = self.field_to_wasm_types(field);
                        for wt in &field_wasm_types {
                            result_types.push(wt.to_val_type());
                            result_go_types.push(go_type_name.clone());
                        }
                    }

                    let wasm_params: Vec<(String, WasmType)> = param_names
                        .iter()
                        .zip(param_types.iter())
                        .map(|(n, vt)| (n.clone(), match vt {
                            ValType::I32 => WasmType::I32,
                            ValType::I64 => WasmType::I64,
                            ValType::F32 => WasmType::F32,
                            ValType::F64 => WasmType::F64,
                            _ => WasmType::I32,
                        }))
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
                        wasm_func_idx: import_idx,
                        type_idx: 0,
                        name: internal_name.clone(),
                        params: wasm_params,
                        results: wasm_results,
                        result_go_types,
                        is_exported: false,
                        recv_type: None,
                        is_variadic: false,
                        variadic_elem_vt: None,
                        iface_param_indices: Vec::new(),
                    });
                    self.forward_declared.insert(internal_name);
                    continue;
                }
            }

            let recv_type_name = if let Some(recv) = &func_decl.recv {
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

            let is_exported =
                name.chars().next().map_or(false, |c| c.is_uppercase())
                && !is_method
                && self.current_package.is_none();

            let mut param_types: Vec<ValType> = Vec::new();
            let mut param_names: Vec<String> = Vec::new();

            if let Some(recv) = &func_decl.recv {
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

            for field in &func_decl.typ.params.list {
                if let ast::Expression::Ellipsis(ellipsis) = &field.typ {
                    is_variadic = true;
                    let elem_vt = if let Some(ref elt) = ellipsis.elt {
                        Self::infer_array_elem_vt(elt)
                    } else {
                        ValType::I64
                    };
                    variadic_elem_vt = Some(elem_vt);
                    param_types.push(ValType::I32);
                    let vname = field.name.first().map_or(
                        format!("_param{}", param_names.len()),
                        |id| id.name.clone()
                    );
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
                        if is_string_param {
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

            let mut result_go_types: Vec<String> = Vec::new();
            for field in &func_decl.typ.result.list {
                let go_type_name = self.expr_type_name(&field.typ);
                let field_wasm_types = self.field_to_wasm_types(field);
                let count = if field.name.len() > 1 { field.name.len() } else { 1 };
                for _ in 0..count {
                    for _ in &field_wasm_types {
                        result_go_types.push(go_type_name.clone());
                    }
                }
            }

            let wasm_params: Vec<(String, WasmType)> = param_names
                .iter()
                .zip(param_types.iter())
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

            let mut result_types: Vec<ValType> = Vec::new();
            for field in &func_decl.typ.result.list {
                let field_wasm_types = self.field_to_wasm_types(field);
                let count = if field.name.len() > 1 { field.name.len() } else { 1 };
                for _ in 0..count {
                    for wt in &field_wasm_types {
                        result_types.push(wt.to_val_type());
                    }
                }
            }

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

            let mut iface_param_indices: Vec<usize> = Vec::new();
            {
                let mut go_arg_idx: usize = 0;
                for field in &func_decl.typ.params.list {
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

            self.functions.push(FuncInfo {
                wasm_func_idx: func_idx,
                type_idx,
                name: internal_name.clone(),
                params: wasm_params,
                results: wasm_results,
                result_go_types,
                is_exported,
                recv_type: recv_type_name,
                is_variadic,
                variadic_elem_vt,
                iface_param_indices,
            });

            // Detect iterator function pattern: single func-typed param, no results
            if !is_method && result_types.is_empty() && !is_variadic {
                let func_typed_fields: Vec<&ast::Field> = func_decl.typ.params.list.iter()
                    .filter(|f| matches!(&f.typ, ast::Expression::TypeFunction(_)))
                    .collect();
                let non_func_fields: Vec<&ast::Field> = func_decl.typ.params.list.iter()
                    .filter(|f| !matches!(&f.typ, ast::Expression::TypeFunction(_)))
                    .collect();
                if func_typed_fields.len() == 1 && non_func_fields.is_empty() {
                    if let ast::Expression::TypeFunction(ft) = &func_typed_fields[0].typ {
                        let has_bool_result = ft.result.list.len() == 1
                            && matches!(&ft.result.list[0].typ, ast::Expression::Ident(id) if id.name == "bool");
                        let yield_param_count: usize = ft.params.list.iter()
                            .map(|p| if p.name.is_empty() { 1 } else { p.name.len() })
                            .sum();
                        if has_bool_result && yield_param_count <= 2 {
                            let mut yield_param_types = Vec::new();
                            for p in &ft.params.list {
                                let wts = self.field_to_wasm_types(p);
                                for wt in wts {
                                    yield_param_types.push(wt.to_val_type());
                                }
                            }
                            self.iter_func_info.insert(internal_name.clone(), IterFuncInfo {
                                yield_param_types,
                                func_idx,
                            });
                        }
                    }
                }
            }

            self.forward_declared.insert(internal_name);
        }
    }

    pub(crate) fn collect_ident_refs(expr: &ast::Expression, refs: &mut Vec<String>) {
        match expr {
            ast::Expression::Ident(id) => {
                if !matches!(id.name.as_str(), "true" | "false" | "nil" | "iota") {
                    refs.push(id.name.clone());
                }
            }
            ast::Expression::Operation(op) => {
                Self::collect_ident_refs(&op.x, refs);
                if let Some(ref y) = op.y {
                    Self::collect_ident_refs(y, refs);
                }
            }
            ast::Expression::Call(call) => {
                Self::collect_ident_refs(&call.func, refs);
                for arg in &call.args {
                    Self::collect_ident_refs(arg, refs);
                }
            }
            ast::Expression::Paren(p) => Self::collect_ident_refs(&p.expr, refs),
            ast::Expression::Selector(sel) => Self::collect_ident_refs(&sel.x, refs),
            ast::Expression::Index(idx) => {
                if let Some(ref l) = idx.left {
                    Self::collect_ident_refs(l, refs);
                }
                Self::collect_ident_refs(&idx.index, refs);
            }
            ast::Expression::CompositeLit(comp) => {
                Self::collect_ident_refs(&comp.typ, refs);
                for kv in &comp.val.values {
                    if let ast::Element::Expr(e) = &kv.val {
                        Self::collect_ident_refs(e, refs);
                    }
                }
            }
            ast::Expression::Star(star) => Self::collect_ident_refs(&star.right, refs),
            ast::Expression::Slice(sl) => {
                Self::collect_ident_refs(&sl.left, refs);
                for idx_opt in &sl.index {
                    if let Some(e) = idx_opt {
                        Self::collect_ident_refs(e, refs);
                    }
                }
            }
            ast::Expression::TypeAssert(ta) => Self::collect_ident_refs(&ta.left, refs),
            ast::Expression::FuncLit(fl) => {
                for stmt in &fl.body.list {
                    Self::collect_stmt_ident_refs(stmt, refs);
                }
            }
            ast::Expression::List(exprs) => {
                for e in exprs {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Expression::Invar(inv) => Self::collect_ident_refs(&inv.expr, refs),
            _ => {}
        }
    }

    pub(crate) fn collect_stmt_ident_refs(stmt: &ast::Statement, refs: &mut Vec<String>) {
        match stmt {
            ast::Statement::Expr(es) => Self::collect_ident_refs(&es.expr, refs),
            ast::Statement::Return(ret) => {
                for e in &ret.ret {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Statement::Assign(a) => {
                for e in &a.right {
                    Self::collect_ident_refs(e, refs);
                }
            }
            ast::Statement::Block(block) => {
                for s in &block.list {
                    Self::collect_stmt_ident_refs(s, refs);
                }
            }
            ast::Statement::If(if_stmt) => {
                Self::collect_ident_refs(&if_stmt.cond, refs);
            }
            _ => {}
        }
    }

    pub(crate) fn sort_declarations_by_deps(decls: &[ast::Declaration]) -> Vec<ast::Declaration> {
        let mut var_decl_indices: Vec<usize> = Vec::new();
        let mut const_decl_indices: Vec<usize> = Vec::new();
        let mut other_indices: Vec<usize> = Vec::new();

        // Collect names defined by each var/const decl
        let mut all_var_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut all_const_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for (i, decl) in decls.iter().enumerate() {
            match decl {
                ast::Declaration::Variable(var_decl) => {
                    var_decl_indices.push(i);
                    for spec in &var_decl.specs {
                        for name in &spec.name {
                            all_var_names.insert(name.name.clone());
                        }
                    }
                }
                ast::Declaration::Const(const_decl) => {
                    const_decl_indices.push(i);
                    for spec in &const_decl.specs {
                        for name in &spec.name {
                            all_const_names.insert(name.name.clone());
                        }
                    }
                }
                _ => {
                    other_indices.push(i);
                }
            }
        }

        // If no variable declarations or only one, no sorting needed
        if var_decl_indices.len() <= 1 {
            return decls.to_vec();
        }

        // Build dependency graph for variable declarations
        let global_names: std::collections::HashSet<&String> =
            all_var_names.iter().chain(all_const_names.iter()).collect();

        let mut var_deps: Vec<(usize, Vec<usize>)> = Vec::new();
        let mut idx_map: HashMap<String, usize> = HashMap::new();
        for (order, &di) in var_decl_indices.iter().enumerate() {
            if let ast::Declaration::Variable(var_decl) = &decls[di] {
                for spec in &var_decl.specs {
                    for name in &spec.name {
                        idx_map.insert(name.name.clone(), order);
                    }
                }
            }
        }

        for (order, &di) in var_decl_indices.iter().enumerate() {
            let mut deps = Vec::new();
            if let ast::Declaration::Variable(var_decl) = &decls[di] {
                for spec in &var_decl.specs {
                    for val in &spec.values {
                        let mut refs = Vec::new();
                        Self::collect_ident_refs(val, &mut refs);
                        for r in &refs {
                            if global_names.contains(r) {
                                if let Some(&dep_order) = idx_map.get(r) {
                                    if dep_order != order {
                                        deps.push(dep_order);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            var_deps.push((order, deps));
        }

        // Topological sort (Kahn's algorithm)
        let n = var_decl_indices.len();
        let mut in_degree = vec![0usize; n];
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (node, deps) in &var_deps {
            for &dep in deps {
                adj[dep].push(*node);
                in_degree[*node] += 1;
            }
        }

        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for i in 0..n {
            if in_degree[i] == 0 {
                queue.push_back(i);
            }
        }

        let mut sorted_var_order: Vec<usize> = Vec::new();
        while let Some(node) = queue.pop_front() {
            sorted_var_order.push(node);
            for &next in &adj[node] {
                in_degree[next] -= 1;
                if in_degree[next] == 0 {
                    queue.push_back(next);
                }
            }
        }

        // If there's a cycle, fall back to original order
        if sorted_var_order.len() != n {
            return decls.to_vec();
        }

        // Build the result: types/consts first (in original order), then sorted vars, then functions
        let mut result: Vec<ast::Declaration> = Vec::with_capacity(decls.len());

        // First: type and const declarations in original order
        for &i in &other_indices {
            if matches!(&decls[i], ast::Declaration::Type(_)) {
                result.push(decls[i].clone());
            }
        }
        for &i in &const_decl_indices {
            result.push(decls[i].clone());
        }

        // Then: sorted variable declarations
        for &order in &sorted_var_order {
            result.push(decls[var_decl_indices[order]].clone());
        }

        // Finally: function declarations in original order
        for &i in &other_indices {
            if matches!(&decls[i], ast::Declaration::Function(_)) {
                result.push(decls[i].clone());
            }
        }

        result
    }

    pub(crate) fn compile_declaration(&mut self, decl: &ast::Declaration) -> Result<(), Error> {
        match decl {
            ast::Declaration::Function(func_decl) => self.compile_func_decl(func_decl, false),
            ast::Declaration::Variable(var_decl) => {
                for spec in &var_decl.specs {
                    self.compile_global_var(spec)?;
                }
                Ok(())
            }
            ast::Declaration::Const(const_decl) => {
                let num_specs = const_decl.specs.len();

                let mut effective_exprs: Vec<Vec<ast::Expression>> = Vec::with_capacity(num_specs);
                let mut effective_types: Vec<Option<String>> = Vec::with_capacity(num_specs);
                {
                    let mut last_exprs: Vec<ast::Expression> = Vec::new();
                    let mut last_type: Option<String> = None;
                    for spec in const_decl.specs.iter() {
                        if !spec.values.is_empty() {
                            last_exprs = spec.values.clone();
                        }
                        if let Some(ref typ_expr) = spec.typ {
                            if let ast::Expression::Ident(ident) = typ_expr {
                                let type_name = if let Some(ref pkg) = self.current_package {
                                    format!("{}.{}", pkg, ident.name)
                                } else {
                                    ident.name.clone()
                                };
                                last_type = Some(type_name);
                            }
                        }
                        effective_exprs.push(last_exprs.clone());
                        effective_types.push(last_type.clone());
                    }
                }

                let mut resolved = vec![false; num_specs];
                loop {
                    let mut progress = false;
                    for (iota_val, spec) in const_decl.specs.iter().enumerate() {
                        if resolved[iota_val] { continue; }
                        self.current_iota = Some(iota_val as i128);

                        let exprs = &effective_exprs[iota_val];
                        let const_type = &effective_types[iota_val];

                        let mut all_ok = true;
                        let mut results: Vec<(String, ConstValue, Option<String>)> = Vec::new();
                        for (i, name) in spec.name.iter().enumerate() {
                            if let Some(expr) = exprs.get(i).or(exprs.first()) {
                                if let Some(cv) = self.try_eval_const_expr(expr) {
                                    results.push((name.name.clone(), cv, const_type.clone()));
                                } else {
                                    all_ok = false;
                                    break;
                                }
                            } else {
                                all_ok = false;
                                break;
                            }
                        }
                        if all_ok {
                            for (name, cv, ct) in results {
                                if let Some(ref tn) = ct {
                                    if let ConstValue::I64(v) = &cv {
                                        if !Self::const_fits_type(*v, tn) {
                                            return Err(Error::SyntaxError(format!(
                                                "constant {} overflows {}", name, tn
                                            )));
                                        }
                                    }
                                    self.constant_types.insert(name.clone(), tn.clone());
                                }
                                self.constants.insert(name.clone(), cv);
                            }
                            resolved[iota_val] = true;
                            progress = true;
                        }
                    }
                    if resolved.iter().all(|&r| r) { break; }
                    if !progress {
                        for (iota_val, spec) in const_decl.specs.iter().enumerate() {
                            if !resolved[iota_val] {
                                self.current_iota = Some(iota_val as i128);
                                let mut dummy_exprs = effective_exprs[iota_val].clone();
                                let mut dummy_type = effective_types[iota_val].clone();
                                self.compile_global_const(spec, &mut dummy_exprs, &mut dummy_type)?;
                            }
                        }
                        break;
                    }
                }
                self.current_iota = None;
                Ok(())
            }
            ast::Declaration::Type(type_decl) => {
                for spec in &type_decl.specs {
                    Self::reject_unsupported_type(&spec.typ)?;
                    let qualified_name = self.qualify_pkg_name(&spec.name.name);
                    // Generic type definitions: store as template for later monomorphization
                    if !spec.params.list.is_empty() {
                        self.generic_types.insert(qualified_name, spec.clone());
                        continue;
                    }
                    if let ast::Expression::TypeStruct(struct_type) = &spec.typ {
                        let struct_def = self.compute_struct_def(&struct_type.fields);
                        self.struct_defs
                            .insert(qualified_name.clone(), struct_def);
                        self.register_struct_field_map_types(&qualified_name, &struct_type.fields);
                    } else if let ast::Expression::Ident(base_type) = &spec.typ {
                        self.type_aliases.insert(
                            qualified_name,
                            (base_type.name.clone(), spec.alias),
                        );
                    } else if matches!(&spec.typ, ast::Expression::TypeSlice(_) | ast::Expression::TypeMap(_) | ast::Expression::TypeArray(_)) {
                        self.named_composite_types.insert(
                            qualified_name,
                            spec.typ.clone(),
                        );
                    }
                    // Interface types are handled during prescan
                }
                Ok(())
            }
        }
    }

    pub(crate) fn register_struct_field_map_types(&mut self, struct_name: &str, fields: &[ast::Field]) {
        for field in fields {
            if let ast::Expression::TypeMap(map_type) = &field.typ {
                let (kv, ks, vv, vs, sk, sv, vst) = self.map_key_val_types(map_type);
                let nested = self.build_nested_map_type_info(map_type);
                let mti = MapTypeInfo {
                    key_vt: kv, val_vt: vv, key_size: ks, val_size: vs,
                    is_string_key: sk, is_string_val: sv,
                    val_struct_type: vst, nested_map_val_type: nested,
                };
                for name_ident in &field.name {
                    self.struct_field_map_types.insert(
                        (struct_name.to_string(), name_ident.name.clone()),
                        mti.clone(),
                    );
                }
            }
        }
    }

    pub(crate) fn compute_struct_def(&self, fields: &[ast::Field]) -> StructDef {
        let mut result_fields = Vec::new();
        let mut offset: u32 = 0;
        let mut embedded_types = Vec::new();

        for field in fields {
            // Handle embedded (anonymous) fields: promote inner struct fields
            if field.name.is_empty() {
                if let ast::Expression::Ident(type_ident) = &field.typ {
                    let embed_name = type_ident.name.clone();
                    if let Some(inner_def) = self.struct_defs.get(&embed_name) {
                        let embed_offset = offset;
                        embedded_types.push((embed_name.clone(), embed_offset));
                        result_fields.push(StructFieldDef {
                            name: embed_name,
                            wasm_type: WasmType::I32,
                            offset: embed_offset,
                            go_type_tag: None,
                        });
                        for inner_field in &inner_def.fields {
                            let abs_offset = embed_offset + inner_field.offset;
                            result_fields.push(StructFieldDef {
                                name: inner_field.name.clone(),
                                wasm_type: inner_field.wasm_type,
                                offset: abs_offset,
                                go_type_tag: inner_field.go_type_tag.clone(),
                            });
                        }
                        offset += inner_def.total_size;
                        continue;
                    }
                }
            }

            let mut wasm_types = self.field_to_wasm_types(field);
            let names: Vec<String> = if field.name.is_empty() {
                vec!["".to_string()]
            } else {
                field.name.iter().map(|n| n.name.clone()).collect()
            };

            let go_type_tag = match &field.typ {
                ast::Expression::TypeSlice(_) => Some("__slice".to_string()),
                ast::Expression::TypeMap(_) => Some("__map".to_string()),
                ast::Expression::Ident(id) if id.name == "string" => Some("__string".to_string()),
                ast::Expression::Ident(id) if id.name == "error" || id.name == "any" || self.iface_defs.contains_key(&id.name) => {
                    Some("__interface".to_string())
                }
                ast::Expression::TypeInterface(_) => Some("__interface".to_string()),
                ast::Expression::TypePointer(ptr) => {
                    if let ast::Expression::Ident(id) = ptr.typ.as_ref() {
                        if self.struct_defs.contains_key(&id.name) {
                            Some(id.name.clone())
                        } else if let Some(ref pkg) = self.current_package {
                            let qualified = format!("{}.{}", pkg, id.name);
                            if self.struct_defs.contains_key(&qualified) {
                                Some(qualified)
                            } else {
                                Some("__ptr".to_string())
                            }
                        } else {
                            Some("__ptr".to_string())
                        }
                    } else {
                        Some("__ptr".to_string())
                    }
                }
                ast::Expression::Ident(id) if self.struct_defs.contains_key(&id.name) => {
                    Some(id.name.clone())
                }
                _ => None,
            };

            // Interface fields need two i32 slots: data_ptr and type_id
            if go_type_tag.as_deref() == Some("__interface") && wasm_types == vec![WasmType::I32] {
                wasm_types = vec![WasmType::I32, WasmType::I32];
            }

            for name in &names {
                if wasm_types.len() == 1 {
                    let wt = wasm_types[0];
                    let size = wt.byte_size();
                    let align = size;
                    offset = (offset + align - 1) & !(align - 1);
                    result_fields.push(StructFieldDef {
                        name: name.clone(),
                        wasm_type: wt,
                        offset,
                        go_type_tag: go_type_tag.clone(),
                    });
                    offset += size;
                } else {
                    for (i, &wt) in wasm_types.iter().enumerate() {
                        let size = wt.byte_size();
                        let align = size;
                        offset = (offset + align - 1) & !(align - 1);
                        result_fields.push(StructFieldDef {
                            name: if i == 0 {
                                name.clone()
                            } else {
                                format!("{}_{}", name, i)
                            },
                            wasm_type: wt,
                            offset,
                            go_type_tag: if i == 0 { go_type_tag.clone() } else { None },
                        });
                        offset += size;
                    }
                }
            }
        }

        let total_size = if offset == 0 {
            // Empty struct: zero size per Go spec.
            // alloc(0) may return the same pointer for distinct values;
            // the spec allows this for zero-size types.
            0
        } else {
            let align = 8u32;
            (offset + align - 1) & !(align - 1)
        };

        StructDef {
            fields: result_fields,
            total_size,
            embedded_types,
        }
    }

    pub(crate) fn compile_global_var(&mut self, spec: &ast::VarSpec) -> Result<(), Error> {
        // Check for composite types (struct, slice, map) that need heap allocation
        let is_composite_type = spec.typ.as_ref().map_or(false, |t| {
            matches!(t, ast::Expression::TypeSlice(_) | ast::Expression::TypeMap(_) | ast::Expression::TypeArray(_))
                || matches!(t, ast::Expression::Ident(id) if {
                    let resolved = self.resolve_struct_in_pkg(&id.name);
                    self.struct_defs.contains_key(&resolved)
                    || self.named_composite_types.contains_key(&id.name)
                })
        }) || spec.values.first().map_or(false, |v| {
            matches!(v, ast::Expression::CompositeLit(_))
        });

        if is_composite_type {
            // Composite globals are I32 pointers to heap-allocated data
            for name in &spec.name {
                let var_name = self.qualify_pkg_name(&name.name);
                let global_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;
                self.global_vars.insert(var_name.clone(), (global_idx, ValType::I32));

                // Track array element type for correct load instructions
                if let Some(type_expr) = &spec.typ {
                    if let ast::Expression::TypeArray(arr) = type_expr {
                        let elem_vt = Self::infer_array_elem_vt(&arr.typ);
                        let (elem_size, elem_align) = Self::go_type_elem_size_and_align(&arr.typ);
                        self.global_array_elem_types.insert(var_name.clone(), (elem_vt, elem_size, elem_align));
                    }
                }
                if let Some(val) = spec.values.first() {
                    if let ast::Expression::CompositeLit(comp) = val {
                        if let ast::Expression::TypeArray(arr) = comp.typ.as_ref() {
                            let elem_vt = Self::infer_array_elem_vt(&arr.typ);
                            let (elem_size, elem_align) = Self::go_type_elem_size_and_align(&arr.typ);
                            self.global_array_elem_types.insert(var_name.clone(), (elem_vt, elem_size, elem_align));
                        }
                    }
                }

                // Track the struct type for selector access
                if let Some(type_expr) = &spec.typ {
                    if let ast::Expression::Ident(type_id) = type_expr {
                        let resolved = self.resolve_struct_in_pkg(&type_id.name);
                        if self.struct_defs.contains_key(&resolved) {
                            // Will be tracked at usage site via global_var_struct_types
                        }
                    }
                }

                if let Some(val) = spec.values.first() {
                    self.global_var_inits.push((var_name, val.clone(), ValType::I32, self.current_package.clone()));
                }
            }
            return Ok(());
        }

        let is_string_type = spec.typ.as_ref().map_or(false, |t| {
            matches!(t, ast::Expression::Ident(id) if id.name == "string")
        }) || spec.values.first().map_or(false, |v| {
            matches!(v, ast::Expression::BasicLit(lit) if lit.kind == LitKind::String)
                || matches!(v, ast::Expression::Operation(op) if {
                    let is_str_const = self.try_eval_const_expr(v);
                    matches!(is_str_const, Some(ConstValue::Str(_)))
                })
        });

        if is_string_type {
            // Strings are (ptr, len) pairs; use two i32 globals and defer initialization
            for name in &spec.name {
                let var_name = self.qualify_pkg_name(&name.name);
                let ptr_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                let len_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                self.global_vars.insert(var_name.clone(), (ptr_idx, ValType::I32));
                self.global_vars.insert(format!("{}_1", var_name), (len_idx, ValType::I32));

                if let Some(val) = spec.values.first() {
                    self.global_var_inits.push((var_name, val.clone(), ValType::I32, self.current_package.clone()));
                }
            }
            return Ok(());
        }

        let is_iface_typed = spec.values.first().map_or(false, |v| {
            if let ast::Expression::Call(call) = v {
                if let ast::Expression::Ident(fn_id) = call.func.as_ref() {
                    return self.iface_defs.contains_key(&fn_id.name)
                        || fn_id.name == "error"
                        || fn_id.name == "any";
                }
            }
            false
        }) || spec.typ.as_ref().map_or(false, |t| {
            if let ast::Expression::Ident(id) = t {
                return self.iface_defs.contains_key(&id.name)
                    || id.name == "error"
                    || id.name == "any";
            }
            false
        });

        if is_iface_typed {
            for name in &spec.name {
                let var_name = self.qualify_pkg_name(&name.name);
                let data_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                let tid_idx = self.next_global_idx;
                self.global_section.global(
                    GlobalType { val_type: ValType::I32, mutable: true, shared: false },
                    &ConstExpr::i32_const(0),
                );
                self.next_global_idx += 1;

                self.global_vars.insert(var_name.clone(), (data_idx, ValType::I32));
                self.global_vars.insert(format!("{}_tid", var_name), (tid_idx, ValType::I32));

                if let Some(val) = spec.values.first() {
                    self.global_var_inits.push((var_name, val.clone(), ValType::I32, self.current_package.clone()));
                }
            }
            return Ok(());
        }

        let vt = if let Some(type_expr) = &spec.typ {
            self.expr_to_val_type(type_expr)
        } else if let Some(val) = spec.values.first() {
            self.infer_val_type_no_locals(val)
        } else {
            ValType::I64
        };

        let const_init = if let Some(val) = spec.values.first() {
            self.try_eval_const_expr(val)
        } else {
            None
        };

        let has_non_const_init = spec.values.first().is_some() && const_init.is_none();

        for name in &spec.name {
            let var_name = self.qualify_pkg_name(&name.name);
            let global_idx = self.next_global_idx;
            self.global_section.global(
                GlobalType {
                    val_type: vt,
                    mutable: true,
                    shared: false,
                },
                &match (&const_init, vt) {
                    (Some(ConstValue::I64(v)), ValType::I64) => ConstExpr::i64_const(*v as i64),
                    (Some(ConstValue::I64(v)), ValType::I32) => ConstExpr::i32_const(*v as i32),
                    (Some(ConstValue::F64(v)), ValType::F64) => ConstExpr::f64_const(*v),
                    (Some(ConstValue::F64(v)), ValType::F32) => ConstExpr::f32_const(*v as f32),
                    (Some(ConstValue::Bool(v)), _) => ConstExpr::i32_const(*v as i32),
                    (_, ValType::I64) => ConstExpr::i64_const(0),
                    (_, ValType::I32) => ConstExpr::i32_const(0),
                    (_, ValType::F64) => ConstExpr::f64_const(0.0),
                    (_, ValType::F32) => ConstExpr::f32_const(0.0),
                    _ => ConstExpr::i64_const(0),
                },
            );
            self.next_global_idx += 1;
            self.global_vars.insert(var_name.clone(), (global_idx, vt));

            if has_non_const_init {
                if let Some(val) = spec.values.first() {
                    if let ast::Expression::Ident(func_ident) = val {
                        if let Some(fi) = self.find_func_in_pkg(&func_ident.name) {
                            self.global_func_vars.insert(var_name, fi.wasm_func_idx);
                            continue;
                        }
                    }
                    self.global_var_inits.push((var_name, val.clone(), vt, self.current_package.clone()));
                }
            }
        }

        Ok(())
    }

    pub(crate) fn compile_global_const(
        &mut self,
        spec: &ast::ConstSpec,
        last_exprs: &mut Vec<ast::Expression>,
        last_const_type: &mut Option<String>,
    ) -> Result<(), Error> {
        let exprs_to_use = if spec.values.is_empty() {
            last_exprs.as_slice()
        } else {
            *last_exprs = spec.values.clone();
            spec.values.as_slice()
        };

        let const_type_name = if let Some(ref typ_expr) = spec.typ {
            let resolved = match typ_expr {
                ast::Expression::Ident(ident) => {
                    let type_name = if let Some(ref pkg) = self.current_package {
                        format!("{}.{}", pkg, ident.name)
                    } else {
                        ident.name.clone()
                    };
                    Some(type_name)
                }
                _ => None,
            };
            if resolved.is_some() {
                *last_const_type = resolved.clone();
            }
            resolved
        } else {
            last_const_type.clone()
        };

        for (i, name) in spec.name.iter().enumerate() {
            let val_expr = exprs_to_use.get(i).or(exprs_to_use.first());

            if let Some(expr) = val_expr {
                let cv = self.eval_const_expr(expr, &name.name)?;
                if let Some(ref type_name) = const_type_name {
                    if let ConstValue::I64(v) = &cv {
                        let fits = Self::const_fits_type(*v, type_name);
                        if !fits {
                            return Err(Error::SyntaxError(format!(
                                "constant {} overflows {}", name.name, type_name
                            )));
                        }
                    }
                    self.constant_types.insert(name.name.clone(), type_name.clone());
                }
                self.constants.insert(name.name.clone(), cv);
            } else {
                return Err(Error::InternalError(format!(
                    "constant '{}' must have a value",
                    name.name
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn is_unsigned_typed_expr(expr: &ast::Expression) -> bool {
        if let ast::Expression::Call(call) = expr {
            if let ast::Expression::Ident(ident) = call.func.as_ref() {
                return matches!(ident.name.as_str(),
                    "uint" | "uint8" | "uint16" | "uint32" | "uint64" | "uintptr" | "byte");
            }
        }
        false
    }

    pub(crate) fn const_fits_type(v: i128, type_name: &str) -> bool {
        let base = type_name.rsplit('.').next().unwrap_or(type_name);
        match base {
            "int" | "int64" => v >= i64::MIN as i128 && v <= i64::MAX as i128,
            "int32" | "rune" => v >= i32::MIN as i128 && v <= i32::MAX as i128,
            "int16" => v >= i16::MIN as i128 && v <= i16::MAX as i128,
            "int8" => v >= i8::MIN as i128 && v <= i8::MAX as i128,
            "uint" | "uint64" | "uintptr" => v >= 0 && v <= u64::MAX as i128,
            "uint32" => v >= 0 && v <= u32::MAX as i128,
            "uint16" => v >= 0 && v <= u16::MAX as i128,
            "uint8" | "byte" => v >= 0 && v <= u8::MAX as i128,
            _ => true,
        }
    }

    pub(crate) fn eval_const_expr(&self, expr: &ast::Expression, name: &str) -> Result<ConstValue, Error> {
        self.try_eval_const_expr(expr).ok_or_else(|| {
            Error::InternalError(format!(
                "constant '{}' has a non-constant initializer",
                name
            ))
        })
    }

    pub(crate) fn try_eval_const_expr(&self, expr: &ast::Expression) -> Option<ConstValue> {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => Self::parse_go_int(&lit.value).ok().map(ConstValue::I64),
                LitKind::Float => Self::parse_go_float(&lit.value).ok().map(ConstValue::F64),
                LitKind::String => {
                    Self::extract_string_content(&lit.value).map(ConstValue::Str)
                }
                LitKind::Char => {
                    let s = lit.value.trim_matches('\'');
                    Self::unescape_go_char(s).ok().map(|c| ConstValue::I64(c as i128))
                }
                LitKind::Imag => {
                    let num_str = lit.value.trim_end_matches('i');
                    num_str.parse::<f64>().ok().map(|v| ConstValue::Complex128(0.0, v))
                }
                _ => None,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" => Some(ConstValue::Bool(true)),
                "false" => Some(ConstValue::Bool(false)),
                "iota" => self.current_iota.map(ConstValue::I64),
                _ => self.constants.get(&ident.name).cloned(),
            },
            ast::Expression::Operation(op) => {
                if op.y.is_none() {
                    let inner = self.try_eval_const_expr(&op.x)?;
                    let is_unsigned_typed = Self::is_unsigned_typed_expr(&op.x);
                    return match op.op {
                        Operator::Sub => match inner {
                            ConstValue::I64(v) => v.checked_neg().map(ConstValue::I64),
                            ConstValue::F64(v) => Some(ConstValue::F64(-v)),
                            ConstValue::Complex128(r, i) => Some(ConstValue::Complex128(-r, -i)),
                            _ => None,
                        },
                        Operator::Add => Some(inner),
                        Operator::Xor => match inner {
                            ConstValue::I64(v) => {
                                if is_unsigned_typed {
                                    Some(ConstValue::I64((!v as u64) as i128))
                                } else {
                                    Some(ConstValue::I64(!v))
                                }
                            }
                            _ => None,
                        },
                        Operator::Not => match inner {
                            ConstValue::Bool(v) => Some(ConstValue::Bool(!v)),
                            _ => None,
                        },
                        _ => None,
                    };
                }
                let lhs = self.try_eval_const_expr(&op.x)?;
                let rhs = self.try_eval_const_expr(op.y.as_ref().unwrap())?;
                match (&lhs, &rhs) {
                    (ConstValue::I64(a), ConstValue::I64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::I64(a.wrapping_add(*b))),
                            Operator::Sub => Some(ConstValue::I64(a.wrapping_sub(*b))),
                            Operator::Star => Some(ConstValue::I64(a.wrapping_mul(*b))),
                            Operator::Quo => {
                                if *b == 0 { return None; }
                                Some(ConstValue::I64(a.wrapping_div(*b)))
                            }
                            Operator::Rem => {
                                if *b == 0 { return None; }
                                Some(ConstValue::I64(a.wrapping_rem(*b)))
                            }
                            Operator::Shl => {
                                if *b < 0 { return None; }
                                if *b >= 128 { return Some(ConstValue::I64(0)); }
                                Some(ConstValue::I64(a.wrapping_shl(*b as u32)))
                            }
                            Operator::Shr => {
                                if *b < 0 { return None; }
                                if *b >= 128 {
                                    return Some(ConstValue::I64(if *a < 0 { -1 } else { 0 }));
                                }
                                Some(ConstValue::I64(a.wrapping_shr(*b as u32)))
                            }
                            Operator::And => Some(ConstValue::I64(a & b)),
                            Operator::Or => Some(ConstValue::I64(a | b)),
                            Operator::Xor => Some(ConstValue::I64(a ^ b)),
                            Operator::AndNot => Some(ConstValue::I64(a & !b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::F64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::I64(a), ConstValue::F64(b)) => {
                        let a = *a as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(a == *b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != *b)),
                            Operator::Less => Some(ConstValue::Bool(a < *b)),
                            Operator::Greater => Some(ConstValue::Bool(a > *b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= *b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= *b)),
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::I64(b)) => {
                        let b = *b as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::F64(a + b)),
                            Operator::Sub => Some(ConstValue::F64(a - b)),
                            Operator::Star => Some(ConstValue::F64(a * b)),
                            Operator::Quo => Some(ConstValue::F64(a / b)),
                            Operator::Equal => Some(ConstValue::Bool(*a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(*a != b)),
                            Operator::Less => Some(ConstValue::Bool(*a < b)),
                            Operator::Greater => Some(ConstValue::Bool(*a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(*a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(*a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::Str(a), ConstValue::Str(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Str(format!("{}{}", a, b))),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            Operator::Less => Some(ConstValue::Bool(a < b)),
                            Operator::Greater => Some(ConstValue::Bool(a > b)),
                            Operator::LessEqual => Some(ConstValue::Bool(a <= b)),
                            Operator::GreaterEqual => Some(ConstValue::Bool(a >= b)),
                            _ => None,
                        }
                    }
                    (ConstValue::Bool(a), ConstValue::Bool(b)) => {
                        match op.op {
                            Operator::AndAnd => Some(ConstValue::Bool(*a && *b)),
                            Operator::OrOr => Some(ConstValue::Bool(*a || *b)),
                            Operator::Equal => Some(ConstValue::Bool(a == b)),
                            Operator::NotEqual => Some(ConstValue::Bool(a != b)),
                            _ => None,
                        }
                    }
                    // Complex constant arithmetic: 1 + 2i, complex + complex, etc.
                    (ConstValue::Complex128(ar, ai), ConstValue::Complex128(br, bi)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, ai + bi)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, ai - bi)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br - ai * bi, ar * bi + ai * br)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (ar * br + ai * bi) / denom,
                                    (ai * br - ar * bi) / denom,
                                ))
                            }
                            Operator::Equal => Some(ConstValue::Bool(ar == br && ai == bi)),
                            Operator::NotEqual => Some(ConstValue::Bool(ar != br || ai != bi)),
                            _ => None,
                        }
                    }
                    (ConstValue::I64(a), ConstValue::Complex128(br, bi)) => {
                        let ar = *a as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, *bi)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, -bi)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br, ar * bi)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (ar * br) / denom,
                                    (-ar * bi) / denom,
                                ))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::Complex128(ar, ai), ConstValue::I64(b)) => {
                        let br = *b as f64;
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + br, *ai)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - br, *ai)),
                            Operator::Star => Some(ConstValue::Complex128(ar * br, ai * br)),
                            Operator::Quo => {
                                let denom = br * br;
                                Some(ConstValue::Complex128(ar * br / denom, ai * br / denom))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::F64(a), ConstValue::Complex128(br, bi)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(a + br, *bi)),
                            Operator::Sub => Some(ConstValue::Complex128(a - br, -bi)),
                            Operator::Star => Some(ConstValue::Complex128(a * br, a * bi)),
                            Operator::Quo => {
                                let denom = br * br + bi * bi;
                                Some(ConstValue::Complex128(
                                    (a * br) / denom,
                                    (-a * bi) / denom,
                                ))
                            }
                            _ => None,
                        }
                    }
                    (ConstValue::Complex128(ar, ai), ConstValue::F64(b)) => {
                        match op.op {
                            Operator::Add => Some(ConstValue::Complex128(ar + b, *ai)),
                            Operator::Sub => Some(ConstValue::Complex128(ar - b, *ai)),
                            Operator::Star => Some(ConstValue::Complex128(ar * b, ai * b)),
                            Operator::Quo => Some(ConstValue::Complex128(ar / b, ai / b)),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            ast::Expression::Paren(p) => self.try_eval_const_expr(&p.expr),
            ast::Expression::Call(call) if call.args.len() == 1 => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    let arg_val = self.try_eval_const_expr(&call.args[0])?;
                    match ident.name.as_str() {
                        "int" | "int64" | "rune" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < i64::MIN as i128 || v > i64::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            ConstValue::F64(v) => Some(ConstValue::I64(v as i64 as i128)),
                            _ => None,
                        },
                        "int32" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < i32::MIN as i128 || v > i32::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            ConstValue::F64(v) => Some(ConstValue::I64(v as i32 as i128)),
                            _ => None,
                        },
                        "int16" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < i16::MIN as i128 || v > i16::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            _ => None,
                        },
                        "int8" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < i8::MIN as i128 || v > i8::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            _ => None,
                        },
                        "uint" | "uint64" | "uintptr" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < 0 || v > u64::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            ConstValue::F64(v) => Some(ConstValue::I64(v as u64 as i128)),
                            _ => None,
                        },
                        "uint32" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < 0 || v > u32::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            ConstValue::F64(v) => Some(ConstValue::I64(v as u32 as i128)),
                            _ => None,
                        },
                        "uint16" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < 0 || v > u16::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            _ => None,
                        },
                        "uint8" | "byte" => match arg_val {
                            ConstValue::I64(v) => {
                                if v < 0 || v > u8::MAX as i128 { return None; }
                                Some(ConstValue::I64(v))
                            }
                            _ => None,
                        },
                        "float64" => match arg_val {
                            ConstValue::I64(v) => Some(ConstValue::F64(v as f64)),
                            ConstValue::F64(v) => Some(ConstValue::F64(v)),
                            _ => None,
                        },
                        "float32" => match arg_val {
                            ConstValue::I64(v) => Some(ConstValue::F64(v as f32 as f64)),
                            ConstValue::F64(v) => Some(ConstValue::F64(v as f32 as f64)),
                            _ => None,
                        },
                        "string" => match arg_val {
                            ConstValue::I64(v) => {
                                if let Some(c) = char::from_u32(v as u32) {
                                    Some(ConstValue::Str(c.to_string()))
                                } else {
                                    Some(ConstValue::Str("\u{FFFD}".to_string()))
                                }
                            }
                            ConstValue::Str(s) => Some(ConstValue::Str(s)),
                            _ => None,
                        },
                        _ => None,
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn infer_val_type_no_locals(&self, expr: &ast::Expression) -> ValType {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => ValType::I64,
                LitKind::Float => ValType::F64,
                LitKind::String => ValType::I32,
                LitKind::Imag => ValType::I32,
                _ => ValType::I64,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => ValType::I32,
                _ => {
                    if let Some(cv) = self.constants.get(&ident.name) {
                        return match cv {
                            ConstValue::I64(_) => ValType::I64,
                            ConstValue::F64(_) => ValType::F64,
                            ConstValue::Bool(_) => ValType::I32,
                            ConstValue::Str(_) => ValType::I32,
                            ConstValue::Complex128(_, _) => ValType::I32,
                        };
                    }
                    ValType::I64
                }
            },
            ast::Expression::Operation(op) if op.y.is_some() => {
                let lhs = self.infer_val_type_no_locals(&op.x);
                let rhs = self.infer_val_type_no_locals(op.y.as_ref().unwrap());
                if lhs == ValType::F64 || rhs == ValType::F64 {
                    ValType::F64
                } else if lhs == ValType::F32 || rhs == ValType::F32 {
                    ValType::F32
                } else if lhs == ValType::I32 && rhs == ValType::I64
                    && matches!(op.y.as_ref().unwrap().as_ref(), ast::Expression::BasicLit(lit) if lit.kind == LitKind::Integer) {
                    ValType::I32
                } else if lhs == ValType::I64 && rhs == ValType::I32
                    && matches!(op.x.as_ref(), ast::Expression::BasicLit(lit) if lit.kind == LitKind::Integer) {
                    ValType::I32
                } else if lhs == ValType::I64 || rhs == ValType::I64 {
                    ValType::I64
                } else {
                    lhs
                }
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "float64" => return ValType::F64,
                        "float32" => return ValType::F32,
                        "int" | "int64" | "uint" | "uint64" => return ValType::I64,
                        "int8" | "int16" | "int32" | "rune"
                        | "byte" | "uint8" | "uint16" | "uint32" | "bool" | "uintptr" => return ValType::I32,
                        "Float64frombits" => return ValType::F64,
                        "Float64bits" => return ValType::I64,
                        "Float32frombits" => return ValType::F32,
                        "Float32bits" => return ValType::I32,
                        _ => {}
                    }
                    if let Some(fi) = self.find_func_in_pkg(&ident.name) {
                        return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                    }
                }
                ValType::I64
            }
            _ => ValType::I64,
        }
    }
}
