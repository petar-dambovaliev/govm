use super::*;

impl WasmCompiler {
    pub(crate) fn is_iface_go_type(&self, go_type: &str) -> bool {
        go_type == "error" || go_type == "any" || self.iface_defs.contains_key(go_type)
    }

    pub(crate) fn emit_nil_iface_box(
        &mut self,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        out.push(Instruction::I32Const(8));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let wrapper = locals.add_local(&format!("__nil_ibox_w_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(wrapper));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::I32Const(0));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        Ok(())
    }

    pub(crate) fn emit_return_iface_box(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        self.compile_expression(expr, out, locals)?;
        let rhs_vt = self.infer_val_type(expr, locals);
        let rhs_type_name = self.infer_concrete_type_name(expr, locals);
        let type_id = self.get_or_create_type_id(&rhs_type_name);

        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            if rhs_vt == Self::gc_ref_val_type(go_string_idx) {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                let str_len = locals.add_local(&format!("__ret_ibox_sl_{}", locals.locals.len()), ValType::I32);
                let str_ptr = locals.add_local(&format!("__ret_ibox_sp_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(str_len));
                out.push(Instruction::LocalSet(str_ptr));

                out.push(Instruction::I32Const(8));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                let data_ptr = locals.add_local(&format!("__ret_ibox_d_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(data_ptr));
                out.push(Instruction::LocalGet(data_ptr));
                out.push(Instruction::LocalGet(str_ptr));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(data_ptr));
                out.push(Instruction::LocalGet(str_len));
                out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                out.push(Instruction::I32Const(8));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                let wrapper = locals.add_local(&format!("__ret_ibox_w_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(wrapper));
                out.push(Instruction::LocalGet(wrapper));
                out.push(Instruction::I32Const(type_id as i32));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(wrapper));
                out.push(Instruction::LocalGet(data_ptr));
                out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(wrapper));
                return Ok(());
            }
        }

        let (elem_size, _) = Self::elem_size_and_align(rhs_vt);

        let val_tmp = locals.add_local(&format!("__ret_ibox_v_{}", locals.locals.len()), rhs_vt);
        out.push(Instruction::LocalSet(val_tmp));

        let alloc_size = (elem_size as i32).max(8);
        out.push(Instruction::I32Const(alloc_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let data_ptr = locals.add_local(&format!("__ret_ibox_d_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(data_ptr));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::LocalGet(val_tmp));
        let (_, align) = Self::elem_size_and_align(rhs_vt);
        Self::emit_typed_store(rhs_vt, 0, align, out);

        out.push(Instruction::I32Const(8));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        let wrapper = locals.add_local(&format!("__ret_ibox_w_{}", locals.locals.len()), ValType::I32);
        out.push(Instruction::LocalSet(wrapper));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::I32Const(type_id as i32));
        out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        out.push(Instruction::LocalGet(data_ptr));
        out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));
        out.push(Instruction::LocalGet(wrapper));
        Ok(())
    }

    pub(crate) fn infer_concrete_type_name(&self, expr: &ast::Expression, locals: &LocalAlloc) -> String {
        match expr {
            ast::Expression::BasicLit(lit) => {
                use crate::parser::token::LitKind;
                match lit.kind {
                    LitKind::Integer => "int".to_string(),
                    LitKind::Float => "float64".to_string(),
                    LitKind::String => "string".to_string(),
                    LitKind::Imag => "complex128".to_string(),
                    _ => "int".to_string(),
                }
            }
            ast::Expression::Ident(ident) => {
                if let Some(st) = locals.get_var_struct_type(&ident.name) {
                    if st == "__string" {
                        return "string".to_string();
                    }
                    if st.starts_with("__") {
                        return "int".to_string();
                    }
                    return st.to_string();
                }
                if let Some(vt) = locals.find_type(&ident.name) {
                    return Self::type_id_for_val_type(vt).to_string();
                }
                "int".to_string()
            }
            ast::Expression::CompositeLit(comp) => {
                if let ast::Expression::Ident(type_ident) = comp.typ.as_ref() {
                    self.qualify_pkg_name(&type_ident.name)
                } else {
                    "int".to_string()
                }
            }
            ast::Expression::Operation(op) if op.y.is_none() && op.op == Operator::And => {
                self.infer_concrete_type_name(&op.x, locals)
            }
            ast::Expression::Call(call) => {
                let go_type = if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    self.find_func_in_pkg(&ident.name)
                        .and_then(|fi| fi.result_go_types.first().cloned())
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    self.resolve_selector_method_name(sel, locals)
                        .and_then(|q| self.functions.iter().find(|f| f.name == q))
                        .and_then(|fi| fi.result_go_types.first().cloned())
                } else {
                    None
                };
                if let Some(gt) = go_type {
                    let stripped = gt.strip_prefix('*').unwrap_or(&gt);
                    let resolved = self.resolve_struct_in_pkg(stripped);
                    if self.struct_defs.contains_key(&resolved) {
                        return resolved;
                    }
                    return gt;
                }
                let vt = self.infer_val_type(expr, locals);
                Self::type_id_for_val_type(vt).to_string()
            }
            _ => {
                let vt = self.infer_val_type(expr, locals);
                Self::type_id_for_val_type(vt).to_string()
            }
        }
    }

    pub(crate) fn emit_box_value(
        &mut self,
        val_vt: ValType,
        elem_size: i32,
        type_id: u32,
        tid_local: u32,
        data_local: u32,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        if let Some(go_string_idx) = self.gc_builtin_types.go_string {
            if val_vt == Self::gc_ref_val_type(go_string_idx) {
                self.emit_gc_string_to_linear(go_string_idx, out, locals)?;
                let str_len = locals.add_local(&format!("__box_sl_{}", locals.locals.len()), ValType::I32);
                let str_ptr = locals.add_local(&format!("__box_sp_{}", locals.locals.len()), ValType::I32);
                out.push(Instruction::LocalSet(str_len));
                out.push(Instruction::LocalSet(str_ptr));

                out.push(Instruction::I32Const(8));
                out.push(Instruction::Call(self.alloc_func_idx()?));
                out.push(Instruction::LocalSet(data_local));

                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalGet(str_ptr));
                out.push(Instruction::I32Store(MemArg { offset: 0, align: 2, memory_index: 0 }));
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalGet(str_len));
                out.push(Instruction::I32Store(MemArg { offset: 4, align: 2, memory_index: 0 }));

                out.push(Instruction::I32Const(type_id as i32));
                out.push(Instruction::LocalSet(tid_local));
                return Ok(());
            }
        }

        // Value is on top of the stack; save it to a temp
        let tmp = locals.add_local(&format!("__box_tmp_{}", locals.locals.len()), val_vt);
        out.push(Instruction::LocalSet(tmp));

        // Allocate memory for the value
        let alloc_size = (elem_size as i32).max(8);
        out.push(Instruction::I32Const(alloc_size));
        out.push(Instruction::Call(self.alloc_func_idx()?));
        out.push(Instruction::LocalSet(data_local));

        // Store value at allocated address
        out.push(Instruction::LocalGet(data_local));
        out.push(Instruction::LocalGet(tmp));
        let (_, align) = Self::elem_size_and_align(val_vt);
        Self::emit_typed_store(val_vt, 0, align, out);

        // Set type_id
        out.push(Instruction::I32Const(type_id as i32));
        out.push(Instruction::LocalSet(tid_local));

        Ok(())
    }

    pub(crate) fn check_interface_nil_cmp(
        &self,
        lhs: &ast::Expression,
        rhs: &ast::Expression,
        locals: &LocalAlloc,
    ) -> (String, bool) {
        // Check: lhs is interface var, rhs is nil (or vice versa)
        if let ast::Expression::Ident(id) = lhs {
            if self.is_interface_var(&id.name, locals) || self.is_interface_var_expr(lhs, locals) {
                if let ast::Expression::Ident(rhs_id) = rhs {
                    if rhs_id.name == "nil" {
                        return (id.name.clone(), true);
                    }
                }
            }
        }
        if let ast::Expression::Ident(id) = rhs {
            if self.is_interface_var(&id.name, locals) || self.is_interface_var_expr(rhs, locals) {
                if let ast::Expression::Ident(lhs_id) = lhs {
                    if lhs_id.name == "nil" {
                        return (id.name.clone(), true);
                    }
                }
            }
        }
        (String::new(), false)
    }

    pub(crate) fn is_interface_var(&self, name: &str, locals: &LocalAlloc) -> bool {
        let sym_says = self.is_sym_interface_var(name);
        let old_says = if let Some(st) = locals.get_var_struct_type(name) {
            st == "__interface" || st.starts_with("__iface_")
        } else {
            false
        };
        sym_says || old_says
    }

    pub(crate) fn is_interface_var_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Ident(id) = expr {
            if self.is_interface_var(&id.name, locals) {
                return true;
            }
            if self.global_vars.contains_key(&format!("{}_tid", id.name)) {
                return true;
            }
            if let Some(ref pkg) = self.current_package {
                if self.global_vars.contains_key(&format!("{}.{}_tid", pkg, id.name)) {
                    return true;
                }
            }
            false
        } else {
            false
        }
    }

    pub(crate) fn is_interface_field_selector(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Selector(sel) = expr {
            if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                if let Some(struct_def) = self.struct_defs.get(&type_name) {
                    if let Some(field) = struct_def.find_field(&sel.sel.name) {
                        return field.go_type_tag.as_deref() == Some("__interface");
                    }
                }
            }
        }
        false
    }

    pub(crate) fn is_call_returning_interface(&self, expr: &ast::Expression, locals: &LocalAlloc) -> bool {
        if let ast::Expression::Call(call_expr) = expr {
            if let ast::Expression::Ident(fn_ident) = call_expr.func.as_ref() {
                if let Some(fi) = self.functions.iter().find(|f| f.name == fn_ident.name) {
                    if let Some(go_type) = fi.result_go_types.first() {
                        return self.is_iface_go_type(go_type);
                    }
                }
                if let Some(fi) = self.find_func_in_pkg(&fn_ident.name) {
                    if let Some(go_type) = fi.result_go_types.first() {
                        return self.is_iface_go_type(go_type);
                    }
                }
            } else if let ast::Expression::Selector(sel) = call_expr.func.as_ref() {
                if let Some(qualified) = self.resolve_selector_method_name(sel, locals) {
                    if let Some(fi) = self.functions.iter().find(|f| f.name == qualified) {
                        if let Some(go_type) = fi.result_go_types.first() {
                            return self.is_iface_go_type(go_type);
                        }
                    }
                }
                // Interface method call: check return type from method signatures
                if let ast::Expression::Ident(recv_id) = sel.x.as_ref() {
                    if self.is_interface_var(&recv_id.name, locals) {
                        let method_name = &sel.sel.name;
                        // Check all registered interface method sigs
                        for (_iface, sigs) in &self.iface_method_sigs {
                            if let Some((_params, results)) = sigs.get(method_name.as_str()) {
                                if !results.is_empty() && results[0] == WasmType::I32 {
                                    // Look at concrete implementations for the go_type
                                    for fi in &self.functions {
                                        if fi.name.ends_with(&format!(".{}", method_name)) && fi.recv_type.is_some() {
                                            if let Some(go_type) = fi.result_go_types.first() {
                                                if self.is_iface_go_type(go_type) {
                                                    return true;
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
        false
    }

    pub(crate) fn get_iface_type_id_local(&self, name: &str, locals: &LocalAlloc) -> Option<u32> {
        locals.iface_type_id_locals.get(name).copied()
    }

    pub(crate) fn compile_iface_expr_to_locals(
        &mut self,
        expr: &ast::Expression,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(u32, u32), Error> {
        if let ast::Expression::Ident(id) = expr {
            if let Some(tid_local) = self.get_iface_type_id_local(&id.name, locals) {
                let data_local = locals.find(&id.name).ok_or_else(|| {
                    Error::InternalError(format!("interface variable '{}' not found", id.name))
                })?;
                return Ok((data_local, tid_local));
            }

            let resolved = self.resolve_global_var_name(&id.name);
            let tid_key = format!("{}_tid", resolved);
            if let Some(&(tid_global, _)) = self.global_vars.get(&tid_key) {
                let &(data_global, _) = self.resolve_global_var(&id.name).ok_or_else(|| {
                    Error::InternalError(format!("global interface '{}' data not found", id.name))
                })?;
                let data_tmp = locals.add_local(
                    &format!("__iface_cmp_data_{}", locals.locals.len()),
                    ValType::I32,
                );
                let tid_tmp = locals.add_local(
                    &format!("__iface_cmp_tid_{}", locals.locals.len()),
                    ValType::I32,
                );
                out.push(Instruction::GlobalGet(data_global));
                out.push(Instruction::LocalSet(data_tmp));
                out.push(Instruction::GlobalGet(tid_global));
                out.push(Instruction::LocalSet(tid_tmp));
                return Ok((data_tmp, tid_tmp));
            }
        }

        if self.is_interface_field_selector(expr, locals) {
            let tid_tmp = locals.add_local(
                &format!("__iface_cmp_tid_{}", locals.locals.len()),
                ValType::I32,
            );
            let data_tmp = locals.add_local(
                &format!("__iface_cmp_data_{}", locals.locals.len()),
                ValType::I32,
            );
            self.compile_expression(expr, out, locals)?;
            out.push(Instruction::LocalSet(tid_tmp));
            out.push(Instruction::LocalSet(data_tmp));
            return Ok((data_tmp, tid_tmp));
        }

        Err(Error::InternalError(format!(
            "expression is not an interface-typed expression"
        )))
    }

    pub(crate) fn types_implementing_interface(&self, iface_name: &str) -> Vec<u32> {
        let iface_methods = match self.iface_defs.get(iface_name) {
            Some(methods) => methods,
            None => return Vec::new(),
        };
        let iface_sigs = self.iface_method_sigs.get(iface_name);

        let mut result = Vec::new();
        for (type_name, &type_id) in &self.type_registry {
            if type_name == "nil" || self.iface_defs.contains_key(type_name) {
                continue;
            }
            let has_all_methods = iface_methods.iter().all(|method| {
                let qualified = format!("{}.{}", type_name, method);
                if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                    if let Some(sigs) = iface_sigs {
                        if let Some((expected_params, expected_results)) = sigs.get(method) {
                            let impl_params: Vec<WasmType> = f.params.iter()
                                .skip(1) // skip receiver
                                .map(|(_, wt)| *wt)
                                .collect();
                            let impl_results: Vec<WasmType> = f.results.clone();
                            return impl_params == *expected_params && impl_results == *expected_results;
                        }
                    }
                    true
                } else {
                    false
                }
            });
            if has_all_methods {
                result.push(type_id);
            }
        }
        result
    }

    pub(crate) fn check_iface_satisfaction(&self, iface_name: &str, concrete_type: &str) -> Result<(), Error> {
        if iface_name == "any" || concrete_type == "nil" {
            return Ok(());
        }
        let iface_methods = match self.iface_defs.get(iface_name) {
            Some(methods) => methods,
            None => return Ok(()),
        };
        let iface_sigs = self.iface_method_sigs.get(iface_name);
        for method in iface_methods {
            let qualified = format!("{}.{}", concrete_type, method);
            if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                if let Some(sigs) = iface_sigs {
                    if let Some((expected_params, expected_results)) = sigs.get(method) {
                        let impl_params: Vec<WasmType> = f.params.iter()
                            .skip(1)
                            .map(|(_, wt)| *wt)
                            .collect();
                        if impl_params != *expected_params || f.results != *expected_results {
                            return Err(Error::TypeError(format!(
                                "type '{}' does not implement interface '{}': method '{}' has wrong signature",
                                concrete_type, iface_name, method
                            )));
                        }
                    }
                }
            } else {
                return Err(Error::TypeError(format!(
                    "type '{}' does not implement interface '{}': missing method '{}'",
                    concrete_type, iface_name, method
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn register_anon_interface(&mut self, iface: &ast::InterfaceType) -> String {
        let name = format!("__anon_iface_{}", self.next_anon_iface_id);
        self.next_anon_iface_id += 1;

        let mut methods = Vec::new();
        let mut method_sigs: HashMap<String, (Vec<WasmType>, Vec<WasmType>)> = HashMap::new();

        for field in &iface.methods.list {
            if field.name.is_empty() {
                if let ast::Expression::Ident(embedded_id) = &field.typ {
                    if let Some(embedded_methods) = self.iface_defs.get(&embedded_id.name).cloned() {
                        methods.extend(embedded_methods);
                    }
                    if let Some(embedded_sigs) = self.iface_method_sigs.get(&embedded_id.name).cloned() {
                        method_sigs.extend(embedded_sigs);
                    }
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

        self.iface_defs.insert(name.clone(), methods);
        self.iface_method_sigs.insert(name.clone(), method_sigs);
        name
    }

    pub(crate) fn resolve_type_assert_target(&mut self, target_type: &ast::Expression) -> Result<String, Error> {
        if let ast::Expression::TypeInterface(iface) = target_type {
            return Ok(self.register_anon_interface(iface));
        }
        Self::extract_type_name_from_expr(target_type).ok_or_else(|| {
            Error::InternalError("type assertion target must be a named type".to_string())
        })
    }

    pub(crate) fn compile_type_assert(
        &mut self,
        ta: &ast::TypeAssertion,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<GoType, Error> {
        // x.(T) -- single-value form (panics on mismatch)
        let target_type = ta.right.as_ref().ok_or_else(|| {
            Error::InternalError("type assertion without target type".to_string())
        })?;

        let target_type_name = self.resolve_type_assert_target(target_type)?;

        // Get the interface variable -- either from a named var or by compiling the expression
        let (tid_local, data_local) = if let ast::Expression::Ident(ident) = ta.left.as_ref() {
            let tid = self.get_iface_type_id_local(&ident.name, locals).ok_or_else(|| {
                Error::InternalError(format!("'{}' is not an interface variable", ident.name))
            })?;
            let data = locals.find(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("variable '{}' not found", ident.name))
            })?;
            (tid, data)
        } else {
            self.compile_expression(&ta.left, out, locals)?;
            let wrapper = locals.add_local(
                &format!("__ta_wrap_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(wrapper));

            let tid = locals.add_local(
                &format!("__ta_tid_{}", locals.locals.len()),
                ValType::I32,
            );
            let data = locals.add_local(
                &format!("__ta_data_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(tid));
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(data));
            (tid, data)
        };

        let lookup_type_name = target_type_name.strip_prefix('*').unwrap_or(&target_type_name).to_string();

        // Check if target is an interface type
        if self.iface_defs.contains_key(&lookup_type_name)
            || lookup_type_name == "any"
            || lookup_type_name == "error"
        {
            if lookup_type_name == "any" {
                // any always succeeds: pass through the interface value
                out.push(Instruction::LocalGet(data_local));
                return Ok(GoType::Interface);
            }

            let valid_type_ids = self.types_implementing_interface(&lookup_type_name);
            if valid_type_ids.is_empty() {
                out.push(Instruction::Unreachable);
                return Ok(GoType::Interface);
            }

            // Check if type_id matches any implementing type
            let match_local = locals.add_local(
                &format!("__ta_imatch_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(match_local));

            for tid in &valid_type_ids {
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::I32Const(*tid as i32));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(match_local));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(match_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            out.push(Instruction::Unreachable);
            out.push(Instruction::End);

            // Pass through the data pointer (interface value stays as-is)
            out.push(Instruction::LocalGet(data_local));
            return Ok(GoType::Interface);
        }

        let target_id = self.get_or_create_type_id(&lookup_type_name);
        let target_vt = Self::val_type_for_type_name(&target_type_name);

        // Check type_id matches target
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Const(target_id as i32));
        out.push(Instruction::I32Ne);
        out.push(Instruction::If(BlockType::Empty));
        out.push(Instruction::Unreachable); // panic on mismatch
        out.push(Instruction::End);

        // Load value from data_ptr
        out.push(Instruction::LocalGet(data_local));
        let (_, align) = Self::elem_size_and_align(target_vt);
        Self::emit_typed_load(target_vt, 0, align, out);

        Ok(GoType::from_val_type(target_vt))
    }

    pub(crate) fn compile_type_assert_ok(
        &mut self,
        ta: &ast::TypeAssertion,
        val_var: &str,
        ok_var: &str,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let target_type = ta.right.as_ref().ok_or_else(|| {
            Error::InternalError("type assertion without target type".to_string())
        })?;

        let target_type_name = self.resolve_type_assert_target(target_type)?;

        let (tid_local, data_local) = if let ast::Expression::Ident(ident) = ta.left.as_ref() {
            let tid = self.get_iface_type_id_local(&ident.name, locals).ok_or_else(|| {
                Error::InternalError(format!("'{}' is not an interface variable", ident.name))
            })?;
            let data = locals.find(&ident.name).ok_or_else(|| {
                Error::InternalError(format!("variable '{}' not found", ident.name))
            })?;
            (tid, data)
        } else {
            self.compile_expression(&ta.left, out, locals)?;
            let wrapper = locals.add_local(
                &format!("__taok_wrap_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(wrapper));

            let tid = locals.add_local(
                &format!("__taok_tid_{}", locals.locals.len()),
                ValType::I32,
            );
            let data = locals.add_local(
                &format!("__taok_data_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(tid));
            out.push(Instruction::LocalGet(wrapper));
            out.push(Instruction::I32Load(MemArg { offset: 4, align: 2, memory_index: 0 }));
            out.push(Instruction::LocalSet(data));
            (tid, data)
        };

        let lookup_type_name = target_type_name.strip_prefix('*').unwrap_or(&target_type_name).to_string();

        // Check if target is an interface type
        if self.iface_defs.contains_key(&lookup_type_name)
            || lookup_type_name == "any"
            || lookup_type_name == "error"
        {
            let val_local = locals.add_local(val_var, ValType::I32);
            let ok_local = locals.add_local(ok_var, ValType::I32);
            locals.set_var_struct_type(val_var, "__interface");

            // Create a type_id local for the result interface variable
            let val_tid_local = locals.add_local(
                &format!("{}__iface_tid", val_var),
                ValType::I32,
            );

            // Reset ok to 0 so that re-evaluation in a loop works correctly
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(ok_local));

            if lookup_type_name == "any" {
                // any always succeeds
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalSet(val_local));
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::LocalSet(val_tid_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
                locals.iface_type_id_locals.insert(val_var.to_string(), val_tid_local);
                return Ok(());
            }

            let valid_type_ids = self.types_implementing_interface(&lookup_type_name);

            let match_local = locals.add_local(
                &format!("__taok_imatch_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::I32Const(0));
            out.push(Instruction::LocalSet(match_local));

            for tid in &valid_type_ids {
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::I32Const(*tid as i32));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(match_local));
                out.push(Instruction::End);
            }

            out.push(Instruction::LocalGet(match_local));
            out.push(Instruction::If(BlockType::Empty));
            {
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::LocalSet(val_local));
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::LocalSet(val_tid_local));
                out.push(Instruction::I32Const(1));
                out.push(Instruction::LocalSet(ok_local));
            }
            out.push(Instruction::End);

            locals.iface_type_id_locals.insert(val_var.to_string(), val_tid_local);

            return Ok(());
        }

        let target_id = self.get_or_create_type_id(&lookup_type_name);
        let target_vt = Self::val_type_for_type_name(&target_type_name);

        let val_local = locals.add_local(val_var, target_vt);
        let ok_local = locals.add_local(ok_var, ValType::I32);

        if self.struct_defs.contains_key(&lookup_type_name) {
            locals.set_var_struct_type(val_var, &lookup_type_name);
        } else {
            let resolved = self.resolve_struct_in_pkg(&lookup_type_name);
            if self.struct_defs.contains_key(&resolved) {
                locals.set_var_struct_type(val_var, &resolved);
            }
        }

        // Reset ok to 0 so that re-evaluation in a loop works correctly
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(ok_local));

        // Check type_id matches target
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Const(target_id as i32));
        out.push(Instruction::I32Eq);
        out.push(Instruction::If(BlockType::Empty));
        {
            // Match: load value and set ok=1
            out.push(Instruction::LocalGet(data_local));
            let (_, align) = Self::elem_size_and_align(target_vt);
            Self::emit_typed_load(target_vt, 0, align, out);
            out.push(Instruction::LocalSet(val_local));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::LocalSet(ok_local));
        }
        out.push(Instruction::End);
        // If no match, val_local stays zero-initialized, ok_local stays 0

        Ok(())
    }

    pub(crate) fn compile_type_switch_labeled(
        &mut self,
        ts: &ast::TypeSwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
        label: Option<String>,
    ) -> Result<(), Error> {
        if label.is_some() {
            out.push(Instruction::Block(BlockType::Empty));
            self.loop_depth.push((label, 0, false, false));
        }
        let r = self.compile_type_switch(ts, out, locals, result_types);
        if self.loop_depth.last().map_or(false, |e| !e.3 && e.0.is_some()) {
            self.loop_depth.pop();
            out.push(Instruction::End);
        }
        r
    }

    pub(crate) fn compile_type_switch(
        &mut self,
        ts: &ast::TypeSwitchStmt,
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
        result_types: &[ValType],
    ) -> Result<(), Error> {
        for clause in &ts.block.body {
            if clause.body.iter().any(|s| Self::contains_fallthrough(s)) {
                return Err(Error::SyntaxError(
                    "cannot fallthrough in type switch".to_string(),
                ));
            }
        }

        let has_init = ts.init.is_some();
        if has_init {
            locals.push_scope();
        }

        // Compile init statement if present
        if let Some(ref init) = ts.init {
            self.compile_statement(init, out, locals, result_types)?;
        }

        // Extract interface variable and optional binding name from tag
        // tag is: v := x.(type)  OR  x.(type)
        // The tag is stored as a Statement (an assignment or expression statement)
        let (iface_var_name, bind_name) = self.extract_type_switch_guard(ts)?;

        let tid_local = self.get_iface_type_id_local(&iface_var_name, locals).ok_or_else(|| {
            Error::InternalError(format!("'{}' is not an interface variable", iface_var_name))
        })?;
        let data_local = locals.find(&iface_var_name).ok_or_else(|| {
            Error::InternalError(format!("variable '{}' not found", iface_var_name))
        })?;

        // Store type_id in a temp
        let type_id_tmp = locals.add_local(
            &format!("__tsw_tid_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::LocalSet(type_id_tmp));

        let matched_local = locals.add_local(
            &format!("__tsw_matched_{}", locals.locals.len()),
            ValType::I32,
        );
        out.push(Instruction::I32Const(0));
        out.push(Instruction::LocalSet(matched_local));

        let mut default_body: Option<&Vec<ast::Statement>> = None;

        for clause in &ts.block.body {
            if clause.list.is_empty() {
                // Default case
                default_body = Some(clause.body.as_ref());
                continue;
            }

            // case T1, T2, ...:
            // Check if type_id matches any of the case types
            out.push(Instruction::LocalGet(matched_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            {
                // Build OR of type matches
                let mut first = true;
                for case_expr in &clause.list {
                    let type_name = Self::extract_type_name_from_expr(case_expr);
                    if let Some(ref tn) = type_name {
                        if tn == "nil" {
                            out.push(Instruction::LocalGet(type_id_tmp));
                            out.push(Instruction::I32Eqz);
                        } else {
                            let lookup_name = tn.strip_prefix('*').unwrap_or(tn);
                            let case_type_id = self.get_or_create_type_id(lookup_name);
                            out.push(Instruction::LocalGet(type_id_tmp));
                            out.push(Instruction::I32Const(case_type_id as i32));
                            out.push(Instruction::I32Eq);
                        }
                        if !first {
                            out.push(Instruction::I32Or);
                        }
                        first = false;
                    }
                }

                if first {
                    out.push(Instruction::I32Const(0));
                }

                out.push(Instruction::If(BlockType::Empty));
                {
                    out.push(Instruction::I32Const(1));
                    out.push(Instruction::LocalSet(matched_local));

                    if let Some(ref bind) = bind_name {
                        if clause.list.len() == 1 {
                            let case_type_name = Self::extract_type_name_from_expr(&clause.list[0]);
                            if let Some(ref tn) = case_type_name {
                                if tn != "nil" {
                                    let bind_vt = Self::val_type_for_type_name(tn);
                                    let bind_local = locals.add_local(bind, bind_vt);
                                    out.push(Instruction::LocalGet(data_local));
                                    let (_, align) = Self::elem_size_and_align(bind_vt);
                                    Self::emit_typed_load(bind_vt, 0, align, out);
                                    out.push(Instruction::LocalSet(bind_local));
                                }
                            }
                        } else {
                            // Multiple types: bind as the interface value (I32 pointer)
                            let bind_local = locals.add_local(bind, ValType::I32);
                            out.push(Instruction::LocalGet(data_local));
                            out.push(Instruction::LocalSet(bind_local));
                        }
                    }

                    self.compile_block_stmts(clause.body.as_ref(), out, locals, result_types)?;
                }
                out.push(Instruction::End);
            }
            out.push(Instruction::End);
        }

        // Default case
        if let Some(default) = default_body {
            out.push(Instruction::LocalGet(matched_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::If(BlockType::Empty));
            {
                self.compile_block_stmts(default, out, locals, result_types)?;
            }
            out.push(Instruction::End);
        }

        if has_init {
            locals.pop_scope();
        }

        Ok(())
    }

    pub(crate) fn extract_type_name_from_expr(expr: &ast::Expression) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => Some(ident.name.clone()),
            ast::Expression::Star(star) => {
                let inner = Self::extract_type_name_from_expr(&star.right)?;
                Some(format!("*{}", inner))
            }
            ast::Expression::TypePointer(ptr) => {
                let inner = Self::extract_type_name_from_expr(&ptr.typ)?;
                Some(format!("*{}", inner))
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    Some(format!("{}.{}", pkg.name, sel.sel.name))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub(crate) fn extract_type_switch_guard(
        &self,
        ts: &ast::TypeSwitchStmt,
    ) -> Result<(String, Option<String>), Error> {
        if let Some(ref tag) = ts.tag {
            match tag.as_ref() {
                ast::Statement::Assign(assign) => {
                    // v := x.(type)
                    let bind_name = if let Some(ast::Expression::Ident(id)) = assign.left.first() {
                        Some(id.name.clone())
                    } else {
                        None
                    };
                    if let Some(ast::Expression::TypeAssert(ta)) = assign.right.first() {
                        if let ast::Expression::Ident(ident) = ta.left.as_ref() {
                            return Ok((ident.name.clone(), bind_name));
                        }
                    }
                    Err(Error::InternalError(
                        "type switch guard must be a type assertion".to_string(),
                    ))
                }
                ast::Statement::Expr(expr_stmt) => {
                    // x.(type)
                    if let ast::Expression::TypeAssert(ta) = &expr_stmt.expr {
                        if let ast::Expression::Ident(ident) = ta.left.as_ref() {
                            return Ok((ident.name.clone(), None));
                        }
                    }
                    Err(Error::InternalError(
                        "type switch guard must be a type assertion".to_string(),
                    ))
                }
                _ => Err(Error::InternalError(
                    "type switch guard must be an assignment or expression".to_string(),
                )),
            }
        } else {
            Err(Error::InternalError(
                "type switch statement missing guard".to_string(),
            ))
        }
    }

    pub(crate) fn compile_interface_method_call(
        &mut self,
        iface_var: &str,
        method_name: &str,
        args: &[ast::Expression],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        let tid_local = self.get_iface_type_id_local(iface_var, locals).ok_or_else(|| {
            Error::InternalError(format!("'{}' is not an interface variable", iface_var))
        })?;
        let data_local = locals.find(iface_var).ok_or_else(|| {
            Error::InternalError(format!("variable '{}' not found", iface_var))
        })?;

        self.compile_interface_method_call_with_locals(tid_local, data_local, method_name, args, out, locals)
    }

    pub(crate) fn compile_interface_method_call_with_locals(
        &mut self,
        tid_local: u32,
        data_local: u32,
        method_name: &str,
        args: &[ast::Expression],
        out: &mut Vec<Instruction<'static>>,
        locals: &mut LocalAlloc,
    ) -> Result<(), Error> {
        // Nil check: if type_id == 0, panic with a descriptive message
        out.push(Instruction::LocalGet(tid_local));
        out.push(Instruction::I32Eqz);
        out.push(Instruction::If(BlockType::Empty));
        {
            let msg = format!("runtime error: nil pointer dereference (calling method {} on nil interface)", method_name);
            let msg_bytes = msg.as_bytes();
            let msg_len = msg_bytes.len() as i32;
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::Call(self.alloc_func_idx()?));
            let msg_ptr_local = locals.add_local(
                &format!("__nil_panic_ptr_{}", locals.locals.len()),
                ValType::I32,
            );
            out.push(Instruction::LocalSet(msg_ptr_local));
            for (j, &byte) in msg_bytes.iter().enumerate() {
                out.push(Instruction::LocalGet(msg_ptr_local));
                out.push(Instruction::I32Const(byte as i32));
                out.push(Instruction::I32Store8(MemArg {
                    offset: j as u64,
                    align: 0,
                    memory_index: 0,
                }));
            }
            out.push(Instruction::LocalGet(msg_ptr_local));
            out.push(Instruction::GlobalSet(self.panic_value_ptr_global));
            out.push(Instruction::I32Const(msg_len));
            out.push(Instruction::GlobalSet(self.panic_value_len_global));
            out.push(Instruction::I32Const(1));
            out.push(Instruction::GlobalSet(self.panicking_global));
            out.push(Instruction::Unreachable);
        }
        out.push(Instruction::End);

        // Compile arguments to temp locals
        let mut arg_locals = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            self.compile_expression(arg, out, locals)?;
            let arg_vt = self.infer_val_type(arg, locals);
            let arg_local = locals.add_local(
                &format!("__imc_arg_{}_{}", i, locals.locals.len()),
                arg_vt,
            );
            out.push(Instruction::LocalSet(arg_local));
            arg_locals.push((arg_local, arg_vt));
        }

        // Find all concrete types that implement this method
        let candidates: Vec<(u32, u32, Vec<ValType>, Vec<String>)> = self
            .functions
            .iter()
            .filter_map(|f| {
                let type_name = f.recv_type.as_ref()?;
                if !f.name.ends_with(&format!(".{}", method_name)) {
                    return None;
                }
                let type_id = self.type_registry.get(type_name).copied().unwrap_or(0);
                let result_types: Vec<ValType> = f.results.iter().map(|r| r.to_val_type()).collect();
                Some((type_id, f.wasm_func_idx, result_types, f.result_go_types.clone()))
            })
            .collect();

        if candidates.is_empty() {
            self.last_iface_call_returns_iface = true;
            out.push(Instruction::Unreachable);
            return Ok(());
        }

        // Track whether this method returns an interface type
        self.last_iface_call_returns_iface = candidates[0].3.first()
            .map_or(false, |gt| self.is_iface_go_type(gt));

        // Determine result types from first candidate
        let result_vts: Vec<ValType> = candidates[0].2.clone();

        // Create locals for each return value
        let mut result_locals: Vec<(u32, ValType)> = Vec::new();
        for (j, &vt) in result_vts.iter().enumerate() {
            let rl = locals.add_local(
                &format!("__imc_res_{}_{}", j, locals.locals.len()),
                vt,
            );
            result_locals.push((rl, vt));
        }

        // Try vtable-based dispatch if itab is populated
        let iface_info = self.find_iface_method_info(method_name);
        if self.itab_base > 0 && self.max_iface_methods > 0 && iface_info.is_some() {
            let (iface_id, method_idx) = iface_info.unwrap();

            // Build the function type for call_indirect (deduplicated)
            let mut param_types: Vec<ValType> = vec![ValType::I32]; // receiver
            for (_, vt) in &arg_locals {
                param_types.push(*vt);
            }
            let call_type_idx = self.get_or_create_call_indirect_type(&param_types, &result_vts);

            // Compute itab pointer:
            // itab_ptr = itab_base + (tid * max_ifaces + iface_id) * (max_methods * 4)
            // func_idx = i32.load(itab_ptr + method_index * 4)
            let func_idx_local = locals.add_local(
                &format!("__vtbl_fidx_{}", locals.locals.len()),
                ValType::I32,
            );

            let max_ifaces = self.next_iface_id;
            let entry_size = self.max_iface_methods * 4;

            out.push(Instruction::I32Const(self.itab_base as i32));
            out.push(Instruction::LocalGet(tid_local));
            out.push(Instruction::I32Const(max_ifaces as i32));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Const(iface_id as i32));
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Const(entry_size as i32));
            out.push(Instruction::I32Mul);
            out.push(Instruction::I32Add);
            out.push(Instruction::I32Load(MemArg {
                offset: (method_idx * 4) as u64,
                align: 2,
                memory_index: 0,
            }));
            out.push(Instruction::LocalSet(func_idx_local));

            // If func_idx is 0, fall through to if/else chain
            out.push(Instruction::LocalGet(func_idx_local));
            out.push(Instruction::I32Eqz);
            out.push(Instruction::I32Eqz); // func_idx != 0
            out.push(Instruction::If(BlockType::Empty));
            {
                // Push receiver (unwrap box)
                out.push(Instruction::LocalGet(data_local));
                out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                for (arg_local, _) in &arg_locals {
                    out.push(Instruction::LocalGet(*arg_local));
                }
                out.push(Instruction::LocalGet(func_idx_local));
                out.push(Instruction::CallIndirect { type_index: call_type_idx, table_index: 0 });
                for (rl, _) in result_locals.iter().rev() {
                    out.push(Instruction::LocalSet(*rl));
                }
            }
            out.push(Instruction::Else);
            {
                // Fallback: if/else chain for types not in itab
                for (type_id, func_idx, _result_types, _result_go_types) in candidates.iter() {
                    out.push(Instruction::LocalGet(tid_local));
                    out.push(Instruction::I32Const(*type_id as i32));
                    out.push(Instruction::I32Eq);
                    out.push(Instruction::If(BlockType::Empty));
                    {
                        out.push(Instruction::LocalGet(data_local));
                        out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                        for (arg_local, _) in &arg_locals {
                            out.push(Instruction::LocalGet(*arg_local));
                        }
                        out.push(Instruction::Call(*func_idx));
                        for (rl, _) in result_locals.iter().rev() {
                            out.push(Instruction::LocalSet(*rl));
                        }
                    }
                    out.push(Instruction::End);
                }
            }
            out.push(Instruction::End);

            self.needs_func_table = true;
        } else {
            // Fallback: if/else chain on type_id (original behavior)
            for (type_id, func_idx, _result_types, _result_go_types) in candidates.iter() {
                out.push(Instruction::LocalGet(tid_local));
                out.push(Instruction::I32Const(*type_id as i32));
                out.push(Instruction::I32Eq);
                out.push(Instruction::If(BlockType::Empty));
                {
                    out.push(Instruction::LocalGet(data_local));
                    out.push(Instruction::I32Load(MemArg { offset: 0, align: 2, memory_index: 0 }));
                    for (arg_local, _) in &arg_locals {
                        out.push(Instruction::LocalGet(*arg_local));
                    }
                    out.push(Instruction::Call(*func_idx));
                    for (rl, _) in result_locals.iter().rev() {
                        out.push(Instruction::LocalSet(*rl));
                    }
                }
                out.push(Instruction::End);
            }
        }

        // Push results
        for (rl, _) in &result_locals {
            out.push(Instruction::LocalGet(*rl));
        }

        Ok(())
    }

    /// Returns `(iface_id, method_index)` for the first interface that contains
    /// `method_name`, using a single HashMap traversal to avoid inconsistency.
    fn find_iface_method_info(&self, method_name: &str) -> Option<(u32, u32)> {
        for (iface_name, methods) in &self.iface_defs {
            for (i, m) in methods.iter().enumerate() {
                if m == method_name {
                    let iface_id = self.iface_ids.get(iface_name).copied().unwrap_or(0);
                    return Some((iface_id, i as u32));
                }
            }
        }
        None
    }
}
