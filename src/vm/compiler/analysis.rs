use std::collections::HashSet;

use crate::parser::ast::*;
use crate::parser::token::Operator;

pub(crate) fn analyze_address_taken_vars(body: &[Statement]) -> HashSet<String> {
    let mut taken = HashSet::new();
    for stmt in body {
        scan_address_taken_stmt(stmt, &mut taken);
    }
    taken
}

fn scan_address_taken_stmt(stmt: &Statement, taken: &mut HashSet<String>) {
    match stmt {
        Statement::Assign(assign) => {
            for expr in &assign.right {
                scan_address_taken_expr(expr, taken);
            }
            for expr in &assign.left {
                scan_address_taken_expr(expr, taken);
            }
        }
        Statement::Declaration(decl_stmt) => {
            if let DeclStmt::Variable(var_decl) = decl_stmt {
                for spec in &var_decl.specs {
                    for val in &spec.values {
                        scan_address_taken_expr(val, taken);
                    }
                }
            }
        }
        Statement::Return(ret) => {
            for expr in &ret.ret {
                scan_address_taken_expr(expr, taken);
            }
        }
        Statement::Expr(es) => {
            scan_address_taken_expr(&es.expr, taken);
        }
        Statement::If(if_stmt) => {
            if let Some(init) = &if_stmt.init {
                scan_address_taken_stmt(init, taken);
            }
            scan_address_taken_expr(&if_stmt.cond, taken);
            for s in &if_stmt.body.list {
                scan_address_taken_stmt(s, taken);
            }
            if let Some(else_) = &if_stmt.else_ {
                scan_address_taken_stmt(else_, taken);
            }
        }
        Statement::For(for_stmt) => {
            if let Some(init) = &for_stmt.init {
                scan_address_taken_stmt(init, taken);
            }
            if let Some(cond) = &for_stmt.cond {
                scan_address_taken_stmt(cond, taken);
            }
            if let Some(post) = &for_stmt.post {
                scan_address_taken_stmt(post, taken);
            }
            for s in &for_stmt.body.list {
                scan_address_taken_stmt(s, taken);
            }
        }
        Statement::Range(range_stmt) => {
            scan_address_taken_expr(&range_stmt.expr, taken);
            for s in &range_stmt.body.list {
                scan_address_taken_stmt(s, taken);
            }
        }
        Statement::Block(block) => {
            for s in &block.list {
                scan_address_taken_stmt(s, taken);
            }
        }
        Statement::Switch(sw) => {
            if let Some(init) = &sw.init {
                scan_address_taken_stmt(init, taken);
            }
            if let Some(tag) = &sw.tag {
                scan_address_taken_expr(tag, taken);
            }
            for clause in &sw.block.body {
                for e in &clause.list {
                    scan_address_taken_expr(e, taken);
                }
                for s in clause.body.iter() {
                    scan_address_taken_stmt(s, taken);
                }
            }
        }
        Statement::IncDec(incdec) => {
            scan_address_taken_expr(&incdec.expr, taken);
        }
        Statement::Label(labeled) => {
            scan_address_taken_stmt(&labeled.stmt, taken);
        }
        _ => {}
    }
}

fn scan_address_taken_expr(expr: &Expression, taken: &mut HashSet<String>) {
    match expr {
        Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
            if let Expression::Ident(id) = op.x.as_ref() {
                taken.insert(id.name.clone());
            }
            scan_address_taken_expr(&op.x, taken);
        }
        Expression::Operation(op) => {
            scan_address_taken_expr(&op.x, taken);
            if let Some(ref y) = op.y {
                scan_address_taken_expr(y, taken);
            }
        }
        Expression::Call(call) => {
            scan_address_taken_expr(&call.func, taken);
            for arg in &call.args {
                scan_address_taken_expr(arg, taken);
            }
        }
        Expression::Paren(p) => scan_address_taken_expr(&p.expr, taken),
        Expression::Index(idx) => {
            scan_address_taken_expr(&idx.left, taken);
            scan_address_taken_expr(&idx.index, taken);
        }
        Expression::Selector(sel) => {
            scan_address_taken_expr(&sel.x, taken);
        }
        Expression::CompositeLit(comp) => {
            for kv in &comp.val.values {
                if let Some(key) = &kv.key {
                    if let Element::Expr(e) = key {
                        scan_address_taken_expr(e, taken);
                    }
                }
                if let Element::Expr(e) = &kv.val {
                    scan_address_taken_expr(e, taken);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Escape analysis
// ---------------------------------------------------------------------------

pub(crate) fn analyze_function_escapes(body: &[Statement]) -> HashSet<String> {
    let mut escaping = HashSet::new();
    let mut assignments: Vec<(String, String)> = Vec::new();

    for stmt in body {
        escape_scan_stmt(stmt, &mut escaping, &mut assignments);
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

fn escape_scan_stmt(
    stmt: &Statement,
    escaping: &mut HashSet<String>,
    assignments: &mut Vec<(String, String)>,
) {
    match stmt {
        Statement::Return(ret) => {
            for expr in &ret.ret {
                mark_expr_escaping(expr, escaping);
            }
        }
        Statement::Assign(assign) => {
            for (i, lhs) in assign.left.iter().enumerate() {
                match lhs {
                    Expression::Selector(_)
                    | Expression::Index(_)
                    | Expression::Star(_) => {
                        if let Some(rhs) = assign.right.get(i) {
                            mark_expr_escaping(rhs, escaping);
                        }
                    }
                    Expression::Operation(op)
                        if op.y.is_none() && matches!(op.op, Operator::Star) =>
                    {
                        if let Some(rhs) = assign.right.get(i) {
                            mark_expr_escaping(rhs, escaping);
                        }
                    }
                    Expression::Ident(dst_id) => {
                        if let Some(rhs) = assign.right.get(i) {
                            if let Expression::Ident(src_id) = rhs {
                                assignments.push((dst_id.name.clone(), src_id.name.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }

            for expr in &assign.right {
                escape_scan_call_args(expr, escaping);
            }
        }
        Statement::Expr(es) => {
            escape_scan_call_args(&es.expr, escaping);
        }
        Statement::If(if_stmt) => {
            if let Some(init) = &if_stmt.init {
                escape_scan_stmt(init, escaping, assignments);
            }
            escape_scan_call_args(&if_stmt.cond, escaping);
            for s in &if_stmt.body.list {
                escape_scan_stmt(s, escaping, assignments);
            }
            if let Some(else_) = &if_stmt.else_ {
                escape_scan_stmt(else_, escaping, assignments);
            }
        }
        Statement::For(for_stmt) => {
            if let Some(init) = &for_stmt.init {
                escape_scan_stmt(init, escaping, assignments);
            }
            if let Some(cond) = &for_stmt.cond {
                escape_scan_stmt(cond, escaping, assignments);
            }
            if let Some(post) = &for_stmt.post {
                escape_scan_stmt(post, escaping, assignments);
            }
            for s in &for_stmt.body.list {
                escape_scan_stmt(s, escaping, assignments);
            }
        }
        Statement::Range(range_stmt) => {
            escape_scan_call_args(&range_stmt.expr, escaping);
            for s in &range_stmt.body.list {
                escape_scan_stmt(s, escaping, assignments);
            }
        }
        Statement::Block(block) => {
            for s in &block.list {
                escape_scan_stmt(s, escaping, assignments);
            }
        }
        Statement::Switch(sw) => {
            if let Some(init) = &sw.init {
                escape_scan_stmt(init, escaping, assignments);
            }
            if let Some(tag) = &sw.tag {
                escape_scan_call_args(tag, escaping);
            }
            for clause in &sw.block.body {
                for e in &clause.list {
                    escape_scan_call_args(e, escaping);
                }
                for s in clause.body.iter() {
                    escape_scan_stmt(s, escaping, assignments);
                }
            }
        }
        Statement::Send(send) => {
            mark_expr_escaping(&send.value, escaping);
            escape_scan_call_args(&send.value, escaping);
            escape_scan_call_args(&send.chan, escaping);
        }
        Statement::Go(go_stmt) => {
            for arg in &go_stmt.call.args {
                mark_expr_escaping(arg, escaping);
            }
        }
        Statement::Defer(defer_stmt) => {
            for arg in &defer_stmt.call.args {
                mark_expr_escaping(arg, escaping);
            }
        }
        Statement::Declaration(decl_stmt) => {
            if let DeclStmt::Variable(var_decl) = decl_stmt {
                for spec in &var_decl.specs {
                    for val in &spec.values {
                        escape_scan_call_args(val, escaping);
                    }
                    for (i, name) in spec.name.iter().enumerate() {
                        if let Some(val) = spec.values.get(i) {
                            if let Expression::Ident(src_id) = val {
                                assignments.push((name.name.clone(), src_id.name.clone()));
                            }
                        }
                    }
                }
            }
        }
        Statement::Label(labeled) => {
            escape_scan_stmt(&labeled.stmt, escaping, assignments);
        }
        Statement::IncDec(incdec) => {
            escape_scan_call_args(&incdec.expr, escaping);
        }
        _ => {}
    }
}

fn mark_expr_escaping(expr: &Expression, escaping: &mut HashSet<String>) {
    match expr {
        Expression::Ident(id) => {
            escaping.insert(id.name.clone());
        }
        Expression::Paren(p) => mark_expr_escaping(&p.expr, escaping),
        Expression::Star(s) => mark_expr_escaping(&s.right, escaping),
        Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => {
            mark_expr_escaping(&op.x, escaping);
        }
        Expression::Selector(sel) => mark_expr_escaping(&sel.x, escaping),
        Expression::Index(idx) => {
            mark_expr_escaping(&idx.left, escaping);
        }
        Expression::Slice(sl) => mark_expr_escaping(&sl.left, escaping),
        Expression::FuncLit(fl) => {
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
            collect_free_vars_block(&fl.body, &mut bound, &mut captured);
            for name in &captured {
                escaping.insert(name.clone());
            }
        }
        Expression::CompositeLit(comp) => {
            for kv in &comp.val.values {
                if let Some(key) = &kv.key {
                    mark_escaping_element(key, escaping);
                }
                mark_escaping_element(&kv.val, escaping);
            }
        }
        Expression::Invar(inv) => mark_expr_escaping(&inv.expr, escaping),
        _ => {}
    }
}

fn mark_escaping_element(elem: &Element, escaping: &mut HashSet<String>) {
    match elem {
        Element::Expr(e) => mark_expr_escaping(e, escaping),
        Element::LitValue(lv) => {
            for kv in &lv.values {
                if let Some(key) = &kv.key {
                    mark_escaping_element(key, escaping);
                }
                mark_escaping_element(&kv.val, escaping);
            }
        }
    }
}

/// Only marks escaping when the argument contains an address-of (`&`) operation.
/// Plain value-type arguments passed by copy cannot cause the original to escape.
fn escape_scan_call_args(expr: &Expression, escaping: &mut HashSet<String>) {
    match expr {
        Expression::Call(call) => {
            for arg in &call.args {
                if contains_address_of(arg) {
                    mark_expr_escaping(arg, escaping);
                }
                escape_scan_call_args(arg, escaping);
            }
            escape_scan_call_args(&call.func, escaping);
        }
        Expression::Operation(op) => {
            escape_scan_call_args(&op.x, escaping);
            if let Some(ref y) = op.y {
                escape_scan_call_args(y, escaping);
            }
            if op.y.is_none() && matches!(op.op, Operator::And) {
                mark_expr_escaping(&op.x, escaping);
            }
        }
        Expression::Paren(p) => escape_scan_call_args(&p.expr, escaping),
        Expression::CompositeLit(comp) => {
            for kv in &comp.val.values {
                if let Some(key) = &kv.key {
                    escape_scan_element(key, escaping);
                }
                escape_scan_element(&kv.val, escaping);
            }
        }
        Expression::FuncLit(fl) => {
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
            collect_free_vars_block(&fl.body, &mut bound, &mut captured);
            for name in &captured {
                escaping.insert(name.clone());
            }
        }
        Expression::Selector(sel) => escape_scan_call_args(&sel.x, escaping),
        Expression::Index(idx) => {
            escape_scan_call_args(&idx.left, escaping);
            escape_scan_call_args(&idx.index, escaping);
        }
        Expression::Slice(sl) => {
            escape_scan_call_args(&sl.left, escaping);
            for opt_idx in &sl.index {
                if let Some(e) = opt_idx {
                    escape_scan_call_args(e, escaping);
                }
            }
        }
        Expression::Star(s) => {
            escape_scan_call_args(&s.right, escaping);
        }
        Expression::Invar(inv) => {
            escape_scan_call_args(&inv.expr, escaping);
        }
        _ => {}
    }
}

fn escape_scan_element(elem: &Element, escaping: &mut HashSet<String>) {
    match elem {
        Element::Expr(e) => {
            mark_expr_escaping(e, escaping);
            escape_scan_call_args(e, escaping);
        }
        Element::LitValue(lv) => {
            for kv in &lv.values {
                if let Some(key) = &kv.key {
                    escape_scan_element(key, escaping);
                }
                escape_scan_element(&kv.val, escaping);
            }
        }
    }
}

fn contains_address_of(expr: &Expression) -> bool {
    match expr {
        Expression::Operation(op) if op.y.is_none() && matches!(op.op, Operator::And) => true,
        Expression::Operation(op) => {
            contains_address_of(&op.x) || op.y.as_ref().map_or(false, |y| contains_address_of(y))
        }
        Expression::Paren(p) => contains_address_of(&p.expr),
        Expression::Call(call) => {
            call.args.iter().any(|a| contains_address_of(a))
                || contains_address_of(&call.func)
        }
        Expression::CompositeLit(comp) => {
            comp.val.values.iter().any(|kv| {
                kv.key.as_ref().map_or(false, |k| contains_address_of_element(k))
                    || contains_address_of_element(&kv.val)
            })
        }
        _ => false,
    }
}

fn contains_address_of_element(elem: &Element) -> bool {
    match elem {
        Element::Expr(e) => contains_address_of(e),
        Element::LitValue(lv) => lv.values.iter().any(|kv| {
            kv.key.as_ref().map_or(false, |k| contains_address_of_element(k))
                || contains_address_of_element(&kv.val)
        }),
    }
}

// ---------------------------------------------------------------------------
// Free variable collection (for closure captures in escape analysis)
// ---------------------------------------------------------------------------

fn collect_free_vars_block(
    block: &BlockStmt,
    bound: &mut HashSet<String>,
    free: &mut HashSet<String>,
) {
    for stmt in &block.list {
        collect_free_vars_stmt(stmt, bound, free);
    }
}

fn collect_free_vars_stmt(
    stmt: &Statement,
    bound: &mut HashSet<String>,
    free: &mut HashSet<String>,
) {
    match stmt {
        Statement::Assign(assign) => {
            for expr in &assign.right {
                collect_free_vars_expr(expr, bound, free);
            }
            if matches!(assign.op, Operator::Define) {
                for lhs in &assign.left {
                    if let Expression::Ident(id) = lhs {
                        bound.insert(id.name.clone());
                    }
                }
            } else {
                for lhs in &assign.left {
                    collect_free_vars_expr(lhs, bound, free);
                }
            }
        }
        Statement::Declaration(decl_stmt) => {
            if let DeclStmt::Variable(var_decl) = decl_stmt {
                for spec in &var_decl.specs {
                    for val in &spec.values {
                        collect_free_vars_expr(val, bound, free);
                    }
                    for name in &spec.name {
                        bound.insert(name.name.clone());
                    }
                }
            }
        }
        Statement::Return(ret) => {
            for expr in &ret.ret {
                collect_free_vars_expr(expr, bound, free);
            }
        }
        Statement::Expr(es) => {
            collect_free_vars_expr(&es.expr, bound, free);
        }
        Statement::If(if_stmt) => {
            if let Some(init) = &if_stmt.init {
                collect_free_vars_stmt(init, bound, free);
            }
            collect_free_vars_expr(&if_stmt.cond, bound, free);
            collect_free_vars_block(&if_stmt.body, bound, free);
            if let Some(else_) = &if_stmt.else_ {
                collect_free_vars_stmt(else_, bound, free);
            }
        }
        Statement::For(for_stmt) => {
            if let Some(init) = &for_stmt.init {
                collect_free_vars_stmt(init, bound, free);
            }
            if let Some(cond) = &for_stmt.cond {
                collect_free_vars_stmt(cond, bound, free);
            }
            if let Some(post) = &for_stmt.post {
                collect_free_vars_stmt(post, bound, free);
            }
            collect_free_vars_block(&for_stmt.body, bound, free);
        }
        Statement::Range(range_stmt) => {
            collect_free_vars_expr(&range_stmt.expr, bound, free);
            if let Some(Expression::Ident(id)) = &range_stmt.key {
                bound.insert(id.name.clone());
            }
            if let Some(Expression::Ident(id)) = &range_stmt.value {
                bound.insert(id.name.clone());
            }
            collect_free_vars_block(&range_stmt.body, bound, free);
        }
        Statement::Block(block) => {
            collect_free_vars_block(block, bound, free);
        }
        Statement::Switch(sw) => {
            if let Some(init) = &sw.init {
                collect_free_vars_stmt(init, bound, free);
            }
            if let Some(tag) = &sw.tag {
                collect_free_vars_expr(tag, bound, free);
            }
            for clause in &sw.block.body {
                for e in &clause.list {
                    collect_free_vars_expr(e, bound, free);
                }
                for s in clause.body.iter() {
                    collect_free_vars_stmt(s, bound, free);
                }
            }
        }
        Statement::Send(send) => {
            collect_free_vars_expr(&send.chan, bound, free);
            collect_free_vars_expr(&send.value, bound, free);
        }
        Statement::Go(go_stmt) => {
            collect_free_vars_expr(&go_stmt.call.func, bound, free);
            for arg in &go_stmt.call.args {
                collect_free_vars_expr(arg, bound, free);
            }
        }
        Statement::Defer(defer_stmt) => {
            collect_free_vars_expr(&defer_stmt.call.func, bound, free);
            for arg in &defer_stmt.call.args {
                collect_free_vars_expr(arg, bound, free);
            }
        }
        Statement::IncDec(incdec) => {
            collect_free_vars_expr(&incdec.expr, bound, free);
        }
        Statement::Label(labeled) => {
            collect_free_vars_stmt(&labeled.stmt, bound, free);
        }
        _ => {}
    }
}

fn collect_free_vars_expr(
    expr: &Expression,
    bound: &HashSet<String>,
    free: &mut HashSet<String>,
) {
    match expr {
        Expression::Ident(id) => {
            if !bound.contains(&id.name) {
                free.insert(id.name.clone());
            }
        }
        Expression::Operation(op) => {
            collect_free_vars_expr(&op.x, bound, free);
            if let Some(ref y) = op.y {
                collect_free_vars_expr(y, bound, free);
            }
        }
        Expression::Call(call) => {
            collect_free_vars_expr(&call.func, bound, free);
            for arg in &call.args {
                collect_free_vars_expr(arg, bound, free);
            }
        }
        Expression::Paren(p) => collect_free_vars_expr(&p.expr, bound, free),
        Expression::Index(idx) => {
            collect_free_vars_expr(&idx.left, bound, free);
            collect_free_vars_expr(&idx.index, bound, free);
        }
        Expression::Selector(sel) => {
            collect_free_vars_expr(&sel.x, bound, free);
        }
        Expression::Star(s) => {
            collect_free_vars_expr(&s.right, bound, free);
        }
        Expression::CompositeLit(comp) => {
            for kv in &comp.val.values {
                if let Some(key) = &kv.key {
                    collect_free_vars_element(key, bound, free);
                }
                collect_free_vars_element(&kv.val, bound, free);
            }
        }
        Expression::FuncLit(fl) => {
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
            collect_free_vars_block(&fl.body, &mut inner_bound, free);
        }
        Expression::Slice(sl) => {
            collect_free_vars_expr(&sl.left, bound, free);
            for opt in &sl.index {
                if let Some(e) = opt {
                    collect_free_vars_expr(e, bound, free);
                }
            }
        }
        Expression::Invar(inv) => collect_free_vars_expr(&inv.expr, bound, free),
        _ => {}
    }
}

fn collect_free_vars_element(
    elem: &Element,
    bound: &HashSet<String>,
    free: &mut HashSet<String>,
) {
    match elem {
        Element::Expr(e) => collect_free_vars_expr(e, bound, free),
        Element::LitValue(lv) => {
            for kv in &lv.values {
                if let Some(key) = &kv.key {
                    collect_free_vars_element(key, bound, free);
                }
                collect_free_vars_element(&kv.val, bound, free);
            }
        }
    }
}
