use super::*;

impl WasmCompiler {
    pub(crate) fn extract_recv_type_name(&self, recv: &ast::FieldList) -> Option<String> {
        recv.list.first().and_then(|field| match &field.typ {
            ast::Expression::TypePointer(p) => {
                if let ast::Expression::Ident(id) = p.typ.as_ref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            ast::Expression::Ident(id) => Some(id.name.clone()),
            ast::Expression::Index(idx) => {
                if let Some(ast::Expression::Ident(id)) = idx.left.as_deref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            ast::Expression::IndexList(idx_list) => {
                if let ast::Expression::Ident(id) = idx_list.left.as_ref() {
                    Some(id.name.clone())
                } else {
                    None
                }
            }
            _ => None,
        })
    }

    pub(crate) fn is_generic_recv(&self, recv: &ast::FieldList) -> bool {
        recv.list.first().map_or(false, |field| {
            matches!(&field.typ,
                ast::Expression::Index(_) | ast::Expression::IndexList(_))
        })
    }

    pub(crate) fn analyze_function_escapes(body: &ast::BlockStmt) -> HashSet<String> {
        let mut escaping = HashSet::new();
        let mut assignments: Vec<(String, String)> = Vec::new();

        for stmt in &body.list {
            Self::escape_scan_stmt(stmt, &mut escaping, &mut assignments);
        }

        let mut changed = true;
        while changed {
            changed = false;
            for (dst, src) in &assignments {
                if escaping.contains(dst.as_str()) && !escaping.contains(src.as_str()) {
                    escaping.insert(src.clone());
                    changed = true;
                }
            }
        }

        escaping
    }

    pub(crate) fn escape_scan_stmt(
        stmt: &ast::Statement,
        escaping: &mut HashSet<String>,
        assignments: &mut Vec<(String, String)>,
    ) {
        match stmt {
            ast::Statement::Return(ret) => {
                for expr in &ret.ret {
                    Self::mark_expr_escaping(expr, escaping);
                    Self::escape_scan_call_args(expr, escaping);
                }
            }
            ast::Statement::Assign(assign) => {
                for expr in &assign.right {
                    Self::escape_scan_call_args(expr, escaping);
                }

                for (i, lhs) in assign.left.iter().enumerate() {
                    match lhs {
                        ast::Expression::Selector(_)
                        | ast::Expression::Index(_)
                        | ast::Expression::Star(_) => {
                            if let Some(rhs) = assign.right.get(i) {
                                Self::mark_expr_escaping(rhs, escaping);
                            }
                        }
                        ast::Expression::Ident(dst_id) => {
                            if let Some(rhs) = assign.right.get(i) {
                                if let ast::Expression::Ident(src_id) = rhs {
                                    assignments.push((dst_id.name.clone(), src_id.name.clone()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            ast::Statement::Expr(es) => {
                Self::escape_scan_call_args(&es.expr, escaping);
            }
            ast::Statement::If(if_stmt) => {
                if let Some(init) = &if_stmt.init {
                    Self::escape_scan_stmt(init, escaping, assignments);
                }
                Self::escape_scan_call_args(&if_stmt.cond, escaping);
                for s in &if_stmt.body.list {
                    Self::escape_scan_stmt(s, escaping, assignments);
                }
                if let Some(else_) = &if_stmt.else_ {
                    Self::escape_scan_stmt(else_, escaping, assignments);
                }
            }
            ast::Statement::For(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    Self::escape_scan_stmt(init, escaping, assignments);
                }
                if let Some(cond) = &for_stmt.cond {
                    Self::escape_scan_stmt(cond, escaping, assignments);
                }
                if let Some(post) = &for_stmt.post {
                    Self::escape_scan_stmt(post, escaping, assignments);
                }
                for s in &for_stmt.body.list {
                    Self::escape_scan_stmt(s, escaping, assignments);
                }
            }
            ast::Statement::Range(range_stmt) => {
                Self::escape_scan_call_args(&range_stmt.expr, escaping);
                for s in &range_stmt.body.list {
                    Self::escape_scan_stmt(s, escaping, assignments);
                }
            }
            ast::Statement::Block(block) => {
                for s in &block.list {
                    Self::escape_scan_stmt(s, escaping, assignments);
                }
            }
            ast::Statement::Switch(sw) => {
                if let Some(init) = &sw.init {
                    Self::escape_scan_stmt(init, escaping, assignments);
                }
                if let Some(tag) = &sw.tag {
                    Self::escape_scan_call_args(tag, escaping);
                }
                for clause in &sw.block.body {
                    for e in &clause.list {
                        Self::escape_scan_call_args(e, escaping);
                    }
                    for s in clause.body.iter() {
                        Self::escape_scan_stmt(s, escaping, assignments);
                    }
                }
            }
            ast::Statement::TypeSwitch(tsw) => {
                if let Some(init) = &tsw.init {
                    Self::escape_scan_stmt(init, escaping, assignments);
                }
                if let Some(tag) = &tsw.tag {
                    Self::escape_scan_stmt(tag, escaping, assignments);
                }
                for clause in &tsw.block.body {
                    for s in clause.body.iter() {
                        Self::escape_scan_stmt(s, escaping, assignments);
                    }
                }
            }
            ast::Statement::Send(send) => {
                Self::mark_expr_escaping(&send.value, escaping);
                Self::escape_scan_call_args(&send.value, escaping);
                Self::escape_scan_call_args(&send.chan, escaping);
            }
            ast::Statement::Go(go_stmt) => {
                for arg in &go_stmt.call.args {
                    Self::mark_expr_escaping(arg, escaping);
                    Self::escape_scan_call_args(arg, escaping);
                }
                Self::escape_scan_call_args(&go_stmt.call.func, escaping);
            }
            ast::Statement::Defer(defer_stmt) => {
                for arg in &defer_stmt.call.args {
                    Self::mark_expr_escaping(arg, escaping);
                    Self::escape_scan_call_args(arg, escaping);
                }
                Self::escape_scan_call_args(&defer_stmt.call.func, escaping);
            }
            ast::Statement::Declaration(decl_stmt) => {
                if let ast::DeclStmt::Variable(var_decl) = decl_stmt {
                    for spec in &var_decl.specs {
                        for val in &spec.values {
                            Self::escape_scan_call_args(val, escaping);
                        }
                        for (i, name) in spec.name.iter().enumerate() {
                            if let Some(val) = spec.values.get(i) {
                                if let ast::Expression::Ident(src_id) = val {
                                    assignments.push((name.name.clone(), src_id.name.clone()));
                                }
                            }
                        }
                    }
                }
            }
            ast::Statement::Label(labeled) => {
                Self::escape_scan_stmt(&labeled.stmt, escaping, assignments);
            }
            ast::Statement::Select(sel) => {
                for clause in &sel.body.body {
                    if let Some(comm) = &clause.comm {
                        Self::escape_scan_stmt(comm, escaping, assignments);
                    }
                    for s in clause.body.iter() {
                        Self::escape_scan_stmt(s, escaping, assignments);
                    }
                }
            }
            ast::Statement::IncDec(incdec) => {
                Self::escape_scan_call_args(&incdec.expr, escaping);
            }
            _ => {}
        }
    }

    pub(crate) fn mark_expr_escaping(expr: &ast::Expression, escaping: &mut HashSet<String>) {
        match expr {
            ast::Expression::Ident(id) => {
                escaping.insert(id.name.clone());
            }
            ast::Expression::Paren(p) => Self::mark_expr_escaping(&p.expr, escaping),
            ast::Expression::Star(s) => Self::mark_expr_escaping(&s.right, escaping),
            ast::Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
                Self::mark_expr_escaping(&op.x, escaping);
            }
            ast::Expression::Selector(sel) => Self::mark_expr_escaping(&sel.x, escaping),
            ast::Expression::Index(idx) => {
                if let Some(ref left) = idx.left {
                    Self::mark_expr_escaping(left, escaping);
                }
            }
            ast::Expression::Slice(sl) => Self::mark_expr_escaping(&sl.left, escaping),
            ast::Expression::FuncLit(fl) => {
                let mut bound = HashSet::new();
                for field in &fl.typ.params.list {
                    for name in &field.name {
                        bound.insert(name.name.clone());
                    }
                }
                for field in &fl.typ.result.list {
                    for name in &field.name {
                        bound.insert(name.name.clone());
                    }
                }
                let mut captured = HashSet::new();
                Self::collect_free_vars_block(&fl.body, &mut bound, &mut captured);
                for name in &captured {
                    escaping.insert(name.clone());
                }
            }
            ast::Expression::CompositeLit(comp) => {
                for kv in &comp.val.values {
                    if let Some(key) = &kv.key {
                        Self::mark_escaping_element(key, escaping);
                    }
                    Self::mark_escaping_element(&kv.val, escaping);
                }
            }
            ast::Expression::Invar(inv) => Self::mark_expr_escaping(&inv.expr, escaping),
            _ => {}
        }
    }

    pub(crate) fn mark_escaping_element(elem: &ast::Element, escaping: &mut HashSet<String>) {
        match elem {
            ast::Element::Expr(e) => Self::mark_expr_escaping(e, escaping),
            ast::Element::LitValue(lv) => {
                for kv in &lv.values {
                    if let Some(key) = &kv.key {
                        Self::mark_escaping_element(key, escaping);
                    }
                    Self::mark_escaping_element(&kv.val, escaping);
                }
            }
        }
    }

    pub(crate) fn escape_scan_element(elem: &ast::Element, escaping: &mut HashSet<String>) {
        match elem {
            ast::Element::Expr(e) => {
                Self::mark_expr_escaping(e, escaping);
                Self::escape_scan_call_args(e, escaping);
            }
            ast::Element::LitValue(lv) => {
                for kv in &lv.values {
                    if let Some(key) = &kv.key {
                        Self::escape_scan_element(key, escaping);
                    }
                    Self::escape_scan_element(&kv.val, escaping);
                }
            }
        }
    }

    pub(crate) fn escape_scan_call_args(expr: &ast::Expression, escaping: &mut HashSet<String>) {
        match expr {
            ast::Expression::Call(call) => {
                for arg in &call.args {
                    Self::mark_expr_escaping(arg, escaping);
                    Self::escape_scan_call_args(arg, escaping);
                }
                Self::escape_scan_call_args(&call.func, escaping);
            }
            ast::Expression::Operation(op) => {
                Self::escape_scan_call_args(&op.x, escaping);
                if let Some(ref y) = op.y {
                    Self::escape_scan_call_args(y, escaping);
                }
                if op.y.is_none() {
                    if matches!(op.op, Operator::And) {
                        Self::mark_expr_escaping(&op.x, escaping);
                    }
                }
            }
            ast::Expression::Paren(p) => Self::escape_scan_call_args(&p.expr, escaping),
            ast::Expression::CompositeLit(comp) => {
                for kv in &comp.val.values {
                    if let Some(key) = &kv.key {
                        Self::escape_scan_element(key, escaping);
                    }
                    Self::escape_scan_element(&kv.val, escaping);
                }
            }
            ast::Expression::FuncLit(fl) => {
                let mut bound = HashSet::new();
                for field in &fl.typ.params.list {
                    for name in &field.name {
                        bound.insert(name.name.clone());
                    }
                }
                for field in &fl.typ.result.list {
                    for name in &field.name {
                        bound.insert(name.name.clone());
                    }
                }
                let mut captured = HashSet::new();
                Self::collect_free_vars_block(&fl.body, &mut bound, &mut captured);
                for name in &captured {
                    escaping.insert(name.clone());
                }
            }
            ast::Expression::Selector(sel) => Self::escape_scan_call_args(&sel.x, escaping),
            ast::Expression::Index(idx) => {
                if let Some(ref left) = idx.left {
                    Self::escape_scan_call_args(left, escaping);
                }
                Self::escape_scan_call_args(&idx.index, escaping);
            }
            ast::Expression::Slice(sl) => {
                Self::escape_scan_call_args(&sl.left, escaping);
                for opt_idx in &sl.index {
                    if let Some(e) = opt_idx {
                        Self::escape_scan_call_args(e, escaping);
                    }
                }
            }
            ast::Expression::IndexList(il) => {
                Self::escape_scan_call_args(&il.left, escaping);
                for idx in &il.indices {
                    Self::escape_scan_call_args(idx, escaping);
                }
            }
            ast::Expression::TypeAssert(ta) => {
                Self::escape_scan_call_args(&ta.left, escaping);
            }
            ast::Expression::Star(s) => {
                Self::escape_scan_call_args(&s.right, escaping);
            }
            ast::Expression::Invar(inv) => {
                Self::escape_scan_call_args(&inv.expr, escaping);
            }
            _ => {}
        }
    }

    pub(crate) fn collect_free_vars_block(
        block: &ast::BlockStmt,
        bound: &mut HashSet<String>,
        free: &mut HashSet<String>,
    ) {
        for stmt in &block.list {
            Self::collect_free_vars_stmt(stmt, bound, free);
        }
    }

    pub(crate) fn collect_free_vars_stmt(
        stmt: &ast::Statement,
        bound: &mut HashSet<String>,
        free: &mut HashSet<String>,
    ) {
        match stmt {
            ast::Statement::Assign(assign) => {
                for expr in &assign.right {
                    Self::collect_free_vars_expr(expr, bound, free);
                }
                if matches!(assign.op, Operator::Define) {
                    for lhs in &assign.left {
                        if let ast::Expression::Ident(id) = lhs {
                            bound.insert(id.name.clone());
                        }
                    }
                } else {
                    for lhs in &assign.left {
                        Self::collect_free_vars_expr(lhs, bound, free);
                    }
                }
            }
            ast::Statement::Expr(es) => {
                Self::collect_free_vars_expr(&es.expr, bound, free);
            }
            ast::Statement::Return(ret) => {
                for e in &ret.ret {
                    Self::collect_free_vars_expr(e, bound, free);
                }
            }
            ast::Statement::If(if_stmt) => {
                let mut scoped = bound.clone();
                if let Some(init) = &if_stmt.init {
                    Self::collect_free_vars_stmt(init, &mut scoped, free);
                }
                Self::collect_free_vars_expr(&if_stmt.cond, &scoped, free);
                Self::collect_free_vars_block(&if_stmt.body, &mut scoped.clone(), free);
                if let Some(else_) = &if_stmt.else_ {
                    Self::collect_free_vars_stmt(else_, &mut scoped.clone(), free);
                }
            }
            ast::Statement::For(for_stmt) => {
                let mut scoped = bound.clone();
                if let Some(init) = &for_stmt.init {
                    Self::collect_free_vars_stmt(init, &mut scoped, free);
                }
                if let Some(cond) = &for_stmt.cond {
                    Self::collect_free_vars_stmt(cond, &mut scoped, free);
                }
                if let Some(post) = &for_stmt.post {
                    Self::collect_free_vars_stmt(post, &mut scoped, free);
                }
                Self::collect_free_vars_block(&for_stmt.body, &mut scoped, free);
            }
            ast::Statement::Range(range_stmt) => {
                Self::collect_free_vars_expr(&range_stmt.expr, bound, free);
                let mut scoped = bound.clone();
                let is_define = range_stmt.op.as_ref()
                    .map_or(false, |(_, op)| matches!(op, Operator::Define));
                if is_define {
                    if let Some(ast::Expression::Ident(k)) = &range_stmt.key {
                        scoped.insert(k.name.clone());
                    }
                    if let Some(ast::Expression::Ident(v)) = &range_stmt.value {
                        scoped.insert(v.name.clone());
                    }
                } else {
                    if let Some(key) = &range_stmt.key {
                        Self::collect_free_vars_expr(key, bound, free);
                    }
                    if let Some(value) = &range_stmt.value {
                        Self::collect_free_vars_expr(value, bound, free);
                    }
                }
                Self::collect_free_vars_block(&range_stmt.body, &mut scoped, free);
            }
            ast::Statement::Block(block) => {
                Self::collect_free_vars_block(block, &mut bound.clone(), free);
            }
            ast::Statement::Declaration(decl_stmt) => {
                if let ast::DeclStmt::Variable(var_decl) = decl_stmt {
                    for spec in &var_decl.specs {
                        for val in &spec.values {
                            Self::collect_free_vars_expr(val, bound, free);
                        }
                        for name in &spec.name {
                            bound.insert(name.name.clone());
                        }
                    }
                }
            }
            ast::Statement::Switch(sw) => {
                let mut scoped = bound.clone();
                if let Some(init) = &sw.init {
                    Self::collect_free_vars_stmt(init, &mut scoped, free);
                }
                if let Some(tag) = &sw.tag {
                    Self::collect_free_vars_expr(tag, &scoped, free);
                }
                for clause in &sw.block.body {
                    let mut clause_scope = scoped.clone();
                    for e in &clause.list {
                        Self::collect_free_vars_expr(e, &clause_scope, free);
                    }
                    for s in clause.body.iter() {
                        Self::collect_free_vars_stmt(s, &mut clause_scope, free);
                    }
                }
            }
            ast::Statement::TypeSwitch(tsw) => {
                let mut scoped = bound.clone();
                if let Some(init) = &tsw.init {
                    Self::collect_free_vars_stmt(init, &mut scoped, free);
                }
                if let Some(tag) = &tsw.tag {
                    Self::collect_free_vars_stmt(tag, &mut scoped, free);
                }
                for clause in &tsw.block.body {
                    let mut clause_scope = scoped.clone();
                    for s in clause.body.iter() {
                        Self::collect_free_vars_stmt(s, &mut clause_scope, free);
                    }
                }
            }
            ast::Statement::Label(labeled) => {
                Self::collect_free_vars_stmt(&labeled.stmt, bound, free);
            }
            ast::Statement::Send(send) => {
                Self::collect_free_vars_expr(&send.chan, bound, free);
                Self::collect_free_vars_expr(&send.value, bound, free);
            }
            ast::Statement::Defer(defer) => {
                Self::collect_free_vars_expr(&defer.call.func, bound, free);
                for arg in &defer.call.args {
                    Self::collect_free_vars_expr(arg, bound, free);
                }
            }
            ast::Statement::Go(go_stmt) => {
                Self::collect_free_vars_expr(&go_stmt.call.func, bound, free);
                for arg in &go_stmt.call.args {
                    Self::collect_free_vars_expr(arg, bound, free);
                }
            }
            ast::Statement::IncDec(incdec) => {
                Self::collect_free_vars_expr(&incdec.expr, bound, free);
            }
            ast::Statement::Select(sel) => {
                for clause in &sel.body.body {
                    let mut clause_scope = bound.clone();
                    if let Some(comm) = &clause.comm {
                        Self::collect_free_vars_stmt(comm, &mut clause_scope, free);
                    }
                    for s in clause.body.iter() {
                        Self::collect_free_vars_stmt(s, &mut clause_scope, free);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn collect_free_vars_expr(
        expr: &ast::Expression,
        bound: &HashSet<String>,
        free: &mut HashSet<String>,
    ) {
        match expr {
            ast::Expression::Ident(id) => {
                if !bound.contains(&id.name)
                    && !matches!(id.name.as_str(), "_" | "true" | "false" | "nil" | "iota"
                        | "len" | "cap" | "append" | "copy" | "delete" | "make" | "new"
                        | "close" | "complex" | "real" | "imag" | "uintptr"
                        | "panic" | "recover" | "print" | "println" | "string" | "int"
                        | "int8" | "int16" | "int32" | "int64" | "uint" | "uint8" | "uint16"
                        | "uint32" | "uint64" | "float32" | "float64" | "byte" | "rune"
                        | "bool" | "complex64" | "complex128" | "error" | "any")
                {
                    free.insert(id.name.clone());
                }
            }
            ast::Expression::Call(call) => {
                Self::collect_free_vars_expr(&call.func, bound, free);
                for arg in &call.args {
                    Self::collect_free_vars_expr(arg, bound, free);
                }
            }
            ast::Expression::Operation(op) => {
                Self::collect_free_vars_expr(&op.x, bound, free);
                if let Some(ref y) = op.y {
                    Self::collect_free_vars_expr(y, bound, free);
                }
            }
            ast::Expression::Selector(sel) => {
                Self::collect_free_vars_expr(&sel.x, bound, free);
            }
            ast::Expression::Index(idx) => {
                if let Some(ref l) = idx.left {
                    Self::collect_free_vars_expr(l, bound, free);
                }
                Self::collect_free_vars_expr(&idx.index, bound, free);
            }
            ast::Expression::Paren(p) => Self::collect_free_vars_expr(&p.expr, bound, free),
            ast::Expression::Star(s) => Self::collect_free_vars_expr(&s.right, bound, free),
            ast::Expression::Slice(sl) => {
                Self::collect_free_vars_expr(&sl.left, bound, free);
                for opt_idx in &sl.index {
                    if let Some(e) = opt_idx {
                        Self::collect_free_vars_expr(e, bound, free);
                    }
                }
            }
            ast::Expression::FuncLit(fl) => {
                let mut inner_bound = bound.clone();
                for field in &fl.typ.params.list {
                    for name in &field.name {
                        inner_bound.insert(name.name.clone());
                    }
                }
                for field in &fl.typ.result.list {
                    for name in &field.name {
                        inner_bound.insert(name.name.clone());
                    }
                }
                Self::collect_free_vars_block(&fl.body, &mut inner_bound, free);
            }
            ast::Expression::IndexList(il) => {
                Self::collect_free_vars_expr(&il.left, bound, free);
                for idx in &il.indices {
                    Self::collect_free_vars_expr(idx, bound, free);
                }
            }
            ast::Expression::TypeAssert(ta) => {
                Self::collect_free_vars_expr(&ta.left, bound, free);
            }
            ast::Expression::Ellipsis(el) => {
                if let Some(ref elt) = el.elt {
                    Self::collect_free_vars_expr(elt, bound, free);
                }
            }
            ast::Expression::CompositeLit(comp) => {
                for kv in &comp.val.values {
                    if let Some(key) = &kv.key {
                        Self::collect_free_vars_element(key, bound, free);
                    }
                    Self::collect_free_vars_element(&kv.val, bound, free);
                }
            }
            ast::Expression::List(exprs) => {
                for e in exprs {
                    Self::collect_free_vars_expr(e, bound, free);
                }
            }
            ast::Expression::Invar(inv) => {
                Self::collect_free_vars_expr(&inv.expr, bound, free);
            }
            _ => {}
        }
    }

    pub(crate) fn collect_free_vars_element(
        elem: &ast::Element,
        bound: &HashSet<String>,
        free: &mut HashSet<String>,
    ) {
        match elem {
            ast::Element::Expr(e) => Self::collect_free_vars_expr(e, bound, free),
            ast::Element::LitValue(lv) => {
                for kv in &lv.values {
                    if let Some(key) = &kv.key {
                        Self::collect_free_vars_element(key, bound, free);
                    }
                    Self::collect_free_vars_element(&kv.val, bound, free);
                }
            }
        }
    }

    pub(crate) fn compute_stack_frame(
        &self,
        decl: &ast::FuncDecl,
        escaping: &HashSet<String>,
    ) -> StackFrameInfo {
        let mut locals: Vec<StackLocal> = Vec::new();
        let mut offset: u32 = 0;

        let addr_taken = if let Some(body) = &decl.body {
            Self::collect_stack_locals_from_block(
                &body.list, escaping, &self.struct_defs, &mut locals, &mut offset,
            );

            let addr_taken = Self::analyze_address_taken_vars(body);

            Self::collect_escaped_scalars_from_block(
                &body.list, &addr_taken, &self.struct_defs, &mut locals, &mut offset,
            );
            addr_taken
        } else {
            HashSet::new()
        };

        for field in &decl.typ.params.list {
            for name in &field.name {
                if addr_taken.contains(&name.name) {
                    if locals.iter().any(|l| l.name == name.name) {
                        continue;
                    }
                    let is_struct_param = if let ast::Expression::Ident(ti) = &field.typ {
                        self.struct_defs.contains_key(&ti.name)
                    } else { false };
                    if is_struct_param {
                        continue;
                    }
                    let aligned = (offset + 7) & !7;
                    locals.push(StackLocal {
                        name: name.name.clone(),
                        offset: aligned,
                        size: 8,
                    });
                    offset = aligned + 8;
                }
            }
        }

        let total_size = (offset + 7) & !7;
        StackFrameInfo {
            locals,
            total_size,
            frame_base_local: None,
        }
    }

    pub(crate) fn collect_stack_locals_from_block(
        stmts: &[ast::Statement],
        escaping: &HashSet<String>,
        struct_defs: &HashMap<String, StructDef>,
        locals: &mut Vec<StackLocal>,
        offset: &mut u32,
    ) {
        for stmt in stmts {
            Self::collect_stack_locals_from_stmt(stmt, escaping, struct_defs, locals, offset);
        }
    }

    pub(crate) fn collect_stack_locals_from_stmt(
        stmt: &ast::Statement,
        escaping: &HashSet<String>,
        struct_defs: &HashMap<String, StructDef>,
        locals: &mut Vec<StackLocal>,
        offset: &mut u32,
    ) {
        match stmt {
            ast::Statement::Assign(assign) if matches!(assign.op, Operator::Define) => {
                for (i, lhs) in assign.left.iter().enumerate() {
                    if let ast::Expression::Ident(id) = lhs {
                        if escaping.contains(&id.name) {
                            continue;
                        }
                        if locals.iter().any(|l| l.name == id.name) {
                            continue;
                        }
                        let size = if let Some(rhs) = assign.right.get(i) {
                            Self::infer_compound_alloc_size(rhs, struct_defs)
                        } else {
                            None
                        };
                        if let Some(sz) = size {
                            let aligned = (*offset + 7) & !7;
                            locals.push(StackLocal {
                                name: id.name.clone(),
                                offset: aligned,
                                size: sz,
                            });
                            *offset = aligned + sz;
                        }
                    }
                }
            }
            ast::Statement::Declaration(decl_stmt) => {
                if let ast::DeclStmt::Variable(var_decl) = decl_stmt {
                    for spec in &var_decl.specs {
                        for (i, name) in spec.name.iter().enumerate() {
                            if escaping.contains(&name.name) {
                                continue;
                            }
                            if locals.iter().any(|l| l.name == name.name) {
                                continue;
                            }
                            let size = if let Some(val) = spec.values.get(i) {
                                Self::infer_compound_alloc_size(val, struct_defs)
                            } else if let Some(typ) = &spec.typ {
                                Self::infer_type_expr_size(typ, struct_defs)
                            } else {
                                None
                            };
                            if let Some(sz) = size {
                                let aligned = (*offset + 7) & !7;
                                locals.push(StackLocal {
                                    name: name.name.clone(),
                                    offset: aligned,
                                    size: sz,
                                });
                                *offset = aligned + sz;
                            }
                        }
                    }
                }
            }
            ast::Statement::If(if_stmt) => {
                if let Some(init) = &if_stmt.init {
                    Self::collect_stack_locals_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                Self::collect_stack_locals_from_block(&if_stmt.body.list, escaping, struct_defs, locals, offset);
                if let Some(else_) = &if_stmt.else_ {
                    Self::collect_stack_locals_from_stmt(else_, escaping, struct_defs, locals, offset);
                }
            }
            ast::Statement::For(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    Self::collect_stack_locals_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                Self::collect_stack_locals_from_block(&for_stmt.body.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Range(range_stmt) => {
                Self::collect_stack_locals_from_block(&range_stmt.body.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Block(block) => {
                Self::collect_stack_locals_from_block(&block.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Switch(sw) => {
                if let Some(init) = &sw.init {
                    Self::collect_stack_locals_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                for clause in &sw.block.body {
                    Self::collect_stack_locals_from_block(&clause.body, escaping, struct_defs, locals, offset);
                }
            }
            ast::Statement::TypeSwitch(tsw) => {
                if let Some(init) = &tsw.init {
                    Self::collect_stack_locals_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                if let Some(tag) = &tsw.tag {
                    Self::collect_stack_locals_from_stmt(tag, escaping, struct_defs, locals, offset);
                }
                for clause in &tsw.block.body {
                    Self::collect_stack_locals_from_block(&clause.body, escaping, struct_defs, locals, offset);
                }
            }
            ast::Statement::Label(labeled) => {
                Self::collect_stack_locals_from_stmt(&labeled.stmt, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Select(sel) => {
                for clause in &sel.body.body {
                    if let Some(comm) = &clause.comm {
                        Self::collect_stack_locals_from_stmt(comm, escaping, struct_defs, locals, offset);
                    }
                    Self::collect_stack_locals_from_block(&clause.body, escaping, struct_defs, locals, offset);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn analyze_address_taken_vars(body: &ast::BlockStmt) -> HashSet<String> {
        let mut taken = HashSet::new();
        for stmt in &body.list {
            Self::scan_address_taken_stmt(stmt, &mut taken);
        }
        taken
    }

    pub(crate) fn scan_address_taken_stmt(stmt: &ast::Statement, taken: &mut HashSet<String>) {
        match stmt {
            ast::Statement::Assign(assign) => {
                for expr in &assign.right {
                    Self::scan_address_taken_expr(expr, taken);
                }
                for expr in &assign.left {
                    Self::scan_address_taken_expr(expr, taken);
                }
            }
            ast::Statement::Declaration(decl_stmt) => {
                if let ast::DeclStmt::Variable(var_decl) = decl_stmt {
                    for spec in &var_decl.specs {
                        for val in &spec.values {
                            Self::scan_address_taken_expr(val, taken);
                        }
                    }
                }
            }
            ast::Statement::Return(ret) => {
                for expr in &ret.ret {
                    Self::scan_address_taken_expr(expr, taken);
                }
            }
            ast::Statement::Expr(es) => {
                Self::scan_address_taken_expr(&es.expr, taken);
            }
            ast::Statement::If(if_stmt) => {
                if let Some(init) = &if_stmt.init {
                    Self::scan_address_taken_stmt(init, taken);
                }
                Self::scan_address_taken_expr(&if_stmt.cond, taken);
                for s in &if_stmt.body.list {
                    Self::scan_address_taken_stmt(s, taken);
                }
                if let Some(else_) = &if_stmt.else_ {
                    Self::scan_address_taken_stmt(else_, taken);
                }
            }
            ast::Statement::For(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    Self::scan_address_taken_stmt(init, taken);
                }
                if let Some(cond) = &for_stmt.cond {
                    Self::scan_address_taken_stmt(cond, taken);
                }
                if let Some(post) = &for_stmt.post {
                    Self::scan_address_taken_stmt(post, taken);
                }
                for s in &for_stmt.body.list {
                    Self::scan_address_taken_stmt(s, taken);
                }
            }
            ast::Statement::Range(range_stmt) => {
                Self::scan_address_taken_expr(&range_stmt.expr, taken);
                for s in &range_stmt.body.list {
                    Self::scan_address_taken_stmt(s, taken);
                }
            }
            ast::Statement::Block(block) => {
                for s in &block.list {
                    Self::scan_address_taken_stmt(s, taken);
                }
            }
            ast::Statement::Switch(sw) => {
                if let Some(init) = &sw.init {
                    Self::scan_address_taken_stmt(init, taken);
                }
                if let Some(tag) = &sw.tag {
                    Self::scan_address_taken_expr(tag, taken);
                }
                for clause in &sw.block.body {
                    for e in &clause.list {
                        Self::scan_address_taken_expr(e, taken);
                    }
                    for s in clause.body.iter() {
                        Self::scan_address_taken_stmt(s, taken);
                    }
                }
            }
            ast::Statement::IncDec(incdec) => {
                Self::scan_address_taken_expr(&incdec.expr, taken);
            }
            ast::Statement::Label(labeled) => {
                Self::scan_address_taken_stmt(&labeled.stmt, taken);
            }
            _ => {}
        }
    }

    pub(crate) fn scan_address_taken_expr(expr: &ast::Expression, taken: &mut HashSet<String>) {
        match expr {
            ast::Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
                if let ast::Expression::Ident(id) = op.x.as_ref() {
                    taken.insert(id.name.clone());
                }
                Self::scan_address_taken_expr(&op.x, taken);
            }
            ast::Expression::Operation(op) => {
                Self::scan_address_taken_expr(&op.x, taken);
                if let Some(ref y) = op.y {
                    Self::scan_address_taken_expr(y, taken);
                }
            }
            ast::Expression::Call(call) => {
                Self::scan_address_taken_expr(&call.func, taken);
                for arg in &call.args {
                    Self::scan_address_taken_expr(arg, taken);
                }
            }
            ast::Expression::Paren(p) => Self::scan_address_taken_expr(&p.expr, taken),
            ast::Expression::Index(idx) => {
                if let Some(ref left) = idx.left {
                    Self::scan_address_taken_expr(left, taken);
                }
                Self::scan_address_taken_expr(&idx.index, taken);
            }
            ast::Expression::Selector(sel) => {
                Self::scan_address_taken_expr(&sel.x, taken);
            }
            ast::Expression::CompositeLit(comp) => {
                for kv in &comp.val.values {
                    if let Some(key) = &kv.key {
                        if let ast::Element::Expr(e) = key {
                            Self::scan_address_taken_expr(e, taken);
                        }
                    }
                    if let ast::Element::Expr(e) = &kv.val {
                        Self::scan_address_taken_expr(e, taken);
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn struct_defs_contains(name: &str, struct_defs: &HashMap<String, StructDef>) -> bool {
        struct_defs.contains_key(name)
            || struct_defs.keys().any(|k| k.ends_with(&format!(".{}", name)))
    }

    pub(crate) fn struct_defs_get<'a>(name: &str, struct_defs: &'a HashMap<String, StructDef>) -> Option<&'a StructDef> {
        struct_defs.get(name)
            .or_else(|| struct_defs.iter().find(|(k, _)| k.ends_with(&format!(".{}", name))).map(|(_, v)| v))
    }

    pub(crate) fn is_struct_expr(expr: &ast::Expression, struct_defs: &HashMap<String, StructDef>) -> bool {
        match expr {
            ast::Expression::CompositeLit(comp) => {
                if let ast::Expression::Ident(ti) = comp.typ.as_ref() {
                    Self::struct_defs_contains(&ti.name, struct_defs)
                } else { false }
            }
            ast::Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
                Self::is_struct_expr(&op.x, struct_defs)
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(fn_id) = call.func.as_ref() {
                    if fn_id.name == "new" {
                        if let Some(ast::Expression::Ident(ti)) = call.args.first() {
                            return Self::struct_defs_contains(&ti.name, struct_defs);
                        }
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub(crate) fn collect_escaped_scalars_from_block(
        stmts: &[ast::Statement],
        escaping: &HashSet<String>,
        struct_defs: &HashMap<String, StructDef>,
        locals: &mut Vec<StackLocal>,
        offset: &mut u32,
    ) {
        for stmt in stmts {
            Self::collect_escaped_scalars_from_stmt(stmt, escaping, struct_defs, locals, offset);
        }
    }

    pub(crate) fn collect_escaped_scalars_from_stmt(
        stmt: &ast::Statement,
        escaping: &HashSet<String>,
        struct_defs: &HashMap<String, StructDef>,
        locals: &mut Vec<StackLocal>,
        offset: &mut u32,
    ) {
        match stmt {
            ast::Statement::Assign(assign) if matches!(assign.op, Operator::Define) => {
                for (idx, lhs) in assign.left.iter().enumerate() {
                    if let ast::Expression::Ident(id) = lhs {
                        if !escaping.contains(&id.name) {
                            continue;
                        }
                        if locals.iter().any(|l| l.name == id.name) {
                            continue;
                        }
                        let rhs_is_struct = if let Some(rhs) = assign.right.get(idx) {
                            Self::is_struct_expr(rhs, struct_defs)
                        } else { false };
                        if rhs_is_struct {
                            continue;
                        }
                        let aligned = (*offset + 7) & !7;
                        locals.push(StackLocal {
                            name: id.name.clone(),
                            offset: aligned,
                            size: 8,
                        });
                        *offset = aligned + 8;
                    }
                }
            }
            ast::Statement::Declaration(decl_stmt) => {
                if let ast::DeclStmt::Variable(var_decl) = decl_stmt {
                    for spec in &var_decl.specs {
                        for name in &spec.name {
                            if !escaping.contains(&name.name) {
                                continue;
                            }
                            if locals.iter().any(|l| l.name == name.name) {
                                continue;
                            }
                            let is_struct = if let Some(ref typ) = spec.typ {
                                if let ast::Expression::Ident(ti) = typ {
                                    Self::struct_defs_contains(&ti.name, struct_defs)
                                } else { false }
                            } else { false };
                            if is_struct {
                                continue;
                            }
                            let aligned = (*offset + 7) & !7;
                            locals.push(StackLocal {
                                name: name.name.clone(),
                                offset: aligned,
                                size: 8,
                            });
                            *offset = aligned + 8;
                        }
                    }
                }
            }
            ast::Statement::If(if_stmt) => {
                if let Some(init) = &if_stmt.init {
                    Self::collect_escaped_scalars_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                Self::collect_escaped_scalars_from_block(&if_stmt.body.list, escaping, struct_defs, locals, offset);
                if let Some(else_) = &if_stmt.else_ {
                    Self::collect_escaped_scalars_from_stmt(else_, escaping, struct_defs, locals, offset);
                }
            }
            ast::Statement::For(for_stmt) => {
                if let Some(init) = &for_stmt.init {
                    Self::collect_escaped_scalars_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                Self::collect_escaped_scalars_from_block(&for_stmt.body.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Range(range_stmt) => {
                Self::collect_escaped_scalars_from_block(&range_stmt.body.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Block(block) => {
                Self::collect_escaped_scalars_from_block(&block.list, escaping, struct_defs, locals, offset);
            }
            ast::Statement::Switch(sw) => {
                if let Some(init) = &sw.init {
                    Self::collect_escaped_scalars_from_stmt(init, escaping, struct_defs, locals, offset);
                }
                for clause in &sw.block.body {
                    Self::collect_escaped_scalars_from_block(&clause.body, escaping, struct_defs, locals, offset);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn infer_compound_alloc_size(
        expr: &ast::Expression,
        struct_defs: &HashMap<String, StructDef>,
    ) -> Option<u32> {
        match expr {
            ast::Expression::CompositeLit(comp) => {
                match comp.typ.as_ref() {
                    ast::Expression::Ident(id) => {
                        if let Some(sdef) = struct_defs.get(&id.name) {
                            Some(sdef.total_size)
                        } else {
                            None
                        }
                    }
                    ast::Expression::TypeSlice(_) => None,
                    ast::Expression::TypeMap(_) => None,
                    ast::Expression::TypeArray(arr) => {
                        if let ast::Expression::BasicLit(lit) = arr.len.as_ref() {
                            if let Ok(n) = lit.value.parse::<u32>() {
                                let elem_size = Self::infer_type_expr_size(&arr.typ, struct_defs).unwrap_or(8);
                                n.checked_mul(elem_size)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            }
            ast::Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
                Self::infer_compound_alloc_size(&op.x, struct_defs)
            }
            ast::Expression::Call(call) => {
                if let ast::Expression::Ident(id) = call.func.as_ref() {
                    if id.name == "new" {
                        if let Some(type_arg) = call.args.first() {
                            if let ast::Expression::Ident(ti) = type_arg {
                                if let Some(sdef) = struct_defs.get(&ti.name) {
                                    return Some(sdef.total_size.max(8));
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    pub(crate) fn infer_type_expr_size(
        typ: &ast::Expression,
        struct_defs: &HashMap<String, StructDef>,
    ) -> Option<u32> {
        match typ {
            ast::Expression::Ident(id) => {
                if let Some(sdef) = Self::struct_defs_get(&id.name, struct_defs) {
                    Some(sdef.total_size)
                } else {
                    match id.name.as_str() {
                        "int" | "int64" | "uint" | "uint64" | "float64" => Some(8),
                        "int32" | "uint32" | "float32" | "rune" => Some(4),
                        "int16" | "uint16" => Some(2),
                        "int8" | "uint8" | "byte" | "bool" => Some(1),
                        "string" => None,
                        _ => None,
                    }
                }
            }
            ast::Expression::TypeSlice(_) => Some(12),
            ast::Expression::TypeMap(_) => Some(20),
            ast::Expression::TypeArray(arr) => {
                if let ast::Expression::BasicLit(lit) = arr.len.as_ref() {
                    if let Ok(n) = lit.value.parse::<u32>() {
                        let elem_size = Self::infer_type_expr_size(&arr.typ, struct_defs).unwrap_or(8);
                        n.checked_mul(elem_size)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
