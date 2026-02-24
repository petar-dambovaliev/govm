use super::*;

impl WasmCompiler {
    pub(crate) fn is_comparable_type(type_name: &str) -> bool {
        matches!(
            type_name,
            "int" | "int8" | "int16" | "int32" | "int64"
            | "uint" | "uint8" | "uint16" | "uint32" | "uint64"
            | "uintptr" | "float32" | "float64" | "complex64" | "complex128"
            | "bool" | "string" | "byte" | "rune"
        )
    }

    pub(crate) fn validate_type_constraints(
        template: &ast::FuncDecl,
        subst: &HashMap<String, String>,
    ) -> Result<(), Error> {
        for field in &template.typ.typ_params.list {
            let constraint_name = match &field.typ {
                ast::Expression::Ident(id) => Some(id.name.as_str()),
                _ => None,
            };
            if constraint_name == Some("comparable") {
                for name_ident in &field.name {
                    if let Some(concrete) = subst.get(&name_ident.name) {
                        if !Self::is_comparable_type(concrete) {
                            return Err(Error::TypeError(format!(
                                "{} does not satisfy comparable constraint",
                                concrete
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn monomorphize_func_decl(
        &self,
        template: &ast::FuncDecl,
        mono_name: &str,
        subst: &HashMap<String, String>,
    ) -> ast::FuncDecl {
        let mut decl = template.clone();
        decl.name = ast::Ident {
            name: mono_name.to_string(),
            pos: template.name.pos,
        };
        // Clear type params so the specialized function is not treated as generic
        decl.typ.typ_params = ast::FieldList { pos: None, list: vec![] };

        // Substitute type parameters in function parameter types
        for field in &mut decl.typ.params.list {
            Self::subst_type_in_expr(&mut field.typ, subst);
        }
        // Substitute in return types
        for field in &mut decl.typ.result.list {
            Self::subst_type_in_expr(&mut field.typ, subst);
        }
        // Substitute in body
        if let Some(ref mut body) = decl.body {
            Self::subst_type_in_block(body, subst);
        }

        decl
    }

    pub(crate) fn subst_type_in_expr(expr: &mut ast::Expression, subst: &HashMap<String, String>) {
        match expr {
            ast::Expression::Ident(ident) => {
                if let Some(replacement) = subst.get(&ident.name) {
                    ident.name = replacement.clone();
                }
            }
            ast::Expression::Call(call) => {
                Self::subst_type_in_expr(&mut call.func, subst);
                for arg in &mut call.args {
                    Self::subst_type_in_expr(arg, subst);
                }
            }
            ast::Expression::Operation(op) => {
                Self::subst_type_in_expr(&mut op.x, subst);
                if let Some(ref mut y) = op.y {
                    Self::subst_type_in_expr(y, subst);
                }
            }
            ast::Expression::Paren(p) => {
                Self::subst_type_in_expr(&mut p.expr, subst);
            }
            ast::Expression::Index(idx) => {
                if let Some(ref mut left) = idx.left {
                    Self::subst_type_in_expr(left, subst);
                }
                Self::subst_type_in_expr(&mut idx.index, subst);
            }
            ast::Expression::Selector(sel) => {
                Self::subst_type_in_expr(&mut sel.x, subst);
            }
            ast::Expression::Star(star) => {
                Self::subst_type_in_expr(&mut star.right, subst);
            }
            ast::Expression::TypeSlice(sl) => {
                Self::subst_type_in_expr(&mut sl.typ, subst);
            }
            ast::Expression::TypeArray(arr) => {
                Self::subst_type_in_expr(&mut arr.typ, subst);
                Self::subst_type_in_expr(&mut arr.len, subst);
            }
            ast::Expression::TypeMap(m) => {
                Self::subst_type_in_expr(&mut m.key, subst);
                Self::subst_type_in_expr(&mut m.val, subst);
            }
            ast::Expression::TypePointer(p) => {
                Self::subst_type_in_expr(&mut p.typ, subst);
            }
            ast::Expression::CompositeLit(cl) => {
                Self::subst_type_in_expr(&mut cl.typ, subst);
            }
            ast::Expression::TypeAssert(ta) => {
                Self::subst_type_in_expr(&mut ta.left, subst);
                if let Some(ref mut right) = ta.right {
                    Self::subst_type_in_expr(right, subst);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn subst_type_in_stmt(stmt: &mut ast::Statement, subst: &HashMap<String, String>) {
        match stmt {
            ast::Statement::Expr(es) => Self::subst_type_in_expr(&mut es.expr, subst),
            ast::Statement::Assign(a) => {
                for e in &mut a.left {
                    Self::subst_type_in_expr(e, subst);
                }
                for e in &mut a.right {
                    Self::subst_type_in_expr(e, subst);
                }
            }
            ast::Statement::Return(r) => {
                for e in &mut r.ret {
                    Self::subst_type_in_expr(e, subst);
                }
            }
            ast::Statement::If(i) => {
                if let Some(ref mut init) = i.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                Self::subst_type_in_expr(&mut i.cond, subst);
                Self::subst_type_in_block(&mut i.body, subst);
                if let Some(ref mut els) = i.else_ {
                    Self::subst_type_in_stmt(els, subst);
                }
            }
            ast::Statement::For(f) => {
                if let Some(ref mut init) = f.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                if let Some(ref mut cond) = f.cond {
                    Self::subst_type_in_stmt(cond, subst);
                }
                if let Some(ref mut post) = f.post {
                    Self::subst_type_in_stmt(post, subst);
                }
                Self::subst_type_in_block(&mut f.body, subst);
            }
            ast::Statement::Range(r) => {
                if let Some(ref mut key) = r.key {
                    Self::subst_type_in_expr(key, subst);
                }
                if let Some(ref mut val) = r.value {
                    Self::subst_type_in_expr(val, subst);
                }
                Self::subst_type_in_expr(&mut r.expr, subst);
                Self::subst_type_in_block(&mut r.body, subst);
            }
            ast::Statement::Block(b) => Self::subst_type_in_block(b, subst),
            ast::Statement::Switch(sw) => {
                if let Some(ref mut init) = sw.init {
                    Self::subst_type_in_stmt(init, subst);
                }
                if let Some(ref mut tag) = sw.tag {
                    Self::subst_type_in_expr(tag, subst);
                }
                for case in &mut sw.block.body {
                    for e in &mut case.list {
                        Self::subst_type_in_expr(e, subst);
                    }
                    for s in case.body.as_mut().iter_mut() {
                        Self::subst_type_in_stmt(s, subst);
                    }
                }
            }
            ast::Statement::IncDec(id) => Self::subst_type_in_expr(&mut id.expr, subst),
            ast::Statement::Declaration(d) => {
                match d {
                    ast::DeclStmt::Variable(v) => {
                        for spec in &mut v.specs {
                            if let Some(ref mut typ) = spec.typ {
                                Self::subst_type_in_expr(typ, subst);
                            }
                            for val in &mut spec.values {
                                Self::subst_type_in_expr(val, subst);
                            }
                        }
                    }
                    ast::DeclStmt::Const(c) => {
                        for spec in &mut c.specs {
                            if let Some(ref mut typ) = spec.typ {
                                Self::subst_type_in_expr(typ, subst);
                            }
                            for val in &mut spec.values {
                                Self::subst_type_in_expr(val, subst);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ast::Statement::Defer(d) => {
                Self::subst_type_in_expr(&mut d.call.func, subst);
                for arg in &mut d.call.args {
                    Self::subst_type_in_expr(arg, subst);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn subst_type_in_block(block: &mut ast::BlockStmt, subst: &HashMap<String, String>) {
        for stmt in &mut block.list {
            Self::subst_type_in_stmt(stmt, subst);
        }
    }

    pub(crate) fn infer_struct_type_from_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::Ident(ident) => {
                locals.get_var_struct_type(&ident.name).map(|s| s.to_string())
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                    let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                    if let Some(type_name) = self.constant_types.get(&sel.sel.name)
                        .or_else(|| self.constant_types.get(&qualified))
                    {
                        return Some(type_name.clone());
                    }
                }
                let parent_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals)?;
                let struct_def = self.struct_defs.get(&parent_type)?;
                let field = struct_def.find_field(&sel.sel.name)?;
                let tag = field.go_type_tag.as_deref()?;
                if tag.starts_with("__") {
                    None
                } else {
                    Some(tag.to_string())
                }
            }
            ast::Expression::Index(idx) => {
                if let Some(ast::Expression::Ident(ident)) = idx.left.as_deref() {
                    if let Some(mti) = locals.map_types.get(&ident.name) {
                        if let Some(ref st) = mti.val_struct_type {
                            return Some(st.clone());
                        }
                    }
                    if let Some(st) = locals.slice_elem_struct_types.get(&ident.name) {
                        return Some(st.clone());
                    }
                }
                None
            }
            ast::Expression::Call(call) => {
                self.infer_return_struct_type(call, locals)
            }
            _ => None,
        }
    }

    pub(crate) fn is_known_named_type(&self, name: &str) -> bool {
        self.struct_defs.contains_key(name)
            || self.type_aliases.contains_key(name)
            || self.current_package.as_ref().map_or(false, |pkg| {
                let q = format!("{}.{}", pkg, name);
                self.struct_defs.contains_key(&q) || self.type_aliases.contains_key(&q)
            })
            || self.compiled_packages.iter().any(|pkg| {
                let q = format!("{}.{}", pkg, name);
                self.struct_defs.contains_key(&q) || self.type_aliases.contains_key(&q)
            })
    }

    pub(crate) fn find_method_func(&self, type_name: &str, method_name: &str) -> Option<&FuncInfo> {
        let direct = format!("{}.{}", type_name, method_name);
        if let Some(f) = self.functions.iter().find(|f| f.name == direct) {
            return Some(f);
        }
        if let Some(ref pkg) = self.current_package {
            let qualified = format!("{}.{}.{}", pkg, type_name, method_name);
            if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                return Some(f);
            }
        }
        for cpkg in &self.compiled_packages {
            let qualified = format!("{}.{}.{}", cpkg, type_name, method_name);
            if let Some(f) = self.functions.iter().find(|f| f.name == qualified) {
                return Some(f);
            }
        }
        None
    }

    pub(crate) fn infer_return_struct_type(&self, call: &ast::Call, locals: &LocalAlloc) -> Option<String> {
        match call.func.as_ref() {
            ast::Expression::Ident(ident) => {
                let fi = self.functions.iter().find(|f| f.name == ident.name && f.recv_type.is_none())?;
                fi.result_go_types.first().cloned()
                    .filter(|t| self.is_known_named_type(t))
            }
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(recv_ident) = sel.x.as_ref() {
                    let recv_type = locals.get_var_struct_type(&recv_ident.name)
                        .map(|s| s.to_string())
                        .or_else(|| {
                            if self.struct_defs.contains_key(&recv_ident.name) {
                                Some(recv_ident.name.clone())
                            } else {
                                None
                            }
                        });
                    if let Some(type_name) = recv_type {
                        let fi = self.find_method_func(&type_name, &sel.sel.name)?;
                        return fi.result_go_types.first().cloned()
                            .filter(|t| self.is_known_named_type(t));
                    }
                }
                let parent_type = self.infer_struct_type_from_expr(sel.x.as_ref(), locals)?;
                let fi = self.find_method_func(&parent_type, &sel.sel.name)?;
                fi.result_go_types.first().cloned()
                    .filter(|t| self.is_known_named_type(t))
            }
            _ => None,
        }
    }

    pub(crate) fn infer_go_type_from_expr(&self, expr: &ast::Expression, locals: &LocalAlloc) -> Option<String> {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => Some("int".to_string()),
                LitKind::Float => Some("float64".to_string()),
                LitKind::String => Some("string".to_string()),
                LitKind::Char => Some("int32".to_string()),
                _ => None,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => Some("bool".to_string()),
                "nil" => None,
                _ => {
                    if locals.string_locals.contains_key(&ident.name) {
                        return Some("string".to_string());
                    }
                    if locals.slice_elem_types.contains_key(&ident.name) {
                        return None;
                    }
                    let vt = locals.find_val_type(&ident.name);
                    match vt {
                        Some(ValType::I64) => Some("int".to_string()),
                        Some(ValType::F64) => Some("float64".to_string()),
                        Some(ValType::F32) => Some("float32".to_string()),
                        Some(ValType::I32) => Some("int32".to_string()),
                        _ => None,
                    }
                }
            },
            ast::Expression::CompositeLit(cl) => Some(Self::type_expr_to_go_string(&cl.typ)),
            ast::Expression::Operation(op) => self.infer_go_type_from_expr(&op.x, locals),
            _ => None,
        }
    }

    pub(crate) fn type_expr_to_go_string(expr: &ast::Expression) -> String {
        match expr {
            ast::Expression::Ident(id) => id.name.clone(),
            ast::Expression::TypeSlice(sl) => format!("[]{}", Self::type_expr_to_go_string(&sl.typ)),
            ast::Expression::TypeArray(arr) => format!("[?]{}", Self::type_expr_to_go_string(&arr.typ)),
            ast::Expression::TypeMap(m) => format!(
                "map[{}]{}",
                Self::type_expr_to_go_string(&m.key),
                Self::type_expr_to_go_string(&m.val),
            ),
            ast::Expression::TypePointer(p) => format!("*{}", Self::type_expr_to_go_string(&p.typ)),
            _ => String::new(),
        }
    }

    pub(crate) fn try_unify_type_param(
        param_type: &ast::Expression,
        arg_go_type: &str,
        type_param_names: &std::collections::HashSet<String>,
        subst: &mut HashMap<String, String>,
    ) {
        match param_type {
            ast::Expression::Ident(id) if type_param_names.contains(&id.name) => {
                subst.entry(id.name.clone()).or_insert_with(|| arg_go_type.to_string());
            }
            ast::Expression::TypeSlice(sl) => {
                if let Some(elem) = arg_go_type.strip_prefix("[]") {
                    Self::try_unify_type_param(&sl.typ, elem, type_param_names, subst);
                }
            }
            ast::Expression::TypeMap(m) => {
                if let Some(rest) = arg_go_type.strip_prefix("map[") {
                    if let Some(bracket_end) = rest.find(']') {
                        let key = &rest[..bracket_end];
                        let val = &rest[bracket_end + 1..];
                        Self::try_unify_type_param(&m.key, key, type_param_names, subst);
                        Self::try_unify_type_param(&m.val, val, type_param_names, subst);
                    }
                }
            }
            ast::Expression::TypePointer(p) => {
                if let Some(inner) = arg_go_type.strip_prefix('*') {
                    Self::try_unify_type_param(&p.typ, inner, type_param_names, subst);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn setup_named_composite_var(
        &self,
        var_name: &str,
        underlying: &ast::Expression,
        locals: &mut LocalAlloc,
    ) {
        match underlying {
            ast::Expression::TypeSlice(slice_type) => {
                locals.set_var_struct_type(var_name, "__slice");
                let elem_vt = Self::infer_array_elem_vt(&slice_type.typ);
                locals.slice_elem_types.insert(var_name.to_string(), elem_vt);
                if let ast::Expression::TypeSlice(inner_st) = slice_type.typ.as_ref() {
                    let inner_vt = Self::infer_array_elem_vt(&inner_st.typ);
                    locals.nested_slice_inner_elem_types.insert(var_name.to_string(), inner_vt);
                }
                if let ast::Expression::Ident(el_id) = slice_type.typ.as_ref() {
                    if self.struct_defs.contains_key(&el_id.name) {
                        locals.slice_elem_struct_types.insert(var_name.to_string(), el_id.name.clone());
                    }
                    if el_id.name == "rune" || el_id.name == "int32" {
                        locals.rune_slices.insert(var_name.to_string());
                    }
                }
            }
            ast::Expression::TypeMap(map_type) => {
                locals.set_var_struct_type(var_name, "__map");
                let key_vt = Self::infer_array_elem_vt(&map_type.key);
                let val_vt = Self::infer_array_elem_vt(&map_type.val);
                let is_string_key = matches!(map_type.key.as_ref(), ast::Expression::Ident(id) if id.name == "string");
                let is_string_val = matches!(map_type.val.as_ref(), ast::Expression::Ident(id) if id.name == "string");
                let key_size = if is_string_key { 8u32 } else { val_type_byte_size(key_vt) };
                let val_size = if is_string_val { 8u32 } else { val_type_byte_size(val_vt) };
                let val_struct_type = if let ast::Expression::Ident(vid) = map_type.val.as_ref() {
                    if self.struct_defs.contains_key(&vid.name) { Some(vid.name.clone()) } else { None }
                } else { None };
                let nested = self.build_nested_map_type_info(map_type);
                locals.map_types.insert(var_name.to_string(), MapTypeInfo {
                    key_vt,
                    val_vt,
                    key_size,
                    val_size,
                    is_string_key,
                    is_string_val,
                    val_struct_type,
                    nested_map_val_type: nested,
                });
            }
            ast::Expression::TypeArray(arr_type) => {
                let arr_len = if let ast::Expression::BasicLit(lit) = arr_type.len.as_ref() {
                    lit.value.parse::<u32>().unwrap_or(0)
                } else { 0 };
                let elem_vt = Self::infer_array_elem_vt(&arr_type.typ);
                locals.set_var_struct_type(var_name, "__array");
                locals.array_info.insert(var_name.to_string(), (elem_vt, arr_len));
            }
            _ => {}
        }
    }

    /// Monomorphize a generic type definition with concrete type arguments.
    /// Returns the monomorphized struct name (e.g., "Pair__int_string").

    pub(crate) fn monomorphize_generic_type(
        &mut self,
        type_name: &str,
        type_args: &[String],
    ) -> Result<String, Error> {
        let mono_name = format!("{}__{}", type_name, type_args.join("_"));

        if self.struct_defs.contains_key(&mono_name) {
            return Ok(mono_name);
        }

        let template = self.generic_types.get(type_name).cloned()
            .ok_or_else(|| Error::InternalError(format!("generic type '{}' not found", type_name)))?;

        let mut subst: HashMap<String, String> = HashMap::new();
        let mut idx = 0;
        for field in &template.params.list {
            for name_ident in &field.name {
                if idx < type_args.len() {
                    subst.insert(name_ident.name.clone(), type_args[idx].clone());
                }
                idx += 1;
            }
        }

        let mut specialized_type = template.typ.clone();
        Self::subst_type_in_expr(&mut specialized_type, &subst);

        if let ast::Expression::TypeStruct(struct_type) = &specialized_type {
            let struct_def = self.compute_struct_def(&struct_type.fields);
            self.struct_defs.insert(mono_name.clone(), struct_def);
            self.register_gc_struct_type_late(&mono_name);
            self.register_struct_field_map_types(&mono_name, &struct_type.fields);
        }

        Ok(mono_name)
    }

    pub(crate) fn resolve_type_name<'a>(&'a self, name: &'a str) -> &'a str {
        let mut resolved = name;
        for _ in 0..10 {
            if let Some((base, _is_alias)) = self.type_aliases.get(resolved) {
                resolved = base;
            } else if let Some(ref pkg) = self.current_package {
                let qualified = format!("{}.{}", pkg, resolved);
                if let Some((base, _is_alias)) = self.type_aliases.get(&qualified) {
                    resolved = base;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        resolved
    }

    pub(crate) fn _is_type_alias(&self, name: &str) -> bool {
        self.type_aliases
            .get(name)
            .map_or(false, |(_base, is_alias)| *is_alias)
    }

    pub(crate) fn field_to_wasm_types(&self, field: &ast::Field) -> Vec<WasmType> {
        match &field.typ {
            ast::Expression::Ident(ident) => match self.resolve_type_name(&ident.name) {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" | "uintptr" => vec![WasmType::I32],
                "int" | "int64" | "uint" | "uint64" => vec![WasmType::I64],
                "float32" => vec![WasmType::F32],
                "float64" => vec![WasmType::F64],
                "string" => {
                    if let Some(gc_idx) = self.gc_builtin_types.go_string {
                        vec![WasmType::Ref(gc_idx)]
                    } else {
                        vec![WasmType::I32, WasmType::I32]
                    }
                }
                "error" => vec![WasmType::I32],
                "Context" => vec![WasmType::I32],
                _ => vec![WasmType::I32],
            },
            ast::Expression::TypePointer(_) => vec![WasmType::I32],
            ast::Expression::TypeSlice(_) => {
                vec![WasmType::I32]
            }
            ast::Expression::TypeArray(_) => vec![WasmType::I32],
            ast::Expression::TypeMap(_) => vec![WasmType::I32],
            ast::Expression::TypeFunction(_) => vec![WasmType::I32, WasmType::I32],
            ast::Expression::TypeStruct(_) => vec![WasmType::I32],
            ast::Expression::TypeInterface(_) => vec![WasmType::I32],
            ast::Expression::Ellipsis(_) => vec![WasmType::I32], // variadic → slice header ptr
            _ => vec![WasmType::I32],
        }
    }

    pub(crate) fn infer_val_type(&self, expr: &ast::Expression, locals: &LocalAlloc) -> ValType {
        match expr {
            ast::Expression::BasicLit(lit) => match lit.kind {
                LitKind::Integer => ValType::I64,
                LitKind::Float => ValType::F64,
                LitKind::String => {
                    if let Some(gc_idx) = self.gc_builtin_types.go_string {
                        Self::gc_ref_val_type(gc_idx)
                    } else {
                        ValType::I32
                    }
                }
                LitKind::Char => ValType::I32,
                LitKind::Imag => {
                    if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                        Self::gc_ref_val_type(gc_idx)
                    } else {
                        ValType::I32
                    }
                }
                _ => ValType::I64,
            },
            ast::Expression::Ident(ident) => match ident.name.as_str() {
                "true" | "false" => ValType::I32,
                "nil" => ValType::I32,
                _ => {
                    if let Some(vt) = locals.find_type(&ident.name) {
                        return vt;
                    }
                    if let Some(ref cc) = self.closure_captures {
                        if let Some(cap) = cc.captures.iter().find(|c| c.name == ident.name) {
                            return cap.val_type;
                        }
                        if let Some((_, vt)) = cc.outer_locals.iter().find(|(n, _)| n == &ident.name) {
                            return *vt;
                        }
                    }
                    if let Some(cv) = self.constants.get(&ident.name) {
                        return match cv {
                            ConstValue::I64(_) => ValType::I64,
                            ConstValue::F64(_) => ValType::F64,
                            ConstValue::Bool(_) => ValType::I32,
                            ConstValue::Str(_) => {
                                if let Some(gc_idx) = self.gc_builtin_types.go_string {
                                    Self::gc_ref_val_type(gc_idx)
                                } else {
                                    ValType::I32
                                }
                            }
                            ConstValue::Complex128(_, _) => {
                                if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                                    Self::gc_ref_val_type(gc_idx)
                                } else {
                                    ValType::I32
                                }
                            }
                        };
                    }
                    if let Some(&(_idx, vt)) = self.resolve_global_var(&ident.name) {
                        return vt;
                    }
                    ValType::I64
                }
            },
            ast::Expression::Operation(op) => {
                if matches!(
                    op.op,
                    Operator::Equal
                        | Operator::NotEqual
                        | Operator::Less
                        | Operator::LessEqual
                        | Operator::Greater
                        | Operator::GreaterEqual
                        | Operator::AndAnd
                        | Operator::OrOr
                        | Operator::Not
                ) {
                    return ValType::I32;
                }
                if op.y.is_some() {
                    let lhs = self.infer_val_type(&op.x, locals);
                    let rhs = self.infer_val_type(op.y.as_ref().unwrap(), locals);
                    if lhs == ValType::F64 || rhs == ValType::F64 {
                        let lhs_is_int = matches!(lhs, ValType::I64 | ValType::I32);
                        let rhs_is_int = matches!(rhs, ValType::I64 | ValType::I32);
                        let lhs_int_float = if let ast::Expression::BasicLit(lit) = op.x.as_ref() {
                            if lit.kind == LitKind::Float { Self::is_integer_representable_float(&lit.value).is_some() } else { false }
                        } else { false };
                        let rhs_int_float = if let ast::Expression::BasicLit(lit) = op.y.as_ref().unwrap().as_ref() {
                            if lit.kind == LitKind::Float { Self::is_integer_representable_float(&lit.value).is_some() } else { false }
                        } else { false };
                        if (lhs == ValType::F64 && rhs_is_int && lhs_int_float)
                            || (rhs == ValType::F64 && lhs_is_int && rhs_int_float)
                        {
                            ValType::I64
                        } else {
                            ValType::F64
                        }
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
                } else {
                    if op.op == Operator::And {
                        ValType::I32
                    } else if op.op == Operator::Star {
                        self.infer_deref_type(&op.x, locals)
                    } else {
                        self.infer_val_type(&op.x, locals)
                    }
                }
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(ident) = call.func.as_ref() {
                    match ident.name.as_str() {
                        "float64" => ValType::F64,
                        "float32" => ValType::F32,
                        "int" | "int64" | "uint" | "uint64" => ValType::I64,
                        "int8" | "int16" | "int32" | "rune"
                        | "byte" | "uint8" | "uint16" | "uint32" | "bool" | "uintptr" => ValType::I32,
                        "len" | "cap" => ValType::I64,
                        "make" | "append" | "new" | "recover" => ValType::I32,
                        "complex" => {
                            let is_c64 = call.args.first().map_or(false, |a| self.infer_val_type(a, locals) == ValType::F32);
                            let gc_idx = if is_c64 { self.gc_builtin_types.complex64 } else { self.gc_builtin_types.complex128 };
                            if let Some(gc_idx) = gc_idx {
                                Self::gc_ref_val_type(gc_idx)
                            } else {
                                ValType::I32
                            }
                        }
                        "copy" => ValType::I64,
                        "string" => {
                            if let Some(gc_idx) = self.gc_builtin_types.go_string {
                                Self::gc_ref_val_type(gc_idx)
                            } else {
                                ValType::I32
                            }
                        }
                        "real" => {
                            if let Some(arg) = call.args.first() {
                                if self.is_complex64_expr(arg, locals) { ValType::F32 } else { ValType::F64 }
                            } else {
                                ValType::F64
                            }
                        }
                        "imag" => {
                            if let Some(arg) = call.args.first() {
                                if self.is_complex64_expr(arg, locals) { ValType::F32 } else { ValType::F64 }
                            } else {
                                ValType::F64
                            }
                        }
                        "min" | "max" => {
                            if let Some(first_arg) = call.args.first() {
                                self.infer_val_type(first_arg, locals)
                            } else {
                                ValType::I64
                            }
                        }
                        "Float64frombits" => ValType::F64,
                        "Float64bits" => ValType::I64,
                        "Float32frombits" => ValType::F32,
                        "Float32bits" => ValType::I32,
                        _ => {
                            if self.type_aliases.contains_key(&ident.name)
                                || self.current_package.as_ref().map_or(false, |pkg| {
                                    self.type_aliases.contains_key(&format!("{}.{}", pkg, ident.name))
                                })
                            {
                                let alias_key = if self.type_aliases.contains_key(&ident.name) {
                                    ident.name.clone()
                                } else {
                                    format!("{}.{}", self.current_package.as_ref().unwrap(), ident.name)
                                };
                                let resolved = self.resolve_type_name(&alias_key);
                                Self::val_type_for_type_name(resolved)
                            } else if let Some(fi) = self.find_func_in_pkg(&ident.name) {
                                fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type())
                            } else {
                                ValType::I64
                            }
                        }
                    }
                } else if let ast::Expression::TypeSlice(_) = call.func.as_ref() {
                    ValType::I32
                } else if let ast::Expression::TypeArray(_) = call.func.as_ref() {
                    ValType::I32
                } else if let ast::Expression::Selector(sel) = call.func.as_ref() {
                    if let ast::Expression::Ident(receiver) = sel.x.as_ref() {
                        // Check compiled stdlib package functions first
                        if self.compiled_packages.contains(receiver.name.as_str()) {
                            let qualified = format!("{}.{}", receiver.name, sel.sel.name);
                            if let Some(fi) = self.functions.iter().find(|f| f.name == qualified && f.recv_type.is_none()) {
                                return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                            }
                        }
                        match (receiver.name.as_str(), sel.sel.name.as_str()) {
                            ("math", _) => ValType::F64,
                            _ => {
                                let method_name = &sel.sel.name;
                                // Check if receiver is an interface variable
                                if self.is_interface_var(&receiver.name, locals) {
                                    if let Some(fi) = self.functions.iter().find(|f| {
                                        f.recv_type.is_some()
                                            && f.name.ends_with(&format!(".{}", method_name))
                                    }) {
                                        return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                                    }
                                }
                                // Try qualified method name (Type.Method)
                                if let Some(type_name) = locals.get_var_struct_type(&receiver.name) {
                                    let qname = format!("{}.{}", type_name, method_name);
                                    if let Some(fi) = self.functions.iter().find(|f| f.name == qname) {
                                        return fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type());
                                    }
                                }
                                // Try bare method name
                                if let Some(fi) = self.functions.iter().find(|f| f.name == *method_name) {
                                    fi.results.first().map_or(ValType::I64, |wt| wt.to_val_type())
                                } else {
                                    ValType::I64
                                }
                            }
                        }
                    } else {
                        ValType::I64
                    }
                } else {
                    ValType::I64
                }
            }
            ast::Expression::Paren(p) => self.infer_val_type(&p.expr, locals),
            ast::Expression::Selector(sel) => {
                if let Some(type_name) = self.infer_struct_type_from_expr(sel.x.as_ref(), locals) {
                    if let Some(sd) = self.struct_defs.get(&type_name) {
                        if let Some(field) = sd.find_field(&sel.sel.name) {
                            return field.wasm_type.to_val_type();
                        }
                    }
                }
                if let ast::Expression::Ident(pkg_ident) = sel.x.as_ref() {
                    if self.compiled_packages.contains(pkg_ident.name.as_str()) {
                        if let Some(cv) = self.constants.get(&sel.sel.name) {
                            return match cv {
                                ConstValue::I64(_) => ValType::I64,
                                ConstValue::F64(_) => ValType::F64,
                                ConstValue::Bool(_) => ValType::I32,
                                ConstValue::Str(_) => {
                                    if let Some(gc_idx) = self.gc_builtin_types.go_string {
                                        Self::gc_ref_val_type(gc_idx)
                                    } else {
                                        ValType::I32
                                    }
                                }
                                ConstValue::Complex128(_, _) => {
                                    if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                                        Self::gc_ref_val_type(gc_idx)
                                    } else {
                                        ValType::I32
                                    }
                                }
                            };
                        }
                        let qualified = format!("{}.{}", pkg_ident.name, sel.sel.name);
                        if let Some(&(_, vt)) = self.global_vars.get(&qualified) {
                            return vt;
                        }
                    }
                }
                ValType::I32
            }
            ast::Expression::CompositeLit(cl) => {
                if let ast::Expression::Ident(id) = cl.typ.as_ref() {
                    let resolved = self.resolve_struct_in_pkg(&id.name);
                    if let Some(sd) = self.struct_defs.get(&resolved) {
                        if let Some(gc_idx) = sd.gc_type_idx {
                            return Self::gc_ref_val_type(gc_idx);
                        }
                    }
                } else if let ast::Expression::Index(idx) = cl.typ.as_ref() {
                    if let Some(ast::Expression::Ident(type_ident)) = idx.left.as_ref().map(|l| l.as_ref()) {
                        if let Some(sd) = self.struct_defs.get(&type_ident.name) {
                            if let Some(gc_idx) = sd.gc_type_idx {
                                return Self::gc_ref_val_type(gc_idx);
                            }
                        }
                    }
                }
                ValType::I32
            }
            ast::Expression::Index(idx) => {
                if let Some(left) = idx.left.as_deref() {
                    if self.is_string_expr(left, locals) {
                        return ValType::I32;
                    }
                }
                if let Some(ast::Expression::Ident(ident)) = idx.left.as_deref() {
                    if let Some(mti) = locals.map_types.get(&ident.name) {
                        return mti.val_vt;
                    }
                    if let Some(&vt) = locals.slice_elem_types.get(&ident.name) {
                        return vt;
                    }
                    if let Some(&(arr_elem_vt, ..)) = locals.array_info.get(&ident.name) {
                        return arr_elem_vt;
                    }
                    let resolved = self.resolve_global_var_name(&ident.name);
                    if let Some(&(vt, _, _)) = self.global_array_elem_types.get(&resolved) {
                        return vt;
                    }
                    ValType::I64
                } else if let Some(ast::Expression::Index(outer_idx)) = idx.left.as_deref() {
                    if let Some(ast::Expression::Ident(outer_ident)) = outer_idx.left.as_deref() {
                        if let Some(&inner_vt) = locals.nested_slice_inner_elem_types.get(&outer_ident.name) {
                            return inner_vt;
                        }
                    }
                    ValType::I64
                } else {
                    ValType::I64
                }
            }
            ast::Expression::Slice(slice) => {
                if self.is_string_expr(&slice.left, locals) {
                    ValType::I32
                } else {
                    ValType::I32
                }
            }
            ast::Expression::FuncLit(_) => ValType::I32,
            ast::Expression::TypeAssert(ta) => {
                if let Some(ref target) = ta.right {
                    if let ast::Expression::Ident(type_ident) = target.as_ref() {
                        return Self::val_type_for_type_name(&type_ident.name);
                    }
                }
                ValType::I64
            }
            _ => ValType::I64,
        }
    }

    pub(crate) fn expr_to_val_type(&self, expr: &ast::Expression) -> ValType {
        match expr {
            ast::Expression::Ident(ident) => match self.resolve_type_name(&ident.name) {
                "bool" | "byte" | "uint8" | "int8" | "int16" | "uint16" | "int32"
                | "uint32" | "rune" | "uintptr" => ValType::I32,
                "int" | "int64" | "uint" | "uint64" => ValType::I64,
                "float32" => ValType::F32,
                "float64" => ValType::F64,
                "complex64" => {
                    if let Some(gc_idx) = self.gc_builtin_types.complex64 {
                        Self::gc_ref_val_type(gc_idx)
                    } else {
                        ValType::I32
                    }
                }
                "complex128" => {
                    if let Some(gc_idx) = self.gc_builtin_types.complex128 {
                        Self::gc_ref_val_type(gc_idx)
                    } else {
                        ValType::I32
                    }
                }
                "string" => {
                    if let Some(gc_idx) = self.gc_builtin_types.go_string {
                        Self::gc_ref_val_type(gc_idx)
                    } else {
                        ValType::I32
                    }
                }
                "error" | "any" => ValType::I32,
                name if self.iface_defs.contains_key(name) => ValType::I32,
                name => {
                    let resolved_name = self.resolve_struct_in_pkg(name);
                    if let Some(sd) = self.struct_defs.get(&resolved_name) {
                        if let Some(gc_idx) = sd.gc_type_idx {
                            return Self::gc_ref_val_type(gc_idx);
                        }
                    }
                    ValType::I32
                }
            },
            ast::Expression::TypePointer(_) => ValType::I32,
            ast::Expression::TypeSlice(_) => ValType::I32,
            ast::Expression::TypeInterface(_) => ValType::I32,
            ast::Expression::TypeMap(_) => ValType::I32,
            ast::Expression::TypeFunction(_) => ValType::I32,
            ast::Expression::TypeArray(_) => ValType::I32,
            ast::Expression::TypeStruct(_) => ValType::I32,
            ast::Expression::Selector(sel) => {
                if let ast::Expression::Ident(pkg) = sel.x.as_ref() {
                    let qualified = format!("{}.{}", pkg.name, sel.sel.name);
                    if let Some(sd) = self.struct_defs.get(&qualified) {
                        if let Some(gc_idx) = sd.gc_type_idx {
                            return Self::gc_ref_val_type(gc_idx);
                        }
                        return ValType::I32;
                    }
                    if self.iface_defs.contains_key(&qualified) || sel.sel.name == "error" {
                        return ValType::I32;
                    }
                    if let Some((alias, _)) = self.type_aliases.get(&qualified) {
                        return self.expr_to_val_type(&ast::Expression::Ident(ast::Ident {
                            name: alias.clone(),
                            ..Default::default()
                        }));
                    }
                }
                ValType::I64
            },
            _ => ValType::I64,
        }
    }
}
