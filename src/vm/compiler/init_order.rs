use crate::parser::ast::{
    Decl, DeclStmt, Declaration, Element, Expression, Package, Statement,
};
use crate::vm::builtin;
use crate::vm::module::ModuleResolver;
use ahash::{HashMap, HashMapExt};
use std::collections::{BinaryHeap, HashSet, VecDeque};
use std::cmp::Ordering;

type NodeId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Const,
    Var,
    Type,
    Func,
}

#[derive(Debug, Clone)]
struct InitNode {
    id: NodeId,
    name: String,
    kind: DeclKind,
    source_order: u32,
    successors: HashSet<NodeId>,
    predecessors: HashSet<NodeId>,
}

// ============================================================
// Pass 1: Register all package-level declarations
// ============================================================

struct InitGraph {
    nodes: Vec<InitNode>,
    name_to_id: HashMap<String, NodeId>,
    id_to_decl: HashMap<NodeId, Declaration>,
}

impl InitGraph {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            name_to_id: HashMap::new(),
            id_to_decl: HashMap::new(),
        }
    }

    fn add_node(&mut self, name: String, kind: DeclKind, decl: Declaration) -> NodeId {
        let id = self.nodes.len() as NodeId;
        let source_order = id;
        self.nodes.push(InitNode {
            id,
            name: name.clone(),
            kind,
            source_order,
            successors: HashSet::new(),
            predecessors: HashSet::new(),
        });
        self.name_to_id.insert(name, id);
        self.id_to_decl.insert(id, decl);
        id
    }

    fn add_edge(&mut self, from: NodeId, to: NodeId) {
        if from == to {
            return;
        }
        self.nodes[from as usize].successors.insert(to);
        self.nodes[to as usize].predecessors.insert(from);
    }

    fn lookup(&self, name: &str) -> Option<NodeId> {
        self.name_to_id.get(name).copied()
    }

    fn register_declarations(&mut self, decls: &[Declaration]) {
        for decl in decls {
            match decl {
                Declaration::Type(tspec) => {
                    for spec in &tspec.specs {
                        self.add_node(
                            spec.name.name.clone(),
                            DeclKind::Type,
                            decl.clone(),
                        );
                    }
                }
                Declaration::Const(c) => {
                    let has_implicit = c.specs.iter().any(|s| s.values.is_empty());
                    if has_implicit {
                        for spec in &c.specs {
                            for name in &spec.name {
                                self.add_node(
                                    name.name.clone(),
                                    DeclKind::Const,
                                    decl.clone(),
                                );
                            }
                        }
                    } else {
                        for spec in &c.specs {
                            let single_spec_decl = Declaration::Const(Decl {
                                docs: c.docs.clone(),
                                pos0: c.pos0,
                                pos1: c.pos1,
                                specs: vec![spec.clone()],
                            });
                            for name in &spec.name {
                                self.add_node(
                                    name.name.clone(),
                                    DeclKind::Const,
                                    single_spec_decl.clone(),
                                );
                            }
                        }
                    }
                }
                Declaration::Variable(v) => {
                    for spec in &v.specs {
                        let single_spec_decl = Declaration::Variable(Decl {
                            docs: v.docs.clone(),
                            pos0: v.pos0,
                            pos1: v.pos1,
                            specs: vec![spec.clone()],
                        });
                        for name in &spec.name {
                            self.add_node(
                                name.name.clone(),
                                DeclKind::Var,
                                single_spec_decl.clone(),
                            );
                        }
                    }
                }
                Declaration::Function(f) => {
                    if f.recv.is_some() {
                        let recv = f.recv.as_ref().unwrap();
                        let recv_type_name = extract_receiver_type_name(recv);
                        let method_key = format!("{}::{}", recv_type_name, f.name.name);
                        self.add_node(method_key, DeclKind::Func, decl.clone());
                    } else {
                        self.add_node(
                            f.name.name.clone(),
                            DeclKind::Func,
                            decl.clone(),
                        );
                    }
                }
            }
        }
    }
}

fn extract_receiver_type_name(recv: &crate::parser::ast::FieldList) -> String {
    if let Some(field) = recv.list.first() {
        return extract_type_name_from_expr(&field.typ);
    }
    String::new()
}

fn extract_type_name_from_expr(expr: &Expression) -> String {
    match expr {
        Expression::Ident(id) => id.name.clone(),
        Expression::Star(star) => extract_type_name_from_expr(&star.right),
        Expression::TypePointer(ptr) => extract_type_name_from_expr(&ptr.typ),
        _ => String::new(),
    }
}

// ============================================================
// Pass 2: Walk AST to collect identifier references as edges
// ============================================================

fn collect_idents_from_expr(expr: &Expression, out: &mut Vec<String>) {
    match expr {
        Expression::Ident(id) => {
            if id.name != "_" && id.name != "iota" && builtin::resolve(&id.name).is_none() {
                out.push(id.name.clone());
            }
        }
        Expression::BasicLit(_) => {}
        Expression::Operation(op) => {
            collect_idents_from_expr(&op.x, out);
            if let Some(y) = &op.y {
                collect_idents_from_expr(y, out);
            }
        }
        Expression::Call(call) => {
            collect_idents_from_expr(&call.func, out);
            for arg in &call.args {
                collect_idents_from_expr(arg, out);
            }
        }
        Expression::Selector(sel) => {
            collect_idents_from_expr(&sel.x, out);
        }
        Expression::Index(idx) => {
            collect_idents_from_expr(&idx.left, out);
            collect_idents_from_expr(&idx.index, out);
        }
        Expression::IndexList(idx) => {
            collect_idents_from_expr(&idx.left, out);
            for i in &idx.indices {
                collect_idents_from_expr(i, out);
            }
        }
        Expression::Slice(sl) => {
            collect_idents_from_expr(&sl.left, out);
            for opt in &sl.index {
                if let Some(e) = opt {
                    collect_idents_from_expr(e, out);
                }
            }
        }
        Expression::FuncLit(fl) => {
            collect_idents_from_field_list(&fl.typ.params, out);
            collect_idents_from_field_list(&fl.typ.result, out);
            collect_idents_from_block(&fl.body.list, out);
        }
        Expression::Ellipsis(el) => {
            if let Some(e) = &el.elt {
                collect_idents_from_expr(e, out);
            }
        }
        Expression::Range(rng) => {
            collect_idents_from_expr(&rng.right, out);
        }
        Expression::Star(star) => {
            collect_idents_from_expr(&star.right, out);
        }
        Expression::Paren(p) => {
            collect_idents_from_expr(&p.expr, out);
        }
        Expression::TypeAssert(ta) => {
            collect_idents_from_expr(&ta.left, out);
            if let Some(r) = &ta.right {
                collect_idents_from_expr(r, out);
            }
        }
        Expression::CompositeLit(cl) => {
            collect_idents_from_expr(&cl.typ, out);
            for kv in &cl.val.values {
                if let Some(key) = &kv.key {
                    collect_idents_from_element(key, out);
                }
                collect_idents_from_element(&kv.val, out);
            }
        }
        Expression::List(exprs) => {
            for e in exprs {
                collect_idents_from_expr(e, out);
            }
        }
        Expression::Invar(inv) => {
            collect_idents_from_expr(&inv.expr, out);
        }
        Expression::TypeMap(m) => {
            collect_idents_from_expr(&m.key, out);
            collect_idents_from_expr(&m.val, out);
        }
        Expression::TypeArray(a) => {
            collect_idents_from_expr(&a.len, out);
            collect_idents_from_expr(&a.typ, out);
        }
        Expression::TypeSlice(s) => {
            collect_idents_from_expr(&s.typ, out);
        }
        Expression::TypeFunction(f) => {
            collect_idents_from_field_list(&f.params, out);
            collect_idents_from_field_list(&f.result, out);
        }
        Expression::TypeStruct(s) => {
            for field in &s.fields {
                collect_idents_from_expr(&field.typ, out);
            }
        }
        Expression::TypeChannel(ch) => {
            collect_idents_from_expr(&ch.typ, out);
        }
        Expression::TypePointer(p) => {
            collect_idents_from_expr(&p.typ, out);
        }
        Expression::TypeInterface(iface) => {
            for method in &iface.methods.list {
                collect_idents_from_expr(&method.typ, out);
            }
        }
    }
}

fn collect_idents_from_element(el: &Element, out: &mut Vec<String>) {
    match el {
        Element::Expr(e) => collect_idents_from_expr(e, out),
        Element::LitValue(lv) => {
            for kv in &lv.values {
                if let Some(k) = &kv.key {
                    collect_idents_from_element(k, out);
                }
                collect_idents_from_element(&kv.val, out);
            }
        }
    }
}

fn collect_idents_from_field_list(fl: &crate::parser::ast::FieldList, out: &mut Vec<String>) {
    for field in &fl.list {
        collect_idents_from_expr(&field.typ, out);
    }
}

fn collect_idents_from_stmt(stmt: &Statement, out: &mut Vec<String>) {
    match stmt {
        Statement::Expr(expr) => {
            collect_idents_from_expr(&expr.expr, out);
        }
        Statement::Return(ret) => {
            for r in &ret.ret {
                collect_idents_from_expr(r, out);
            }
        }
        Statement::If(ifstmt) => {
            if let Some(init) = &ifstmt.init {
                collect_idents_from_stmt(init, out);
            }
            collect_idents_from_expr(&ifstmt.cond, out);
            collect_idents_from_block(&ifstmt.body.list, out);
            if let Some(els) = &ifstmt.else_ {
                collect_idents_from_stmt(els, out);
            }
        }
        Statement::For(forstmt) => {
            if let Some(init) = &forstmt.init {
                collect_idents_from_stmt(init, out);
            }
            if let Some(cond) = &forstmt.cond {
                collect_idents_from_stmt(cond, out);
            }
            if let Some(post) = &forstmt.post {
                collect_idents_from_stmt(post, out);
            }
            collect_idents_from_block(&forstmt.body.list, out);
        }
        Statement::Range(rng) => {
            if let Some(k) = &rng.key {
                collect_idents_from_expr(k, out);
            }
            if let Some(v) = &rng.value {
                collect_idents_from_expr(v, out);
            }
            collect_idents_from_expr(&rng.expr, out);
            collect_idents_from_block(&rng.body.list, out);
        }
        Statement::Assign(assign) => {
            for l in &assign.left {
                collect_idents_from_expr(l, out);
            }
            for r in &assign.right {
                collect_idents_from_expr(r, out);
            }
        }
        Statement::Block(block) => {
            collect_idents_from_block(&block.list, out);
        }
        Statement::Declaration(declr) => match declr {
            DeclStmt::Variable(var) => {
                for spec in &var.specs {
                    if let Some(t) = &spec.typ {
                        collect_idents_from_expr(t, out);
                    }
                    for value in &spec.values {
                        collect_idents_from_expr(value, out);
                    }
                }
            }
            DeclStmt::Const(cnst) => {
                for spec in &cnst.specs {
                    if let Some(t) = &spec.typ {
                        collect_idents_from_expr(t, out);
                    }
                    for value in &spec.values {
                        collect_idents_from_expr(value, out);
                    }
                }
            }
            DeclStmt::Type(t) => {
                for spec in &t.specs {
                    collect_idents_from_expr(&spec.typ, out);
                }
            }
        },
        Statement::Switch(sw) => {
            if let Some(init) = &sw.init {
                collect_idents_from_stmt(init, out);
            }
            if let Some(tag) = &sw.tag {
                collect_idents_from_expr(tag, out);
            }
            for clause in &sw.block.body {
                for expr in &clause.list {
                    collect_idents_from_expr(expr, out);
                }
                for stmt in clause.body.as_ref() {
                    collect_idents_from_stmt(stmt, out);
                }
            }
        }
        Statement::TypeSwitch(sw) => {
            if let Some(init) = &sw.init {
                collect_idents_from_stmt(init, out);
            }
            if let Some(tag) = &sw.tag {
                collect_idents_from_stmt(tag, out);
            }
            for clause in &sw.block.body {
                for expr in &clause.list {
                    collect_idents_from_expr(expr, out);
                }
                for stmt in clause.body.as_ref() {
                    collect_idents_from_stmt(stmt, out);
                }
            }
        }
        Statement::IncDec(incdec) => {
            collect_idents_from_expr(&incdec.expr, out);
        }
        Statement::Send(send) => {
            collect_idents_from_expr(&send.chan, out);
            collect_idents_from_expr(&send.value, out);
        }
        Statement::Go(go) => {
            collect_idents_from_expr(&Expression::Call(go.call.clone()), out);
        }
        Statement::Defer(defer) => {
            collect_idents_from_expr(&Expression::Call(defer.call.clone()), out);
        }
        Statement::Select(sel) => {
            for clause in &sel.body.body {
                if let Some(comm) = &clause.comm {
                    collect_idents_from_stmt(comm, out);
                }
                for stmt in clause.body.as_ref() {
                    collect_idents_from_stmt(stmt, out);
                }
            }
        }
        Statement::Label(label) => {
            collect_idents_from_stmt(&label.stmt, out);
        }
        Statement::Empty(_) | Statement::Branch(_) => {}
    }
}

fn collect_idents_from_block(stmts: &[Statement], out: &mut Vec<String>) {
    for stmt in stmts {
        collect_idents_from_stmt(stmt, out);
    }
}

fn analyze_dependencies(graph: &mut InitGraph) {
    let decls: Vec<(NodeId, Declaration)> = graph
        .id_to_decl
        .iter()
        .map(|(id, d)| (*id, d.clone()))
        .collect();

    for (node_id, decl) in &decls {
        let mut refs = Vec::new();

        match decl {
            Declaration::Type(tspec) => {
                for spec in &tspec.specs {
                    collect_idents_from_expr(&spec.typ, &mut refs);
                }
            }
            Declaration::Const(c) => {
                for spec in &c.specs {
                    if let Some(t) = &spec.typ {
                        collect_idents_from_expr(t, &mut refs);
                    }
                    for value in &spec.values {
                        collect_idents_from_expr(value, &mut refs);
                    }
                }
            }
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    if let Some(t) = &spec.typ {
                        collect_idents_from_expr(t, &mut refs);
                    }
                    for value in &spec.values {
                        collect_idents_from_expr(value, &mut refs);
                    }
                }
            }
            Declaration::Function(f) => {
                if let Some(recv) = &f.recv {
                    collect_idents_from_field_list(recv, &mut refs);
                }
                collect_idents_from_field_list(&f.typ.params, &mut refs);
                collect_idents_from_field_list(&f.typ.result, &mut refs);
                if let Some(body) = &f.body {
                    collect_idents_from_block(&body.list, &mut refs);
                }
            }
        }

        let own_name = graph.nodes[*node_id as usize].name.clone();
        for ref_name in &refs {
            if *ref_name == own_name {
                continue;
            }
            if let Some(dep_id) = graph.lookup(ref_name) {
                graph.add_edge(*node_id, dep_id);
            }
        }
    }
}

// ============================================================
// Pass 3: Function elimination + topological sort
// ============================================================

#[derive(Eq, PartialEq)]
struct HeapEntry {
    ndeps: usize,
    is_const: bool,
    source_order: u32,
    id: NodeId,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: fewer deps first, constants before non-constants, lower source order first
        other
            .ndeps
            .cmp(&self.ndeps)
            .then_with(|| self.is_const.cmp(&other.is_const))
            .then_with(|| other.source_order.cmp(&self.source_order))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn eliminate_functions(graph: &mut InitGraph) {
    let func_ids: Vec<NodeId> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == DeclKind::Func)
        .map(|n| n.id)
        .collect();

    // Sort by cost ascending so cheap eliminations happen first
    let mut func_ids_sorted = func_ids.clone();
    func_ids_sorted.sort_by_key(|id| {
        let n = &graph.nodes[*id as usize];
        n.predecessors.len() * n.successors.len()
    });

    for func_id in func_ids_sorted {
        let preds: Vec<NodeId> = graph.nodes[func_id as usize]
            .predecessors
            .iter()
            .copied()
            .collect();
        let succs: Vec<NodeId> = graph.nodes[func_id as usize]
            .successors
            .iter()
            .copied()
            .collect();

        for &pred in &preds {
            if pred == func_id {
                continue;
            }
            for &succ in &succs {
                if succ == func_id {
                    continue;
                }
                graph.nodes[pred as usize].successors.insert(succ);
                graph.nodes[succ as usize].predecessors.insert(pred);
            }
            graph.nodes[pred as usize].successors.remove(&func_id);
        }

        for &succ in &succs {
            graph.nodes[succ as usize].predecessors.remove(&func_id);
        }

        graph.nodes[func_id as usize].predecessors.clear();
        graph.nodes[func_id as usize].successors.clear();
    }
}

fn find_cycle(graph: &InitGraph, start: NodeId) -> Option<Vec<String>> {
    let mut visited = HashSet::new();
    let mut path = Vec::new();

    fn dfs(
        graph: &InitGraph,
        current: NodeId,
        target: NodeId,
        visited: &mut HashSet<NodeId>,
        path: &mut Vec<NodeId>,
        first: bool,
    ) -> bool {
        if !first && current == target {
            return true;
        }
        if visited.contains(&current) {
            return false;
        }
        visited.insert(current);
        path.push(current);

        for &succ in &graph.nodes[current as usize].successors {
            if dfs(graph, succ, target, visited, path, false) {
                return true;
            }
        }

        path.pop();
        false
    }

    if dfs(graph, start, start, &mut visited, &mut path, true) {
        path.push(start);
        let names: Vec<String> = path
            .iter()
            .map(|id| graph.nodes[*id as usize].name.clone())
            .collect();
        Some(names)
    } else {
        None
    }
}

fn topological_sort(graph: &InitGraph) -> Result<Vec<NodeId>, String> {
    let non_func_ids: Vec<NodeId> = graph
        .nodes
        .iter()
        .filter(|n| n.kind != DeclKind::Func)
        .map(|n| n.id)
        .collect();

    let mut dep_count: HashMap<NodeId, usize> = HashMap::new();
    for &id in &non_func_ids {
        dep_count.insert(id, graph.nodes[id as usize].successors.len());
    }

    let mut heap = BinaryHeap::new();
    for &id in &non_func_ids {
        if dep_count[&id] == 0 {
            let node = &graph.nodes[id as usize];
            heap.push(HeapEntry {
                ndeps: 0,
                is_const: node.kind == DeclKind::Const,
                source_order: node.source_order,
                id,
            });
        }
    }

    let mut result = Vec::with_capacity(non_func_ids.len());

    while let Some(entry) = heap.pop() {
        result.push(entry.id);

        for &pred in &graph.nodes[entry.id as usize].predecessors {
            if graph.nodes[pred as usize].kind == DeclKind::Func {
                continue;
            }
            if let Some(count) = dep_count.get_mut(&pred) {
                *count -= 1;
                if *count == 0 {
                    let node = &graph.nodes[pred as usize];
                    heap.push(HeapEntry {
                        ndeps: 0,
                        is_const: node.kind == DeclKind::Const,
                        source_order: node.source_order,
                        id: pred,
                    });
                }
            }
        }
    }

    if result.len() != non_func_ids.len() {
        let processed: HashSet<NodeId> = result.iter().copied().collect();
        for &id in &non_func_ids {
            if !processed.contains(&id) {
                if let Some(cycle) = find_cycle(graph, id) {
                    return Err(format!(
                        "initialization cycle: {}",
                        cycle.join(" -> ")
                    ));
                }
            }
        }
        return Err("initialization cycle detected".to_string());
    }

    Ok(result)
}

// ============================================================
// Public API
// ============================================================

/// Computes the initialization order for all package-level declarations
/// across all files in a package. Returns declarations in the order they
/// should be compiled, with functions appended at the end.
pub fn compute_init_order(
    decls: &[Declaration],
) -> Result<Vec<Declaration>, String> {
    let mut graph = InitGraph::new();

    // Pass 1: register
    graph.register_declarations(decls);

    // Pass 2: analyze dependencies
    analyze_dependencies(&mut graph);

    // Pass 3a: eliminate function nodes from the var/const/type ordering
    eliminate_functions(&mut graph);

    // Pass 3b: topological sort of non-function nodes
    let sorted_ids = topological_sort(&graph)?;

    let mut result = Vec::with_capacity(decls.len());
    let mut emitted: HashSet<NodeId> = HashSet::new();

    for id in sorted_ids {
        if emitted.contains(&id) {
            continue;
        }
        emitted.insert(id);
        if let Some(decl) = graph.id_to_decl.get(&id) {
            result.push(decl.clone());
        }
    }

    // Append function declarations at the end (order among them doesn't matter
    // for initialization since Go allows mutual recursion).
    for node in &graph.nodes {
        if node.kind == DeclKind::Func && !emitted.contains(&node.id) {
            emitted.insert(node.id);
            if let Some(decl) = graph.id_to_decl.get(&node.id) {
                result.push(decl.clone());
            }
        }
    }

    Ok(result)
}

/// Computes the package dependency order from import relationships.
/// Returns packages in the order they should be initialized
/// (dependencies before dependents).
///
/// When a `ModuleResolver` is provided, import paths are resolved through
/// the module system (stripping the module prefix). Otherwise, import paths
/// are treated as relative to the parent directory (legacy fallback).
pub fn compute_package_order(
    project: Vec<Package>,
    resolver: Option<&ModuleResolver>,
) -> Result<(Vec<String>, HashMap<String, Package>), String> {
    let mut pkg_map: HashMap<String, Package> = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    let mut in_degree: HashMap<String, usize> = HashMap::new();

    for p in &project {
        let canon = p
            .path
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize {:?}: {}", p.path, e))?;
        let key = canon.to_str().ok_or("non-UTF8 path")?.to_string();

        if !pkg_map.contains_key(&key) {
            pkg_map.insert(key.clone(), p.clone());
        }
        adj.entry(key.clone()).or_default();
        in_degree.entry(key.clone()).or_insert(0);
    }

    for p in &project {
        let canon = p
            .path
            .canonicalize()
            .map_err(|e| format!("cannot canonicalize {:?}: {}", p.path, e))?;
        let pkg_key = canon.to_str().ok_or("non-UTF8 path")?.to_string();

        for file in &p.files {
            for import in &file.imports {
                let raw_import = import.path.value.trim_matches('"');

                let resolved_path = if let Some(resolver) = resolver {
                    resolver.resolve_import(raw_import).map_err(|e| {
                        format!("failed to resolve import '{}': {}", raw_import, e)
                    })?
                } else {
                    canon
                        .parent()
                        .ok_or("path has no parent")?
                        .join(raw_import)
                };

                let dep_canon = resolved_path.canonicalize().map_err(|e| {
                    format!(
                        "cannot canonicalize resolved import '{}' -> {:?}: {}",
                        raw_import, resolved_path, e
                    )
                })?;
                let dep_key = dep_canon
                    .to_str()
                    .ok_or("non-UTF8 import path")?
                    .to_string();

                if !adj.contains_key(&dep_key) {
                    return Err(format!(
                        "dependency '{}' (resolved to '{}') cannot be found among parsed packages",
                        raw_import, dep_key
                    ));
                }

                adj.entry(dep_key.clone())
                    .or_default()
                    .push(pkg_key.clone());
                *in_degree.entry(pkg_key.clone()).or_insert(0) += 1;
            }
        }
    }

    // Kahn's algorithm
    let mut queue: VecDeque<String> = VecDeque::new();
    for (key, &deg) in &in_degree {
        if deg == 0 {
            queue.push_back(key.clone());
        }
    }

    let mut order = Vec::with_capacity(pkg_map.len());

    while let Some(pkg) = queue.pop_front() {
        order.push(pkg.clone());

        if let Some(dependents) = adj.get(&pkg) {
            for dep in dependents {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
    }

    if order.len() != pkg_map.len() {
        return Err("cyclic package dependency detected".to_string());
    }

    Ok((order, pkg_map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::*;
    use crate::parser::token::{LitKind, Operator};
    use std::path::Path;
    use std::rc::Rc;

    fn make_var(name: &str, deps: Vec<&str>) -> Declaration {
        let value_exprs: Vec<Expression> = if deps.is_empty() {
            vec![Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "0".to_string(),
            })]
        } else {
            let mut expr = Expression::Ident(Ident {
                pos: 0,
                name: deps[0].to_string(),
            });
            for dep in &deps[1..] {
                expr = Expression::Operation(Operation {
                    pos: 0,
                    op: Operator::Add,
                    x: Box::new(expr),
                    y: Some(Box::new(Expression::Ident(Ident {
                        pos: 0,
                        name: dep.to_string(),
                    }))),
                });
            }
            vec![expr]
        };

        Declaration::Variable(Decl {
            docs: vec![],
            pos0: 0,
            pos1: None,
            specs: vec![VarSpec {
                docs: vec![],
                name: vec![Ident {
                    pos: 0,
                    name: name.to_string(),
                }],
                typ: Some(Expression::Ident(Ident {
                    pos: 0,
                    name: "int".to_string(),
                })),
                values: value_exprs,
            }],
        })
    }

    fn make_const(name: &str, deps: Vec<&str>) -> Declaration {
        let value_exprs: Vec<Expression> = if deps.is_empty() {
            vec![Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "1".to_string(),
            })]
        } else {
            vec![Expression::Ident(Ident {
                pos: 0,
                name: deps[0].to_string(),
            })]
        };

        Declaration::Const(Decl {
            docs: vec![],
            pos0: 0,
            pos1: None,
            specs: vec![ConstSpec {
                docs: vec![],
                name: vec![Ident {
                    pos: 0,
                    name: name.to_string(),
                }],
                typ: None,
                values: value_exprs,
            }],
        })
    }

    fn make_func(name: &str, body_refs: Vec<&str>) -> Declaration {
        let stmts: Vec<Statement> = body_refs
            .iter()
            .map(|r| {
                Statement::Expr(ExprStmt {
                    expr: Expression::Call(Call {
                        pos: (0, 0),
                        args: vec![],
                        func: Box::new(Expression::Ident(Ident {
                            pos: 0,
                            name: r.to_string(),
                        })),
                        dots: None,
                    }),
                })
            })
            .collect();

        Declaration::Function(FuncDecl {
            docs: vec![],
            recv: None,
            name: Ident {
                pos: 0,
                name: name.to_string(),
            },
            typ: FuncType {
                pos: 0,
                typ_params: Default::default(),
                params: Default::default(),
                result: Default::default(),
            },
            body: Some(BlockStmt {
                pos: (0, 0),
                list: stmts,
            }),
        })
    }

    fn get_name(decl: &Declaration) -> String {
        match decl {
            Declaration::Variable(v) => v.specs[0].name[0].name.clone(),
            Declaration::Const(c) => c.specs[0].name[0].name.clone(),
            Declaration::Function(f) => f.name.name.clone(),
            Declaration::Type(t) => t.specs[0].name.name.clone(),
        }
    }

    #[test]
    fn test_basic_ordering() {
        // a depends on c and b; b, c, d have no deps
        // b and c must come before a; d is independent
        let decls = vec![
            make_var("a", vec!["c", "b"]),
            make_var("b", vec![]),
            make_var("c", vec![]),
            make_var("d", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();
        let pos_c = names.iter().position(|n| n == "c").unwrap();

        assert!(pos_b < pos_a, "b should come before a");
        assert!(pos_c < pos_a, "c should come before a");
    }

    #[test]
    fn test_chain_ordering() {
        // c depends on b, b depends on a; a must come first, then b, then c
        let decls = vec![
            make_var("a", vec![]),
            make_var("b", vec!["a"]),
            make_var("c", vec!["b"]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();
        let pos_c = names.iter().position(|n| n == "c").unwrap();

        assert!(pos_a < pos_b, "a should come before b");
        assert!(pos_b < pos_c, "b should come before c");
    }

    #[test]
    fn test_constants_before_vars() {
        let decls = vec![
            make_var("v", vec![]),
            make_const("c", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_v = names.iter().position(|n| n == "v").unwrap();
        let pos_c = names.iter().position(|n| n == "c").unwrap();

        assert!(pos_c < pos_v, "constants should come before variables");
    }

    #[test]
    fn test_mutual_recursion_allowed() {
        // Two functions calling each other should NOT cause a cycle error
        let decls = vec![
            make_func("foo", vec!["bar"]),
            make_func("bar", vec!["foo"]),
            make_var("x", vec![]),
        ];

        let result = compute_init_order(&decls);
        assert!(result.is_ok(), "mutual recursion should be allowed");
    }

    #[test]
    fn test_cycle_detected() {
        // a depends on b, b depends on a -- cycle among vars is not allowed
        let decls = vec![
            make_var("a", vec!["b"]),
            make_var("b", vec!["a"]),
        ];

        let result = compute_init_order(&decls);
        assert!(result.is_err(), "should detect cycle");
        assert!(
            result.unwrap_err().contains("initialization cycle"),
            "error should mention cycle"
        );
    }

    #[test]
    fn test_func_deps_pass_through() {
        // var a depends on func f, func f depends on var b
        // After function elimination, a should depend on b
        let decls = vec![
            make_var("a", vec!["f"]),
            make_func("f", vec!["b"]),
            make_var("b", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();

        assert!(pos_b < pos_a, "b should come before a (through function f)");
    }

    #[test]
    fn test_cross_file_deps() {
        // Simulate declarations from two different files in the same package.
        // file1: var a = b + 1
        // file2: var b = 0
        // Since compute_init_order works on all declarations merged, b must come before a.
        let file1_decls = vec![make_var("a", vec!["b"])];
        let file2_decls = vec![make_var("b", vec![])];

        let mut all_decls = Vec::new();
        all_decls.extend(file1_decls);
        all_decls.extend(file2_decls);

        let ordered = compute_init_order(&all_decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();

        assert!(pos_b < pos_a, "b (from file2) should come before a (from file1)");
    }

    #[test]
    fn test_diamond_dependency() {
        //     d
        //    / \
        //   b   c
        //    \ /
        //     a
        // a depends on b and c; b and c depend on d; d has no deps
        let decls = vec![
            make_var("a", vec!["b", "c"]),
            make_var("b", vec!["d"]),
            make_var("c", vec!["d"]),
            make_var("d", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();
        let pos_c = names.iter().position(|n| n == "c").unwrap();
        let pos_d = names.iter().position(|n| n == "d").unwrap();

        assert!(pos_d < pos_b, "d before b");
        assert!(pos_d < pos_c, "d before c");
        assert!(pos_b < pos_a, "b before a");
        assert!(pos_c < pos_a, "c before a");
    }

    #[test]
    fn test_type_dependency() {
        // type T struct { ... }
        // var x depends on T (through type annotation or composite literal)
        let type_decl = Declaration::Type(Decl {
            docs: vec![],
            pos0: 0,
            pos1: None,
            specs: vec![TypeSpec {
                docs: vec![],
                alias: false,
                name: Ident { pos: 0, name: "T".to_string() },
                params: Default::default(),
                typ: Expression::TypeStruct(StructType {
                    pos: (0, 0),
                    fields: vec![],
                }),
            }],
        });

        // var x = T{}
        let var_decl = Declaration::Variable(Decl {
            docs: vec![],
            pos0: 0,
            pos1: None,
            specs: vec![VarSpec {
                docs: vec![],
                name: vec![Ident { pos: 0, name: "x".to_string() }],
                typ: None,
                values: vec![Expression::CompositeLit(CompositeLit {
                    typ: Box::new(Expression::Ident(Ident {
                        pos: 0,
                        name: "T".to_string(),
                    })),
                    val: LiteralValue {
                        pos: (0, 0),
                        values: vec![],
                    },
                })],
            }],
        });

        let decls = vec![var_decl, type_decl];
        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_t = names.iter().position(|n| n == "T").unwrap();
        let pos_x = names.iter().position(|n| n == "x").unwrap();

        assert!(pos_t < pos_x, "type T should come before var x");
    }

    #[test]
    fn test_three_node_cycle() {
        // a -> b -> c -> a: should detect cycle
        let decls = vec![
            make_var("a", vec!["b"]),
            make_var("b", vec!["c"]),
            make_var("c", vec!["a"]),
        ];

        let result = compute_init_order(&decls);
        assert!(result.is_err(), "should detect 3-node cycle");
        let err = result.unwrap_err();
        assert!(err.contains("initialization cycle"), "error: {}", err);
    }

    #[test]
    fn test_no_declarations() {
        let decls: Vec<Declaration> = vec![];
        let ordered = compute_init_order(&decls).unwrap();
        assert!(ordered.is_empty());
    }

    #[test]
    fn test_functions_appended_at_end() {
        let decls = vec![
            make_func("f", vec![]),
            make_var("x", vec![]),
            make_const("c", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_f = names.iter().position(|n| n == "f").unwrap();
        let pos_x = names.iter().position(|n| n == "x").unwrap();
        let pos_c = names.iter().position(|n| n == "c").unwrap();

        assert!(pos_c < pos_f, "const c before func f");
        assert!(pos_x < pos_f, "var x before func f");
    }

    #[test]
    fn test_const_depends_on_const() {
        // const b = 1; const a = b
        let decls = vec![
            make_const("a", vec!["b"]),
            make_const("b", vec![]),
        ];

        let ordered = compute_init_order(&decls).unwrap();
        let names: Vec<String> = ordered.iter().map(get_name).collect();

        let pos_a = names.iter().position(|n| n == "a").unwrap();
        let pos_b = names.iter().position(|n| n == "b").unwrap();

        assert!(pos_b < pos_a, "const b should come before const a");
    }

    // ============================================================
    // compute_package_order tests
    // ============================================================

    fn make_package(dir: &Path, name: &str, imports: Vec<&str>) -> Package {
        let import_specs: Vec<Import> = imports
            .iter()
            .map(|path| Import {
                name: None,
                path: StringLit {
                    pos: 0,
                    value: format!("\"{}\"", path),
                },
            })
            .collect();

        Package {
            path: dir.to_path_buf(),
            files: vec![File {
                path: None,
                line_info: vec![],
                docs: vec![],
                pkg_name: Ident {
                    pos: 0,
                    name: name.to_string(),
                },
                imports: import_specs,
                decl: vec![],
                comments: vec![],
            }],
        }
    }

    #[test]
    fn test_pkg_order_single_package_no_imports() {
        let dir = std::env::temp_dir().join("govm_test_pkg_order_single");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let pkg = make_package(&dir, "main", vec![]);
        let result = compute_package_order(vec![pkg], None);
        assert!(result.is_ok());
        let (order, _) = result.unwrap();
        assert_eq!(order.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_pkg_order_linear_dependency() {
        let base = std::env::temp_dir().join("govm_test_pkg_order_linear");
        let _ = std::fs::remove_dir_all(&base);
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let canon_a = dir_a.canonicalize().unwrap();
        let canon_b = dir_b.canonicalize().unwrap();

        let pkg_a = make_package(&dir_a, "a", vec![]);
        let import_path = canon_a.to_str().unwrap();
        let pkg_b = make_package(&dir_b, "b", vec![import_path]);

        let result = compute_package_order(vec![pkg_a, pkg_b], None);
        assert!(result.is_ok());
        let (order, _) = result.unwrap();
        assert_eq!(order.len(), 2);

        let pos_a = order.iter().position(|k| k == canon_a.to_str().unwrap()).unwrap();
        let pos_b = order.iter().position(|k| k == canon_b.to_str().unwrap()).unwrap();
        assert!(pos_a < pos_b, "package a should come before package b");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_pkg_order_diamond_dependency() {
        let base = std::env::temp_dir().join("govm_test_pkg_order_diamond");
        let _ = std::fs::remove_dir_all(&base);
        let dir_leaf = base.join("leaf");
        let dir_mid1 = base.join("mid1");
        let dir_mid2 = base.join("mid2");
        let dir_top = base.join("top");
        std::fs::create_dir_all(&dir_leaf).unwrap();
        std::fs::create_dir_all(&dir_mid1).unwrap();
        std::fs::create_dir_all(&dir_mid2).unwrap();
        std::fs::create_dir_all(&dir_top).unwrap();

        let canon_leaf = dir_leaf.canonicalize().unwrap();
        let canon_mid1 = dir_mid1.canonicalize().unwrap();
        let canon_mid2 = dir_mid2.canonicalize().unwrap();
        let canon_top = dir_top.canonicalize().unwrap();

        let leaf_path = canon_leaf.to_str().unwrap();
        let mid1_path = canon_mid1.to_str().unwrap();
        let mid2_path = canon_mid2.to_str().unwrap();

        let pkg_leaf = make_package(&dir_leaf, "leaf", vec![]);
        let pkg_mid1 = make_package(&dir_mid1, "mid1", vec![leaf_path]);
        let pkg_mid2 = make_package(&dir_mid2, "mid2", vec![leaf_path]);
        let pkg_top = make_package(&dir_top, "top", vec![mid1_path, mid2_path]);

        let result = compute_package_order(vec![pkg_leaf, pkg_mid1, pkg_mid2, pkg_top], None);
        assert!(result.is_ok());
        let (order, _) = result.unwrap();
        assert_eq!(order.len(), 4);

        let pos_leaf = order.iter().position(|k| k == canon_leaf.to_str().unwrap()).unwrap();
        let pos_mid1 = order.iter().position(|k| k == canon_mid1.to_str().unwrap()).unwrap();
        let pos_mid2 = order.iter().position(|k| k == canon_mid2.to_str().unwrap()).unwrap();
        let pos_top = order.iter().position(|k| k == canon_top.to_str().unwrap()).unwrap();

        assert!(pos_leaf < pos_mid1, "leaf before mid1");
        assert!(pos_leaf < pos_mid2, "leaf before mid2");
        assert!(pos_mid1 < pos_top, "mid1 before top");
        assert!(pos_mid2 < pos_top, "mid2 before top");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_pkg_order_cycle_detected() {
        let base = std::env::temp_dir().join("govm_test_pkg_order_cycle");
        let _ = std::fs::remove_dir_all(&base);
        let dir_a = base.join("a");
        let dir_b = base.join("b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let canon_a = dir_a.canonicalize().unwrap();
        let canon_b = dir_b.canonicalize().unwrap();

        let pkg_a = make_package(&dir_a, "a", vec![canon_b.to_str().unwrap()]);
        let pkg_b = make_package(&dir_b, "b", vec![canon_a.to_str().unwrap()]);

        let result = compute_package_order(vec![pkg_a, pkg_b], None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("cyclic"), "error should mention cycle: {}", err);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_pkg_order_with_module_resolver() {
        let base = std::env::temp_dir().join("govm_test_pkg_order_resolver");
        let _ = std::fs::remove_dir_all(&base);
        let dir_main = base.join("main_pkg");
        let dir_sub = base.join("main_pkg").join("sub");
        std::fs::create_dir_all(&dir_main).unwrap();
        std::fs::create_dir_all(&dir_sub).unwrap();

        std::fs::write(
            dir_main.join("go.mod"),
            "module example.com/testmod\n\ngo 1.20\n",
        )
        .unwrap();

        let resolver = ModuleResolver::from_project_root(&dir_main).unwrap();

        let pkg_sub = make_package(&dir_sub, "sub", vec![]);
        let pkg_main = make_package(&dir_main, "main", vec!["example.com/testmod/sub"]);

        let result = compute_package_order(vec![pkg_sub, pkg_main], Some(&resolver));
        assert!(result.is_ok());
        let (order, _) = result.unwrap();
        assert_eq!(order.len(), 2);

        let canon_main = dir_main.canonicalize().unwrap();
        let canon_sub = dir_sub.canonicalize().unwrap();

        let pos_main = order.iter().position(|k| k == canon_main.to_str().unwrap()).unwrap();
        let pos_sub = order.iter().position(|k| k == canon_sub.to_str().unwrap()).unwrap();
        assert!(pos_sub < pos_main, "sub should come before main");

        let _ = std::fs::remove_dir_all(&base);
    }
}
