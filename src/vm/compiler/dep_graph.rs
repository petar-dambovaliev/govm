use crate::parser::ast::{DeclStmt, Declaration, Expression, Package, Statement};
use crate::parser::token::LitKind;
use crate::vm::builtin;
use crate::vm::compiler::call::CallType;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::symbols::{ContextType, DefineType};
use ahash::{HashMap, HashMapExt};
use dep_graph::{DepGraph, Node};

pub fn make_package_dep_graph(
    project: Vec<Package>,
) -> (DepGraph<String>, HashMap<String, Package>) {
    let mut nodes: Vec<Node<String>> = project
        .iter()
        .map(|p| Node::new(p.path.canonicalize().unwrap().to_str().unwrap().to_string()))
        .collect();

    let mut map = HashMap::new();

    for p in &project {
        let c = p.path.canonicalize().unwrap();
        let key = c.to_str().unwrap();
        if !map.contains_key(key) {
            map.insert(key.to_string(), p.clone());
        }
        let pn = if let Some(pn) = nodes.iter().position(|a| a.id() == key) {
            pn
        } else {
            panic!();
        };
        for file in &p.files {
            for import in &file.imports {
                let import_path = c
                    .parent()
                    .unwrap()
                    .join(import.path.value.trim_matches('"'))
                    .to_str()
                    .unwrap()
                    .to_string();

                nodes[pn].add_dep(import_path.clone());
                if nodes.iter_mut().find(|n| n.id() == &import_path).is_none() {
                    panic!(
                        "dependency `{}` cannot be found. dependencies: {:#?}",
                        import_path, nodes
                    );
                }
            }
        }
    }

    let graph = DepGraph::new(&nodes);

    (graph, map)
}

//todo this only looks for identifiers
// it needs to check those are actually globals
// also implement all expressions in the analysis of functions

fn register_types(
    declrs: &[Declaration],
    nodes: &mut Vec<Node<(String, DefineType)>>,
    declrs_map: &mut HashMap<(String, DefineType), Declaration>,
    c: &mut Compiler,
) {
    for declr in declrs {
        match declr {
            Declaration::Type(tspec) => {
                for spec in &tspec.specs {
                    match &spec.typ {
                        //type spec
                        Expression::Ident(id) => {
                            let name = spec.name.name.clone();
                            let dt = DefineType::Spec {
                                name: name.clone(),
                                inner: Box::from(DefineType::Null),
                                methods: vec![],
                                is_transparent: spec.alias,
                            };

                            let key = (name.to_string(), dt.clone());
                            let node = Node::new(key.clone());
                            nodes.push(node);
                            declrs_map.insert(key.clone(), declr.clone());

                            let _ = c.symbols.define(&name, dt, false);
                        }
                        Expression::TypeInterface(_it) => {
                            if spec.alias {
                                unimplemented!("interface alias");
                            }
                            let dt = DefineType::Interface {
                                name: spec.name.name.clone(),
                                methods: vec![],
                            };
                            let key = (spec.name.name.to_string(), dt.clone());
                            let node = Node::new(key.clone());
                            nodes.push(node);
                            declrs_map.insert(key.clone(), declr.clone());

                            let _ = c.symbols.define(&spec.name.name.clone(), dt, false);
                        }
                        Expression::TypeStruct(_ta) => {
                            if spec.alias {
                                unimplemented!("struct alias");
                            }
                            let name = spec.name.name.as_str();

                            let dt = DefineType::Struct {
                                name: name.to_string(),
                                fields: vec![],
                                methods: vec![],
                            };

                            let key = (name.to_string(), dt.clone());
                            let node = Node::new(key.clone());
                            nodes.push(node);
                            declrs_map.insert(key.clone(), declr.clone());

                            let _ = c.symbols.define(name, dt, false);
                        }
                        _ => unimplemented!("{:#?}", spec),
                    }
                }
            }
            _ => {}
        }
    }
}

fn register_others(
    declrs: &[Declaration],
    nodes: &mut Vec<Node<(String, DefineType)>>,
    declrs_map: &mut HashMap<(String, DefineType), Declaration>,
    c: &mut Compiler,
) {
    for declr in declrs {
        match declr {
            Declaration::Const(v) => {
                for spec in &v.specs {
                    for name in &spec.name {
                        let dt = DefineType::Const(Box::new(DefineType::Null));
                        let key = (name.name.to_string(), dt.clone());
                        let node = Node::new(key.clone());
                        nodes.push(node);
                        declrs_map.insert(key.clone(), declr.clone());

                        let _ = c.symbols.define(name.name.as_str(), dt.clone(), false);
                    }
                }
            }
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    for name in &spec.name {
                        let dt = DefineType::Var(Box::new(DefineType::Null));
                        let key = (name.name.to_string(), dt.clone());
                        let node = Node::new(key.clone());
                        nodes.push(node);

                        declrs_map.insert(key.clone(), declr.clone());

                        let _ = c.symbols.define(name.name.as_str(), dt.clone(), false);
                    }
                }
            }
            Declaration::Function(f) => {
                let (f_name, _recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
                    let recv = recv.list.first().unwrap();
                    let t = c.expression_to_define_type(&recv.typ).unwrap();

                    let ret = (
                        Compiler::make_method_name(t.strip_ref(), &f.name.name),
                        Some(recv),
                        Some(Box::new(t)),
                    );

                    ret
                } else {
                    (f.name.name.clone(), None, None)
                };

                let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

                for p in &f.typ.params.list {
                    let t = c.expression_to_define_type(&p.typ).unwrap();
                    for name in &p.name {
                        decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));
                    }
                }

                let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());

                for el in &f.typ.result.list {
                    let t = c.expression_to_define_type(&el.typ).unwrap();
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
                    recv: recv_t.clone(),
                    args: decl_arg_types,
                    rt: Box::new(r_t),
                };

                let _ = c.symbols.define(&f_name, func_def.clone(), false);

                //add method to struct symbol
                if let Some(recv) = recv_t {
                    let tt = c.symbols.resolve(&recv.get_type_name()).unwrap().get_type();

                    if tt.is_struct() {
                        let (r_name, r_fields, mut r_methods) = tt.as_struct().unwrap();

                        r_methods.push(func_def.clone());
                        let updated = c.symbols.update_dt(
                            &r_name,
                            DefineType::Struct {
                                name: r_name.to_string(),
                                fields: r_fields.clone(),
                                methods: r_methods.clone(),
                            },
                        );
                        assert!(updated);
                    } else if tt.is_spec() {
                        let (name, inner, mut methods, is_transparent) = tt.as_spec().unwrap();

                        methods.push(func_def.clone());
                        let updated = c.symbols.update_dt(
                            &name,
                            DefineType::Spec {
                                name: name.to_string(),
                                inner: Box::new(inner.clone()),
                                methods: methods.clone(),
                                is_transparent,
                            },
                        );
                        assert!(updated);
                    } else {
                        unimplemented!("{:#?}", tt);
                    }
                }

                let key = (f_name, func_def.clone());
                let node = Node::new(key.clone());
                nodes.push(node);

                declrs_map.insert(key.clone(), declr.clone());
            }
            _ => {}
        }
    }
}

pub fn make_init_dep_graph(
    declrs: &[Declaration],
    c: &mut Compiler,
) -> (
    DepGraph<(String, DefineType)>,
    HashMap<(String, DefineType), Declaration>,
) {
    let mut nodes = vec![];
    let mut declrs_map = HashMap::new();

    register_types(declrs, &mut nodes, &mut declrs_map, c);
    register_others(declrs, &mut nodes, &mut declrs_map, c);

    for declr in declrs {
        match declr {
            Declaration::Const(v) => {
                for spec in &v.specs {
                    for (name, expr) in spec.name.iter().zip(&spec.values) {
                        let pos = nodes
                            .iter()
                            .position(|n| {
                                n.id()
                                    == &(
                                        name.name.clone(),
                                        DefineType::Const(Box::new(DefineType::Null)),
                                    )
                            })
                            .unwrap();

                        let deps = get_const_idents_from_expr(expr, c).unwrap();

                        for dep_id in deps {
                            nodes[pos].add_dep(dep_id);
                        }
                    }
                }
            }
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    for (name, expr) in spec.name.iter().zip(&spec.values) {
                        let pos = nodes
                            .iter()
                            .position(|n| {
                                n.id()
                                    == &(
                                        name.name.clone(),
                                        DefineType::Var(Box::new(DefineType::Null)),
                                    )
                            })
                            .unwrap();

                        let deps = get_const_idents_from_expr(expr, c).unwrap();

                        for dep_id in deps {
                            nodes[pos].add_dep(dep_id);
                        }
                    }
                }
            }
            Declaration::Function(f) => {
                let f_name = match &f.recv {
                    Some(recv) => {
                        let t = recv.list.first().unwrap();
                        let id = t.typ.as_ident().unwrap();
                        let r = c.symbols.resolve(&id.name).unwrap();
                        let dt = r.get_type();
                        CallType::make_method_name(dt, &f.name.name)
                    }
                    None => f.name.name.clone(),
                };

                let pos = nodes
                    .iter()
                    .position(|n| {
                        let id = n.id();
                        id.0 == f_name.clone() && id.1.is_func()
                    })
                    .expect(&f.name.name);

                let mut idents = vec![];

                if let Some(recv) = &f.recv {
                    for r in &recv.list {
                        idents.append(&mut get_const_idents_from_expr(&r.typ, c).unwrap());
                    }
                }

                for stmt in &f.body.as_ref().unwrap().list {
                    idents.append(&mut get_const_idents_from_stmt(stmt, c).unwrap());
                }

                for res in &f.typ.result.list {
                    idents.append(&mut get_const_idents_from_expr(&res.typ, c).unwrap());
                }

                for arg in &f.typ.params.list {
                    idents.append(&mut get_const_idents_from_expr(&arg.typ, c).unwrap());
                }

                for (name, dt) in idents {
                    if let Some(dep) = nodes.iter().find(|n| n.id().0 == name).cloned() {
                        nodes[pos].add_dep(dep.id().clone());
                    }
                }
            }
            Declaration::Type(tspec) => {
                for spec in &tspec.specs {
                    match &spec.typ {
                        // type spec
                        Expression::Ident(_id) => {
                            let pos = nodes
                                .iter()
                                .position(|n| {
                                    let id = n.id();
                                    id.0 == spec.name.name.clone() && id.1.is_spec()
                                })
                                .unwrap();

                            let idents = get_const_idents_from_expr(&spec.typ, c).unwrap();

                            for (name, dt) in idents {
                                if let Some(dep) = nodes.iter().find(|n| n.id().0 == name).cloned()
                                {
                                    nodes[pos].add_dep(dep.id().clone());
                                }
                            }
                        }
                        Expression::TypeInterface(it) => {
                            let pos = nodes
                                .iter()
                                .position(|n| {
                                    let id = n.id();
                                    id.0 == spec.name.name.clone() && id.1.is_interface()
                                })
                                .unwrap();

                            let mut idents = vec![];

                            for method in &it.methods.list {
                                idents.append(
                                    &mut get_const_idents_from_expr(&method.typ, c).unwrap(),
                                );
                            }

                            for (name, dt) in idents {
                                if let Some(dep) = nodes.iter().find(|n| n.id().0 == name).cloned()
                                {
                                    nodes[pos].add_dep(dep.id().clone());
                                }
                            }
                        }
                        Expression::TypeStruct(ta) => {
                            let pos = nodes
                                .iter()
                                .position(|n| {
                                    let id = n.id();
                                    id.0 == spec.name.name.clone() && id.1.is_struct()
                                })
                                .unwrap();

                            let mut idents = vec![];

                            for field in &ta.fields {
                                idents.append(
                                    &mut get_const_idents_from_expr(&field.typ, c).unwrap(),
                                );
                            }

                            for (name, dt) in idents {
                                if let Some(dep) = nodes.iter().find(|n| n.id().0 == name).cloned()
                                {
                                    nodes[pos].add_dep(dep.id().clone());
                                }
                            }
                        }
                        _ => unimplemented!("{:#?}", spec),
                    }
                }
            }

            _ => {}
        }
    }

    let graph = DepGraph::new(&nodes);
    (graph, declrs_map)
}

fn get_const_idents_from_stmt(
    stmt: &Statement,
    c: &mut Compiler,
) -> Result<Vec<(String, DefineType)>, String> {
    let mut idents = vec![];
    match stmt {
        Statement::Expr(expr) => {
            idents.append(&mut get_const_idents_from_expr(&expr.expr, c)?);
        }
        Statement::Return(ret) => {
            for r in &ret.ret {
                idents.append(&mut get_const_idents_from_expr(r, c)?);
            }
        }
        Statement::If(ifstmt) => {
            for s in &ifstmt.body.list {
                idents.append(&mut get_const_idents_from_stmt(s, c)?);
            }
            if let Some(init) = &ifstmt.init {
                idents.append(&mut get_const_idents_from_stmt(init.as_ref(), c)?);
            }
            idents.append(&mut get_const_idents_from_expr(&ifstmt.cond, c)?);
            if let Some(els) = &ifstmt.else_ {
                idents.append(&mut get_const_idents_from_stmt(els.as_ref(), c)?);
            }
        }
        Statement::Declaration(declr) => match declr {
            DeclStmt::Variable(var) => {
                for spec in &var.specs {
                    if let Some(t) = &spec.typ {
                        idents.append(&mut get_const_idents_from_expr(t, c)?);
                    }

                    for value in &spec.values {
                        idents.append(&mut get_const_idents_from_expr(value, c)?);
                    }
                }
            }
            DeclStmt::Const(cnst) => {
                for spec in &cnst.specs {
                    if let Some(t) = &spec.typ {
                        idents.append(&mut get_const_idents_from_expr(t, c)?);
                    }
                    for value in &spec.values {
                        idents.append(&mut get_const_idents_from_expr(value, c)?);
                    }
                }
            }
            DeclStmt::Type(t) => {
                for spec in &t.specs {
                    idents.append(&mut get_const_idents_from_expr(&spec.typ, c)?);
                }
            }
        },
        t => println!("get_const_idents_from_stmt: not implemented {:#?}", t),
    }
    Ok(idents)
}

fn get_const_idents_from_expr(
    expr: &Expression,
    c: &mut Compiler,
) -> Result<Vec<(String, DefineType)>, String> {
    let mut idents = vec![];
    match expr {
        Expression::Ident(id) => {
            if id.name != "iota" && builtin::resolve(id.name.as_str()).is_none() {
                if let Some(r) = c.symbols.resolve(id.name.as_str()) {
                    let dt = r.get_type();
                    idents.push((id.name.clone(), dt));
                }
            }
        }
        Expression::BasicLit(bl) => {
            if bl.kind == LitKind::Ident {
                let dt = c.symbols.resolve(bl.value.as_str()).unwrap().get_type();
                idents.push((bl.value.clone(), dt));
            }
        }
        Expression::Operation(op) => {
            idents.append(&mut get_const_idents_from_expr(
                &mut op.x.as_ref().clone(),
                c,
            )?);

            if let Some(s) = &op.y {
                idents.append(&mut get_const_idents_from_expr(&mut s.as_ref().clone(), c)?);
            }
        }
        // Expression::Call(call) => {
        //     idents.append(&mut get_const_idents_from_expr(&call.func, c)?);
        //     for arg in &call.args {
        //         idents.append(&mut get_const_idents_from_expr(&arg, c)?);
        //     }
        // }
        Expression::CompositeLit(clit) => {
            idents.append(&mut get_const_idents_from_expr(clit.typ.as_ref(), c)?);
        }
        // Expression::Selector(sel) => {
        //     let dt = c.expression_to_define_type(sel.x.as_ref());
        //     panic!("{:#?} {:#?}", dt, sel);
        // }
        t => println!("get_const_idents_from_expr: not implemented {:#?}", t),
    }
    Ok(idents)
}
