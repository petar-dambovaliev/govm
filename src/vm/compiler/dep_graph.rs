use crate::parser::ast::{Decl, DeclStmt, Declaration, Expression, FuncDecl, Statement, VarSpec};
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::vm::builtin;
use crate::vm::compiler::call::CallType;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::declaration::compile_variable;
use crate::vm::compiler::{Context, FuncContext, LoopContext};
use crate::vm::symbols::{
    is_integer_coerceable_to, is_uint_coerceable_to, ContextType, DefineType, Resolved, Scope,
};
use ahash::{HashMap, HashMapExt};
use dep_graph::{DepGraph, Node};

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
                    if !spec.alias {
                        match &spec.typ {
                            //type spec
                            Expression::Ident(id) => {
                                let name = spec.name.name.clone();
                                let dt = DefineType::Spec {
                                    name: name.clone(),
                                    inner: Box::from(DefineType::Null),
                                    methods: vec![],
                                    is_transparent: false,
                                };

                                let key = (name.to_string(), dt.clone());
                                let node = Node::new(key.clone());
                                nodes.push(node);
                                declrs_map.insert(key.clone(), declr.clone());

                                let _ = c.symbols.define(&name, dt, false);
                            }
                            Expression::TypeInterface(_it) => {
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
                    } else {
                        unimplemented!("type aliases");
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

pub fn make_dep_graph(
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
                let pos = nodes
                    .iter()
                    .position(|n| {
                        let id = n.id();
                        id.0 == f.name.name.clone() && id.1.is_func()
                    })
                    .unwrap();

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
                    if !spec.alias {
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
                                    if let Some(dep) =
                                        nodes.iter().find(|n| n.id().0 == name).cloned()
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
                                    if let Some(dep) =
                                        nodes.iter().find(|n| n.id().0 == name).cloned()
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
                                    if let Some(dep) =
                                        nodes.iter().find(|n| n.id().0 == name).cloned()
                                    {
                                        nodes[pos].add_dep(dep.id().clone());
                                    }
                                }
                            }
                            _ => unimplemented!("{:#?}", spec),
                        }
                    } else {
                        unimplemented!("type aliases");
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

                compile_variable(var, c).unwrap();
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
        Expression::Call(call) => {
            idents.append(&mut get_const_idents_from_expr(&call.func, c)?);
            for arg in &call.args {
                idents.append(&mut get_const_idents_from_expr(&arg, c)?);
            }
        }
        Expression::CompositeLit(clit) => {
            idents.append(&mut get_const_idents_from_expr(clit.typ.as_ref(), c)?);
        }
        Expression::Selector(sel) => {
            let dt = c.expression_to_define_type(sel.x.as_ref());
            panic!("{:#?} {:#?}", dt, sel);
        }
        t => println!("get_const_idents_from_expr: not implemented {:#?}", t),
    }
    Ok(idents)
}

pub fn idents_from_function_body(f: &FuncDecl, c: &mut Compiler) -> Vec<String> {
    let pos_jump = c.instructions.len();

    c.func_contexts.push(FuncContext::new(pos_jump));

    let (f_name, recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
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

    let _ = match c.symbols.resolve(&f_name) {
        Some(s) => {
            let updated = c.symbols.update_dt(
                &f_name,
                DefineType::Func {
                    name: f.name.name.clone(),
                    recv: recv_t.clone(),
                    args: vec![],
                    rt: Box::new(DefineType::Null),
                },
            );

            assert!(updated, "function not updated: {}", f_name);
            s.get_symbol()
        }
        None => c.symbols.define(
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

    // Compile function in a new scope
    c.symbols.new_context(false);

    if let Some(recv) = recv {
        let t = c.expression_to_define_type(&recv.typ).unwrap();

        //check if there is a field with the same name
        if let DefineType::Struct { fields, .. } = &t.strip_ref() {
            for field in fields {
                let field_name = match field {
                    ContextType::Named(s, _) => s.clone(),
                    ContextType::Embedded(s, _) => s.clone(),
                    _ => unimplemented!(),
                };
                if field_name == f.name.name {
                    panic!("field and method with the same name {}", field_name);
                }
            }
        }

        c.symbols.define(
            &recv.name.first().unwrap().name,
            DefineType::Var(Box::new(t.clone())),
            t.is_invar(),
        );
    }

    let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

    for p in &f.typ.params.list {
        let t = c.expression_to_define_type(&p.typ).unwrap();
        for name in &p.name {
            decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

            c.symbols.define(
                &name.name,
                DefineType::Var(Box::new(t.clone())),
                t.is_invar(),
            );
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
        recv: recv_t,
        args: decl_arg_types,
        rt: Box::new(r_t.clone()),
    };
    let updated = c.symbols.update_dt(&f_name, func_def.clone());
    assert!(updated);

    c.func_contexts.last_mut().unwrap().expected_ret = r_t;

    //add method to struct symbol
    if let Some(recv) = recv {
        let t = c.expression_to_define_type(&recv.typ).unwrap();
        let tt = c.symbols.resolve(&t.get_type_name()).unwrap().get_type();

        if tt.is_struct() {
            let (r_name, r_fields, mut r_methods) = tt.as_struct().unwrap();

            r_methods.push(func_def.clone());
            let updated = c.symbols.update_dt(
                &r_name,
                DefineType::Struct {
                    name: r_name.to_string(),
                    fields: r_fields,
                    methods: r_methods,
                },
            );
            assert!(updated);
        } else if tt.is_spec() {
            let (name, inner, mut methods, is_transparent) = tt.as_spec().unwrap();
            assert!(!is_transparent);

            methods.push(func_def.clone());

            let updated = c.symbols.update_dt(
                &name,
                DefineType::Spec {
                    name: name.to_string(),
                    inner: Box::new(inner),
                    methods,
                    is_transparent,
                },
            );
            assert!(updated);
        }
    }

    let mut idents = vec![];
    if let Some(body) = &f.body {
        idents = idents_from_block(&body.list, c);
    }

    idents
}

pub fn idents_from_variable(
    v: &Decl<VarSpec>,
    c: &mut Compiler,
) -> Vec<(Vec<(String, DefineType)>, DefineType)> {
    let mut res = vec![];

    for spec in &v.specs {
        let declared_tp = c
            .expression_to_define_type(
                spec.typ
                    .as_ref()
                    .expect("no declared values requires a declared type"),
            )
            .unwrap();

        let mut value_is_default = false;
        let values = if spec.values.is_empty() {
            let mut defaults = Vec::with_capacity(spec.name.len());
            for _ in 0..spec.name.len() {
                defaults.push(c.make_type_default_val(declared_tp.clone()));
            }
            value_is_default = true;
            defaults
        } else {
            spec.values.clone()
        };

        for (name, value) in spec.name.iter().zip(values.iter()) {
            let (mut idents, mut rt) = ident_expression(value, c);

            if let Some(ss) = rt.udt_ident() {
                idents.push((ss, rt.clone()));
            }

            if let Some(ss) = declared_tp.udt_ident() {
                idents.push((ss, declared_tp.clone()));
            }

            if rt.is_nil()
                && (declared_tp.is_ref() || declared_tp.is_func() || declared_tp.is_slice())
            {
                rt = declared_tp.clone();
            }

            let mut should_upcast = false;
            let mut should_cast_alias = false;

            if rt != declared_tp {
                if value.is_int_lit() {
                    let i = value.as_int_lit().unwrap();
                    if i >= u8::MIN as isize && i <= u8::MAX as isize {
                        rt = declared_tp.clone();
                    }
                } else if declared_tp.is_interface() {
                    rt = declared_tp.clone();
                    should_upcast = true;
                } else if declared_tp.is_spec() {
                    let spec = declared_tp.as_spec().unwrap();

                    if spec.1 == rt {
                        rt = declared_tp.clone();
                        should_cast_alias = true;
                    }
                }
            }

            if c.symbols.current_context().scope == Scope::Global {
                let updated = c
                    .symbols
                    .update_dt(name.name.as_str(), DefineType::Var(Box::new(rt.clone())));

                assert!(updated);

                c.symbols.resolve(name.name.as_str()).unwrap().get_symbol()
            } else {
                c.symbols.define(
                    name.name.as_str(),
                    DefineType::Var(Box::new(rt.clone())),
                    rt.is_invar(),
                )
            };

            if should_upcast {
                let (name, _) = declared_tp.as_interface();
                let (s, _) = c.symbols.resolve(&name).unwrap().as_local();
            } else if should_cast_alias {
                let (name, _, _, _) = declared_tp.as_spec().unwrap();
                let (s, _) = c.symbols.resolve(&name).unwrap().as_local();
            }

            res.push((idents, rt));
        }
    }
    res
}

pub(crate) fn idents_from_block(block: &[Statement], c: &mut Compiler) -> Vec<String> {
    let mut idents = vec![];
    if block.is_empty() {
        return idents;
    }

    c.symbols.enter_scope();

    for s in block {
        idents.append(&mut idents_from_statement(s, c));
    }

    c.symbols.leave_scope();

    idents
}

fn idents_from_statement(stmt: &Statement, c: &mut Compiler) -> Vec<String> {
    let mut idents = vec![];

    match stmt {
        Statement::For(forstmt) => {
            c.symbols.enter_scope();
            let label = c.label_contexts.get(&(forstmt.pos, 0)).cloned();

            if let Some(init) = &forstmt.init {
                idents.append(&mut idents_from_statement(init.as_ref(), c));
            }

            c.contexts
                .push(Context::For(LoopContext::new(c.instructions.len(), label)));

            if let Some(cond) = &forstmt.cond {
                idents.append(&mut idents_from_statement(cond.as_ref(), c));
            }

            idents.append(&mut idents_from_block(&forstmt.body.list, c));

            if let Some(post) = &forstmt.post {
                idents.append(&mut idents_from_statement(post.as_ref(), c));
            }

            c.symbols.leave_scope();
        }
        Statement::If(ifstmt) => {
            if let Some(init) = &ifstmt.init {
                idents.append(&mut idents_from_statement(init.as_ref(), c));
            }

            idents.append(
                &mut ident_expression(&ifstmt.cond, c)
                    .0
                    .iter()
                    .map(|a| a.0)
                    .collect(),
            );
            idents.append(&mut idents_from_block(&ifstmt.body.list, c));

            if let Some(alternative) = &ifstmt.else_ {
                match alternative.as_ref() {
                    Statement::Block(bl) => {
                        idents.append(&mut idents_from_block(&bl.list, c));
                    }
                    Statement::If(_elseif) => {
                        idents.append(&mut idents_from_statement(alternative.as_ref(), c));
                    }
                    _ => panic!("else should be a block: {:#?}", alternative),
                }
            }
        }
        Statement::Assign(assign) => {
            if assign.left.len() > 1 && assign.right.len() == 1 {
                let first = assign.right.first().unwrap();
                let (ids, dt) = ident_expression(first, c);

                let (ret, is_type_assert) = match dt {
                    DefineType::Func { rt: ret, .. } => (ret, Some(false)),
                    DefineType::Tuple(_) => {
                        if let Expression::Index(_) = first {
                            (Box::new(dt.clone()), None)
                        } else {
                            (Box::new(dt.clone()), Some(true))
                        }
                    }
                    _ => panic!("expected a func: got {:#?}", dt),
                };

                let tuple = ret.as_tuple();
                assert_eq!(assign.left.len(), tuple.len());
                let mut i = 0;

                for (left, ct) in assign.left.iter().zip(tuple).rev() {
                    match &assign.op {
                        Operator::Define => {
                            let name = match left {
                                Expression::Ident(ident) => &ident.name,
                                _ => panic!("only identifiers can be defined: {:#?}", left),
                            };

                            let symbol = c.symbols.define(
                                name.as_str(),
                                DefineType::Var(Box::new(ct.clone())),
                                ct.is_invar(),
                            );
                        }
                        Operator::Assign => 'assign: {
                            let name = match &left {
                                Expression::Ident(name) => name.name.as_str(),
                                _ => panic!(),
                            };

                            //todo emit pop for the right expr
                            if name == "_" {
                                break 'assign;
                            }

                            let resolved = c
                                .symbols
                                .resolve(name)
                                .expect(&format!("assign: `{name}` is not defined"));
                        }
                        _ => unimplemented!(),
                    }
                    i += 1;
                }
                return ids.iter().map(|a| a.0).collect();
            }

            assert_eq!(assign.left.len(), assign.right.len());

            for (left, right) in assign.left.iter().zip(assign.right.iter()) {
                match &assign.op {
                    Operator::AddAssign => {
                        idents
                            .append(&mut ident_expression(left, c).0.iter().map(|a| a.0).collect());
                        idents.append(
                            &mut ident_expression(right, c).0.iter().map(|a| a.0).collect(),
                        );
                    }
                    Operator::Define => {
                        let name = match left {
                            Expression::Ident(ident) => &ident.name,
                            _ => panic!("only identifiers can be defined: {:#?}", left),
                        };

                        let (ids, mut rt) = ident_expression(right, c);

                        idents.append(&mut ids.iter().map(|a| a.0).collect());

                        rt = match rt {
                            DefineType::Tuple(tuple) => tuple[0].clone(),
                            _ => rt,
                        };

                        let symbol = c.symbols.define(
                            name.as_str(),
                            DefineType::Var(Box::new(rt.clone())),
                            rt.is_invar(),
                        );
                    }
                    Operator::Assign => 'assign: {
                        let (name, sel, is_deref) = match &left {
                            Expression::Ident(name) => (name.name.to_string(), None, false),
                            Expression::Selector(sl) => (
                                sl.x.as_ident().unwrap().name.clone(),
                                Some(sl.sel.name.to_string()),
                                false,
                            ),
                            Expression::Index(ind) => {
                                let (ids1, _) = ident_expression(ind.left.as_ref(), c);
                                let (ids2, _) = ident_expression(ind.index.as_ref(), c);
                                let (ids3, _) = ident_expression(right, c);

                                idents.append(&mut ids1.iter().map(|a| a.0).collect());
                                idents.append(&mut ids2.iter().map(|a| a.0).collect());
                                idents.append(&mut ids3.iter().map(|a| a.0).collect());

                                return idents;
                            }
                            Expression::Operation(op) => {
                                //deref
                                if op.y.is_none() && op.op == Operator::Star {
                                    let ident = op.x.as_ident().unwrap();
                                    (ident.name.to_string(), None, true)
                                } else {
                                    panic!("cannot assign a value to expressions of type");
                                }
                            }
                            _ => panic!(),
                        };

                        //todo pop expression if its not assigned to anything
                        if name == "_" {
                            break 'assign;
                        }

                        let resolved = c
                            .symbols
                            .resolve(&name)
                            .expect(&format!("assign: `{name}` is not defined"));

                        if let Some(s) = sel {
                            let t = resolved.get_type().strip_var().strip_ref();

                            match t {
                                DefineType::Struct { fields, .. } => {
                                    let mut i = None;
                                    for (ind, field) in fields.iter().enumerate() {
                                        let f = field.as_named().unwrap();
                                        if f.0 == s {
                                            i = Some(ind);
                                        }
                                    }

                                    let (ids3, _) = ident_expression(right, c);

                                    idents.append(&mut ids3.iter().map(|a| a.0).collect());
                                    return idents;
                                }
                                _ => unimplemented!("{:#?}", t),
                            }
                        }

                        let (ids3, got_t) = ident_expression(right, c);

                        idents.append(&mut ids3.iter().map(|a| a.0).collect());

                        let expect_t = match resolved {
                            Resolved::Enclosed((s, t)) => t.strip_var(),
                            Resolved::Local((symbol, t)) => t.strip_var(),
                        };

                        if !is_deref {
                            let stripped = expect_t.strip_type();

                            if right.is_int_lit() {
                                let is_value_coercable = if let Ok(i) = right.as_int_lit() {
                                    is_integer_coerceable_to(i, &stripped)
                                } else if let Ok(i) = right.as_uint_lit() {
                                    is_uint_coerceable_to(i, &stripped)
                                } else {
                                    false
                                };

                                if !(got_t.is_coerceable_to(&stripped) && is_value_coercable) {
                                    assert_eq!(
                                        expect_t.strip_type(),
                                        got_t,
                                        "left:{:#?}---right:{:#?}",
                                        left,
                                        right
                                    );
                                }
                            }
                        }
                    }
                    _ => unimplemented!(),
                }
            }
            return idents;
        }
        Statement::Expr(expr) => {
            let (ids3, got_t) = ident_expression(&expr.expr, c);
            idents.append(&mut ids3.iter().map(|a| a.0).collect());
        }
        Statement::Block(stmts) => {
            let mut ids3 = idents_from_block(&stmts.list, c);
            idents.append(&mut ids3);
            return idents;
        }
        Statement::Declaration(declr) => match declr {
            DeclStmt::Type(t) => {
                for spec in &t.specs {
                    for param in &spec.params.list {
                        if let Some(dt) = c.expression_to_define_type(&param.typ) {
                            // if its not defined in the current context
                            // add it as a dependency
                            if let Some(id) = dt.udt_ident() {
                                if c.symbols.current_context().resolve(&id).is_none() {
                                    idents.push(id);
                                }
                            }
                        }
                    }
                }
            }
            DeclStmt::Const(t) => {}
            DeclStmt::Variable(t) => {
                let res = idents_from_variable(t, c);

                for re in res {
                    for r in re.0 {
                        if c.symbols.current_context().resolve(&r.0).is_none() {
                            idents.push(r.0);
                        }
                    }
                }
            }
        },
        Statement::Return(expr) => {
            let mut rts = Vec::with_capacity(expr.ret.len());

            for r in &expr.ret {
                let (mut ids, t) = ident_expression(&r, c);
                idents.append(&mut ids.iter().map(|a| a.0).collect());
                rts.push(t);
            }

            let mut rts_len = rts.len();
            let mut rt = if rts_len == 0 {
                DefineType::Null
            } else if rts_len == 1 {
                rts[0].clone()
            } else {
                DefineType::Tuple(rts)
            };

            if rts_len == 1 && rt.is_tuple() {
                let tuple = rt.as_tuple();
                rts_len = tuple.len();
                rt = DefineType::Tuple(tuple);
            }

            let expect_t = c.func_contexts.last().unwrap().expected_ret.clone();

            let is_type_assert = if expr.ret.len() == 1 {
                if let Expression::TypeAssert(_) = &expr.ret[0] {
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if !expect_t.is_tuple() && is_type_assert {
                rts_len = 1;
            }

            c.func_contexts
                .last_mut()
                .unwrap()
                .ret_types
                .push((rt, is_type_assert));

            assert!(rts_len < u16::MAX as usize);

            return idents;
        }
        Statement::Branch(branch) => {}
        Statement::IncDec(incdec) => {
            let name = match &incdec.expr {
                Expression::Ident(ident) => ident.clone(),
                _ => panic!("only ident allowed inc/dec"),
            };

            let r = c.symbols.resolve(&name.name).unwrap();

            let t = match r {
                Resolved::Enclosed((s, t)) => t,
                Resolved::Local((symbol, t)) => match symbol.scope {
                    Scope::Local => t,
                    Scope::Global => t,
                },
            };

            if !t.strip_var().is_numeric() {
                panic!("cannot use inc/dec operators on {:#?}", t);
            }

            if c.symbols.current_context().resolve(&name.name).is_none() {
                idents.push(name.name.clone());
            }
        }
        Statement::Empty(_) => {}
        Statement::Range(rng) => {
            let (ids, rt) = ident_expression(&rng.expr, c);
            idents.append(&mut ids.iter().map(|a| a.0).collect());

            if let Some(k) = &rng.key {
                let (ids, rt) = ident_expression(&k, c);
                idents.append(&mut ids.iter().map(|a| a.0).collect());

                match k {
                    Expression::Ident(key_id) => {
                        let key_symbol = c.symbols.define(
                            key_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
                            false,
                        );
                    }
                    _ => panic!("invalid"),
                }
            }

            if let Some(v) = &rng.value {
                let (ids, rt) = ident_expression(&v, c);
                idents.append(&mut ids.iter().map(|a| a.0).collect());

                match v {
                    Expression::Ident(value_id) => {
                        let value_symbol = c.symbols.define(
                            value_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
                            false,
                        );
                    }
                    _ => panic!("invalid"),
                }
            }

            let mut ids = idents_from_block(&rng.body.list, c);
            idents.append(&mut ids);
            return idents;
        }
        Statement::Label(lstmt) => {
            // if self.symbols.resolve(lstmt.name.name.as_str()).is_some() {
            //     panic!("label already defined: {:#?}", lstmt.name.name);
            // }
            //
            // let pos = match lstmt.stmt.as_ref() {
            //     Statement::For(f) => f.pos,
            //     Statement::Switch(sw) => sw.pos,
            //     _ => panic!("expected for or switch statement"),
            // };
            //
            // self.label_contexts
            //     .insert((pos, 0), lstmt.name.name.clone());
            // let r = self.compile_statement(lstmt.stmt.as_ref());
            // self.label_contexts.remove(&(pos, 0));
            //
            // return r;
        }
        Statement::Switch(switch) => {
            // let label = self.label_contexts.get(&(switch.pos, 0)).cloned();
            // self.contexts.push(Context::Switch(SwitchContext::new(
            //     self.instructions.len(),
            //     label,
            // )));
            //
            // if let Some(init) = &switch.init {
            //     self.compile_statement(&init)?;
            // }
            // let internal_tag = Ident {
            //     pos: 0,
            //     name: "__tag__".to_string(),
            // };
            // let tag = switch.tag.clone().unwrap_or(Expression::Ident(Ident {
            //     pos: 0,
            //     name: "nil".to_string(),
            // }));
            //
            // self.compile_statement(&Statement::Assign(AssignStmt {
            //     pos: 0,
            //     op: Operator::Define,
            //     left: vec![Expression::Ident(internal_tag.clone())],
            //     right: vec![tag.clone()],
            // }))?;
            //
            // let mut terminates = true;
            // let mut has_default = false;
            //
            // for clause in &switch.block.body {
            //     match clause.tok {
            //         Keyword::Default => {
            //             self.compile_expression(&cond)?;
            //
            //             terminates = terminates
            //                 && self
            //                     .compile_block_statement(&clause.body)?
            //                     .unwrap_or_default();
            //
            //         }
            //         Keyword::Case => {
            //             for expr in &clause.list {
            //                 let cond = match expr {
            //                     Expression::Ident(id) => {
            //                         assert!(switch.tag.is_some());
            //                         Expression::Operation(Operation {
            //                             pos: 0,
            //                             op: Operator::Equal,
            //                             x: Box::new(Expression::Ident(internal_tag.clone())),
            //                             y: Some(Box::new(Expression::Ident(id.clone()))),
            //                         })
            //                     }
            //                     Expression::BasicLit(bl) => {
            //                         assert!(switch.tag.is_some());
            //                         Expression::Operation(Operation {
            //                             pos: 0,
            //                             op: Operator::Equal,
            //                             x: Box::new(Expression::Ident(internal_tag.clone())),
            //                             y: Some(Box::new(Expression::BasicLit(bl.clone()))),
            //                         })
            //                     }
            //                     _ => {
            //                         assert!(switch.tag.is_none());
            //                         expr.clone()
            //                     }
            //                 };
            //
            //                 self.compile_expression(&cond)?;
            //
            //                 let mut clause_body = clause.body.clone();
            //
            //                 if !has_fallthrough {
            //                     clause_body.push(Statement::Branch(BranchStmt {
            //                         pos: 0,
            //                         key: Keyword::Break,
            //                         ident: None,
            //                     }));
            //                 }
            //
            //                 terminates = terminates
            //                     && self
            //                         .compile_block_statement(&clause_body)?
            //                         .unwrap_or_default();
            //
            //                 if self.last_instruction_is(OpCode::Pop) {
            //                     self.remove_last_instruction();
            //                 } else {
            //                     self.emit_opcode(OpCode::Null);
            //                 }
            //
            //                 let pos_jump = self.instructions.len();
            //                 self.emit_opcode(OpCode::Jump);
            //                 self.emit_u16(JUMP_PLACEHOLDER);
            //
            //                 self.change_jump_operand_at(
            //                     pos_jump_if_false,
            //                     self.instructions.len().try_into().unwrap(),
            //                 );
            //
            //                 self.change_jump_operand_at(
            //                     pos_jump,
            //                     self.instructions.len().try_into().unwrap(),
            //                 );
            //             }
            //         }
            //         _ => unimplemented!(),
            //     }
            // }
            //
            // return Ok(Some(terminates));
        }
        Statement::TypeSwitch(switch) => {
            // let label = self.label_contexts.get(&(switch.pos, 0)).cloned();
            // self.contexts.push(Context::Switch(SwitchContext::new(
            //     self.instructions.len(),
            //     label,
            // )));
            //
            // if let Some(init) = &switch.init {
            //     self.compile_statement(&init)?;
            // }
            //
            // let internal_tag = Ident {
            //     pos: 0,
            //     name: "__tag__".to_string(),
            // };
            //
            // let (mut left_ass, mut left_ass_type) = (None, None);
            //
            // match switch.tag.clone().map(|a| *a) {
            //     Some(Statement::Expr(expr)) => {
            //         self.compile_statement(&Statement::Assign(AssignStmt {
            //             pos: 0,
            //             op: Operator::Define,
            //             left: vec![Expression::Ident(internal_tag.clone())],
            //             right: vec![expr.expr],
            //         }))?;
            //     }
            //     Some(Statement::Assign(ass)) => {
            //         let left = ass.left.first().unwrap().as_ident().unwrap().clone();
            //         left_ass = Some(left.clone());
            //
            //         self.compile_statement(&Statement::Assign(ass))?;
            //         self.compile_statement(&Statement::Assign(AssignStmt {
            //             pos: 0,
            //             op: Operator::Define,
            //             left: vec![Expression::Ident(internal_tag.clone())],
            //             right: vec![Expression::Ident(left.clone())],
            //         }))?;
            //         left_ass_type = Some(self.symbols.resolve(&left.name).unwrap().get_type());
            //     }
            //     Some(_) => unreachable!(),
            //     None => unreachable!(),
            // }
            //
            // let mut terminates = true;
            // let mut has_default = false;
            //
            // for clause in &switch.block.body {
            //     match clause.tok {
            //         Keyword::Default => {
            //             if has_default {
            //                 panic!("only one default allowed within a switch");
            //             }
            //
            //             if let (Some(lat), Some(la)) = (&left_ass_type, &left_ass) {
            //                 let updated = self.symbols.update_dt(&la.name, lat.clone());
            //                 assert!(updated);
            //             }
            //
            //             has_default = true;
            //             let cond = Expression::BasicLit(BasicLit {
            //                 pos: 0,
            //                 kind: LitKind::Ident,
            //                 value: "true".to_string(),
            //             });
            //
            //             self.compile_expression(&cond)?;
            //
            //             if self.last_instruction_is(OpCode::Pop) {
            //                 self.remove_last_instruction();
            //             }
            //
            //             let pos_jump_if_false = self.instructions.len();
            //             self.emit_opcode(OpCode::JumpIfFalse);
            //             self.emit_u16(JUMP_PLACEHOLDER);
            //
            //             terminates = terminates
            //                 && self
            //                     .compile_block_statement(&clause.body)?
            //                     .unwrap_or_default();
            //
            //             if self.last_instruction_is(OpCode::Pop) {
            //                 self.remove_last_instruction();
            //             } else {
            //                 self.emit_opcode(OpCode::Null);
            //             }
            //
            //             let pos_jump = self.instructions.len();
            //             self.emit_opcode(OpCode::Jump);
            //             self.emit_u16(JUMP_PLACEHOLDER);
            //
            //             self.change_jump_operand_at(
            //                 pos_jump_if_false,
            //                 self.instructions.len().try_into().unwrap(),
            //             );
            //
            //             self.change_jump_operand_at(
            //                 pos_jump,
            //                 self.instructions.len().try_into().unwrap(),
            //             );
            //         }
            //         Keyword::Case => {
            //             for expr in &clause.list {
            //                 match expr {
            //                     Expression::Ident(id) => {
            //                         // in each case the tag identifier has to be updated to the case's asserted type
            //                         // only if there is a single clause in the case
            //                         if clause.list.len() == 1 {
            //                             if let Some(la) = &left_ass {
            //                                 let r =
            //                                     self.symbols.resolve(&id.name).unwrap().get_type();
            //
            //                                 let updated = self
            //                                     .symbols
            //                                     .update_dt(&la.name, DefineType::Var(Box::new(r)));
            //                                 assert!(updated);
            //                             }
            //                         }
            //
            //                         assert!(switch.tag.is_some());
            //                         self.compile_expression(&Expression::Ident(
            //                             internal_tag.clone(),
            //                         ))?;
            //                         self.compile_expression(&Expression::Ident(id.clone()))?;
            //                         self.emit_opcode(OpCode::TypeCmp);
            //                     }
            //                     Expression::TypePointer(pt) => {
            //                         let id = pt.typ.as_ident().unwrap();
            //
            //                         // in each case the tag identifier has to be updated to the case's asserted type
            //                         // only if there is a single clause in the case
            //                         if clause.list.len() == 1 {
            //                             if let Some(la) = &left_ass {
            //                                 let r =
            //                                     self.symbols.resolve(&id.name).unwrap().get_type();
            //
            //                                 let updated = self
            //                                     .symbols
            //                                     .update_dt(&la.name, DefineType::Var(Box::new(r)));
            //
            //                                 assert!(updated);
            //                             }
            //                         }
            //
            //                         self.compile_expression(&Expression::Ident(
            //                             internal_tag.clone(),
            //                         ))?;
            //                         self.compile_expression(&Expression::Ident(id.clone()))?;
            //                         self.emit_opcode(OpCode::Ref);
            //                         self.emit_opcode(OpCode::TypeCmp);
            //                     }
            //                     _ => {
            //                         assert!(switch.tag.is_none());
            //                         self.compile_expression(&expr)?;
            //                     }
            //                 };
            //
            //                 if self.last_instruction_is(OpCode::Pop) {
            //                     self.remove_last_instruction();
            //                 }
            //
            //                 let pos_jump_if_false = self.instructions.len();
            //                 self.emit_opcode(OpCode::JumpIfFalse);
            //                 self.emit_u16(JUMP_PLACEHOLDER);
            //
            //                 let mut has_fallthrough = false;
            //                 let bl = clause.body.len();
            //                 for (i, stmt) in clause.body.iter().enumerate() {
            //                     let is_fallthrough = if let Statement::Branch(br) = stmt {
            //                         br.key == Keyword::FallThrough
            //                     } else {
            //                         false
            //                     };
            //                     if is_fallthrough {
            //                         if i == bl - 1 {
            //                             has_fallthrough = true;
            //                         } else {
            //                             panic!("misplaced fallthrough");
            //                         }
            //                     }
            //                 }
            //
            //                 let mut clause_body = clause.body.clone();
            //
            //                 if !has_fallthrough {
            //                     clause_body.push(Statement::Branch(BranchStmt {
            //                         pos: 0,
            //                         key: Keyword::Break,
            //                         ident: None,
            //                     }));
            //                 }
            //
            //                 terminates = terminates
            //                     && self
            //                         .compile_block_statement(&clause_body)?
            //                         .unwrap_or_default();
            //
            //                 if self.last_instruction_is(OpCode::Pop) {
            //                     self.remove_last_instruction();
            //                 } else {
            //                     self.emit_opcode(OpCode::Null);
            //                 }
            //
            //                 let pos_jump = self.instructions.len();
            //                 self.emit_opcode(OpCode::Jump);
            //                 self.emit_u16(JUMP_PLACEHOLDER);
            //
            //                 self.change_jump_operand_at(
            //                     pos_jump_if_false,
            //                     self.instructions.len().try_into().unwrap(),
            //                 );
            //
            //                 self.change_jump_operand_at(
            //                     pos_jump,
            //                     self.instructions.len().try_into().unwrap(),
            //                 );
            //             }
            //         }
            //         _ => unimplemented!(),
            //     }
            // }
            //
            // let ctx = self.contexts.pop().unwrap().to_switch();
            //
            // for ip in &ctx.break_instructions {
            //     self.change_jump_operand_at(*ip, self.instructions.len().try_into().unwrap());
            // }
            //
            // return Ok(Some(terminates));
        }
        _ => {
            panic!("stmt not supported: {:#?}", stmt);
        }
    }

    idents
}

pub(crate) fn ident_expression(
    expr: &Expression,
    c: &mut Compiler,
) -> (Vec<(String, DefineType)>, DefineType) {
    let mut idents = vec![];

    match expr {
        //todo this is a total mess: fix me
        Expression::Call(call) => {
            let ct = CallType::from_call(&call, c);

            let rt = match ct {
                CallType::Func { func_dt, .. } => {
                    let (_, _, mut arg_types, rts) = func_dt.as_func();
                    let rts = rts.type_to_val_t();
                    //println!("{:#?}", arg_types);
                    //assert_eq!(arg_types.len(), call.args.len());
                    let (is_variadic, variadic_len) = if let Some(last) = arg_types.last().cloned()
                    {
                        let dt = last.get_type();
                        if dt.is_variadic() {
                            arg_types.pop();
                            let v_t = dt.as_variadic();
                            let mut length = 0;

                            while arg_types.len() < call.args.len() {
                                length += 1;
                                arg_types.push(ContextType::Named("".to_string(), v_t.clone()))
                            }

                            (true, length)
                        } else {
                            (false, 0)
                        }
                    } else {
                        (false, 0)
                    };

                    let variadic_start = arg_types.len() - variadic_len;

                    for (i, (a, t)) in call.args.iter().zip(arg_types).enumerate() {
                        let got = self.compile_expression(a)?.strip_var();
                        let expected = t.get_type();

                        if expected.is_interface() && got.implements(&expected, self) {
                            let (name, _) = expected.as_interface();
                            let (s, _) = self.symbols.resolve(&name).unwrap().as_local();

                            self.emit_opcode(OpCode::Upcast);
                            self.emit_u16(s.index);
                        } else {
                            let got = got.strip_var();
                            let t = t.get_type().strip_type();

                            if is_variadic && i >= variadic_start {
                                match got {
                                    DefineType::Array { inner_type, .. } => {
                                        assert_eq!(t, inner_type.strip_type());
                                    }
                                    DefineType::Slice(inner_type) => {
                                        assert_eq!(t, inner_type.strip_type());
                                    }
                                    got_t => assert_eq!(t, got_t),
                                }
                            } else {
                                assert_eq!(t.strip_type(), got.strip_type());
                            }
                        }
                    }

                    if is_variadic {
                        self.emit_opcode(OpCode::Variadic);
                        //panic!("{}", variadic_len);
                        self.emit_u16(variadic_len as u16);
                    }
                    self.compile_expression(call.func.as_ref())?;

                    self.emit_opcode(OpCode::Call);
                    let arg_len: u8 = call.args.len().try_into().unwrap();

                    let v_len = if is_variadic && variadic_len > 0 {
                        variadic_len - 1
                    } else {
                        0
                    };

                    //println!("{}-{}", arg_len, v_len);

                    self.emit_u8(arg_len - v_len as u8);

                    rts
                }
                CallType::Method {
                    mangled_name,
                    struct_expr,
                    method_dt,
                    struct_dt,
                    ..
                } => {
                    let (_, recv, arg_types, rts) = method_dt.as_func();
                    let rts = rts.type_to_val_t();
                    assert_eq!(arg_types.len(), call.args.len());

                    //here we do automatic passing by reference
                    // if the signature of the function is by ref
                    // and our value is not we emit a ref opcode
                    //let got = self.compile_expression(&struct_expr)?;
                    if struct_dt.is_ref() && !recv.unwrap().is_ref() {
                        self.emit_opcode(OpCode::Ref);
                    }

                    for (a, t) in call.args.iter().zip(arg_types) {
                        let got = self.compile_expression(a)?;
                        assert_eq!(t.as_named().unwrap().1, got);
                    }

                    self.compile_expression(&Expression::Ident(Ident {
                        pos: 0,
                        name: mangled_name,
                    }))?;

                    self.emit_opcode(OpCode::Call);
                    let arg_len: u8 = call.args.len().try_into().unwrap();
                    self.emit_u8(arg_len + 1);

                    rts
                }
                CallType::DynamicDispatch {
                    method_index,
                    method_dt,
                    iface_expr,
                } => {
                    let (_, _, arg_types, rts) = method_dt.as_func();
                    let rts = rts.type_to_val_t();
                    assert_eq!(arg_types.len(), call.args.len());

                    //downcast for the receiver
                    self.compile_expression(&iface_expr)?;
                    self.emit_opcode(OpCode::Downcast);

                    for (a, t) in call.args.iter().zip(arg_types) {
                        let got = self.compile_expression(a)?;
                        match t {
                            ContextType::Named(_, adt) => {
                                assert_eq!(adt, got);
                            }
                            ContextType::Embedded(_, adt) => {
                                assert_eq!(adt, got);
                            }
                            ContextType::Unnamed(adt) => {
                                assert_eq!(adt.as_type().0, got);
                            }
                        }
                    }

                    // need to push the same interface for the dynamic dispatch info
                    self.compile_expression(&iface_expr)?;
                    self.emit_opcode(OpCode::DynamicDispatch);
                    let arg_len: u16 = call.args.len().try_into().unwrap();
                    self.emit_u16(arg_len + 1);
                    self.emit_u16(method_index as u16);

                    rts
                }
            };

            return Ok(rt);
        }
        Expression::TypeMap(_tm) => {
            let rt = self.expression_to_define_type(expr).unwrap();
            let obj = rt.clone().to_object();
            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(rt);
        }
        Expression::Operation(op) => {
            match op.op {
                Operator::Star => {
                    match &op.y {
                        // a * b // multiplication
                        Some(y) => {
                            match (op.x.as_ref(), y.as_ref()) {
                                (Expression::Ident(name), Expression::BasicLit(lit))
                                | (Expression::BasicLit(lit), Expression::Ident(name))
                                    if lit.kind == LitKind::Integer =>
                                {
                                    let value: isize = lit.value.parse().unwrap();
                                    let res = self.compile_const_var_infix_expression(
                                        &name.name, value, &op.op,
                                    );
                                    if res.is_ok() {
                                        return Ok(res.unwrap());
                                    }
                                }
                                _ => (),
                            }

                            // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                            let rt_left = self.compile_expression(op.x.as_ref())?.strip_var();
                            let rt_right = self.compile_expression(y.as_ref())?.strip_var();

                            fn emit_opcode(offset: u8, dt: &DefineType, c: &mut Compiler) {
                                match dt {
                                    DefineType::Float32 => {
                                        c.emit_opcode(OpCode::CastToFloat32);
                                        c.emit_u8(offset);
                                    }
                                    DefineType::Float64 => {
                                        c.emit_opcode(OpCode::CastToFloat64);
                                        c.emit_u8(offset);
                                    }
                                    _ => unimplemented!(),
                                }
                            }

                            match (
                                rt_left.is_const_coerceable_to(&rt_right),
                                rt_right.is_const_coerceable_to(&rt_left),
                            ) {
                                (true, false) => {
                                    emit_opcode(1, &rt_right, self);
                                }
                                (false, true) => {
                                    emit_opcode(0, &rt_right, self);
                                }
                                _ => assert_eq!(rt_left, rt_right, "{:#?}", op),
                            }

                            self.compile_operator(&op.op);

                            if op.x.as_ref().is_int_lit() && y.as_ref().is_int_lit() {
                                return Ok(DefineType::Const(Box::new(rt_right)));
                            } else {
                                return Ok(rt_right);
                            }
                        }
                        // *a // deref
                        None => {
                            //panic!("{:#?}", op);
                            let _ident = op.x.as_ident().unwrap();
                            self.compile_expression(op.x.as_ref())?;
                            self.emit_opcode(OpCode::Deref);
                        }
                    }
                }
                Operator::Less
                | Operator::LessEqual
                | Operator::NotEqual
                | Operator::Greater
                | Operator::GreaterEqual => {
                    match &op.y {
                        // a * b // multiplication
                        Some(y) => {
                            match (op.x.as_ref(), y.as_ref()) {
                                (Expression::Ident(name), Expression::BasicLit(lit))
                                | (Expression::BasicLit(lit), Expression::Ident(name))
                                    if lit.kind == LitKind::Integer =>
                                {
                                    let value: isize = lit.value.parse().unwrap();
                                    let res = self.compile_const_var_infix_expression(
                                        &name.name, value, &op.op,
                                    );
                                    if res.is_ok() {
                                        return Ok(res.unwrap());
                                    }
                                }
                                _ => {}
                            }

                            // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                            self.compile_expression(op.x.as_ref())?;
                            self.compile_expression(y.as_ref())?;
                            self.compile_operator(&op.op);

                            return Ok(DefineType::Int);
                        }
                        _ => unimplemented!(),
                    }
                }
                Operator::Add
                | Operator::Sub
                | Operator::Rem
                | Operator::Equal
                | Operator::Quo
                | Operator::AndAnd
                | Operator::OrOr => {
                    match &op.y {
                        Some(y) => {
                            //todo work on Go constants
                            // weird conversions
                            match (op.x.as_ref(), y.as_ref()) {
                                (Expression::Ident(name), Expression::BasicLit(lit))
                                | (Expression::BasicLit(lit), Expression::Ident(name))
                                    if lit.kind == LitKind::Integer =>
                                {
                                    let value = lit
                                        .value
                                        .parse::<isize>()
                                        .or_else(|_| isize::from_str_radix(&lit.value, 16))
                                        .unwrap();

                                    let res = self.compile_const_var_infix_expression(
                                        &name.name, value, &op.op,
                                    );
                                    if res.is_ok() {
                                        return Ok(res.unwrap());
                                    }
                                }
                                _ => {}
                            }

                            // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                            let rt_left = self.compile_expression(op.x.as_ref())?;
                            let rt_right = self.compile_expression(y.as_ref())?;

                            fn emit_opcode(offset: u8, dt: &DefineType, c: &mut Compiler) {
                                match dt {
                                    DefineType::Float32 => {
                                        c.emit_opcode(OpCode::CastToFloat32);
                                        c.emit_u8(offset);
                                    }
                                    DefineType::Float64 => {
                                        c.emit_opcode(OpCode::CastToFloat64);
                                        c.emit_u8(offset);
                                    }
                                    _ => unimplemented!(),
                                }
                            }

                            let rt = match (
                                rt_left.is_const_coerceable_to(&rt_right),
                                rt_right.is_const_coerceable_to(&rt_left),
                            ) {
                                (true, false) => {
                                    emit_opcode(1, &rt_right, self);
                                    rt_right
                                }
                                (false, true) => {
                                    emit_opcode(0, &rt_right, self);
                                    rt_left
                                }
                                _ => {
                                    assert_eq!(
                                        rt_left.strip_var().strip_const(),
                                        rt_right.strip_var().strip_const()
                                    );
                                    rt_right
                                }
                            };

                            self.compile_operator(&op.op);

                            return Ok(rt);
                        }
                        None => {
                            if op.op == Operator::Sub {
                                let left = self.compile_expression(op.x.as_ref())?;
                                assert!(left.is_numeric());
                                self.emit_opcode(OpCode::Negate);
                                return Ok(left);
                            } else {
                                unimplemented!("{:#?}", op)
                            }
                        }
                    }
                }
                Operator::And => {
                    match &op.y {
                        Some(_y) => {
                            // a & b
                        }
                        //reference expression
                        None => {
                            let t = self.compile_expression(&op.x)?;
                            self.emit_opcode(OpCode::Ref);
                            return Ok(DefineType::Ref(Box::new(t.strip_var())));
                        }
                    }
                }
                Operator::Not => match &op.y {
                    None => {
                        let t = self.compile_expression(&op.x)?;

                        if t.strip_var() != DefineType::Bool {
                            panic!("expected bool got {:#?}", t.strip_var());
                        }

                        self.emit_opcode(OpCode::Not);
                        return Ok(DefineType::Bool);
                    }
                    Some(y) => {
                        unimplemented!("operator::not y {:#?}", y)
                    }
                },
                _ => panic!("unsupported op: {:#?}", op),
            }
            //
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::Ident && (lit.value == "iota") => {
            return self.compile_expression(&Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: self.iota.to_string(),
            }));
        }
        Expression::BasicLit(lit)
            if lit.kind == LitKind::Ident && (lit.value == "true" || lit.value == "false") =>
        {
            let opcode = if lit.value == "true" {
                OpCode::True
            } else {
                OpCode::False
            };
            self.emit_opcode(opcode);

            return Ok(DefineType::Bool);
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::Float => {
            let (obj, dt) = match lit.value.parse::<f64>() {
                Ok(f) => (Object::float64(f), DefineType::Float64),
                _ => (
                    Object::float32(lit.value.parse().unwrap()),
                    DefineType::Float32,
                ),
            };

            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(dt);
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::Integer => {
            // add to gc
            let value = lit
                .value
                .parse::<isize>()
                .or_else(|_| isize::from_str_radix(&lit.value, 16))
                .unwrap();

            let idx = self.add_constant(Object::int(value));
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(DefineType::Const(Box::new(DefineType::Int)));
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::String => {
            let obj = Object::string(lit.value.clone());
            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(DefineType::String);
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::Char => {
            let mut chars: Vec<char> = lit.value.chars().collect();

            if chars.is_empty() {
                chars = vec![char::default()];
            } else {
                assert_eq!(3, chars.len());
                chars = vec![chars[1]];
            }

            assert_eq!(1, chars.len(), "{:#?}", chars);

            let obj = Rune::from_char(*chars.first().unwrap());
            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(DefineType::Rune);
        }
        Expression::BasicLit(lit) if lit.kind == LitKind::Ident => {
            let resolved = self
                .symbols
                .resolve(&lit.value)
                .ok_or(Error::ReferenceError(format!(
                    "identifier: {} not found",
                    lit.value
                )))?;

            let (index, getop) = match resolved {
                Resolved::Enclosed((s, _)) => (s.index, OpCode::GetCaptured),
                Resolved::Local((symbol, _)) => match symbol.scope {
                    Scope::Local => (symbol.index, OpCode::GetLocal),
                    Scope::Global => (symbol.index, OpCode::GetGlobal),
                },
            };

            self.emit_opcode(getop);
            self.emit_u16(index);
        }
        Expression::CompositeLit(clit) => {
            //map
            if let Expression::TypeMap(mp) = clit.typ.as_ref() {
                let inner_key_t = match mp.key.as_ref() {
                    Expression::Ident(ident) => ident.clone(),
                    _ => unimplemented!(),
                };

                let inner_val_t = match mp.val.as_ref() {
                    Expression::Ident(ident) => ident.clone(),
                    _ => unimplemented!(),
                };

                let (_, map_key_t) = self
                    .symbols
                    .resolve(inner_key_t.name.as_str())
                    .unwrap()
                    .as_local();
                let (map_key_t, _) = map_key_t.as_type();

                let (_, map_val_t) = self
                    .symbols
                    .resolve(inner_val_t.name.as_str())
                    .unwrap()
                    .as_local();
                let (map_val_t, _) = map_val_t.as_type();

                for v in &clit.val.values {
                    if let Some(key) = &v.key {
                        match key {
                            Element::Expr(el_expr) => {
                                let expr_t = self.compile_expression(el_expr)?;
                                assert_eq!(map_key_t, expr_t);
                            }
                            _ => {
                                panic!("TypeMap val");
                            }
                        }
                    }

                    match &v.val {
                        Element::Expr(el_expr) => {
                            let expr_t = self.compile_expression(el_expr)?;
                            assert_eq!(map_val_t, expr_t);
                        }
                        _ => {
                            panic!("TypeMap key");
                        }
                    }
                }
                self.emit_opcode(OpCode::Map);
                self.emit_u16(clit.val.values.len().try_into().unwrap());
                return Ok(DefineType::Map(Box::new(map_key_t), Box::new(map_val_t)));
            }

            //slice
            if let Expression::TypeSlice(ta) = clit.typ.as_ref() {
                //todo assert length
                //if ta.len != clit.val.values.len() { }

                let slice_t = self.expression_to_define_type(ta.typ.as_ref()).unwrap();
                let mut el_t = None;
                let key_required = clit
                    .val
                    .values
                    .first()
                    .map(|a| a.key.is_some())
                    .unwrap_or_default();

                for v in &clit.val.values {
                    //todo replace this with error handling
                    //this makes sure keyed and unkeyed slice values aren't mixed
                    assert_eq!(key_required, v.key.is_some());

                    match &v.val {
                        Element::Expr(el_expr) => {
                            let expr_t = self.compile_expression(el_expr)?;
                            assert_eq!(slice_t.strip_type(), expr_t);
                            if let Some(expected_t) = &el_t {
                                assert_eq!(expected_t, &expr_t);
                            } else {
                                el_t = Some(expr_t);
                            }
                        }
                        _ => {
                            panic!("123");
                        }
                    }
                }
                self.emit_opcode(OpCode::MakeSlice);
                self.emit_u16(clit.val.values.len().try_into().unwrap());

                let rt = DefineType::Slice(Box::new(slice_t));

                let obj = rt.clone().to_object();
                let cid = self.add_constant(obj);
                self.emit_u16(cid);

                return Ok(rt);
            }

            //struct
            if let Expression::Ident(name) = clit.typ.as_ref() {
                //todo this can be locally defined type
                return Ok(literal::compile_struct(clit, name, self)?);
            }

            //array
            if let Expression::TypeArray(ta) = clit.typ.as_ref() {
                //todo assert length
                //if ta.len != clit.val.values.len() { }

                let slice_t = match ta.typ.as_ref() {
                    Expression::Ident(ident) => self
                        .symbols
                        .resolve(ident.name.as_str())
                        .unwrap()
                        .as_local()
                        .1
                        .strip_type(),
                    Expression::TypeArray(_at) => {
                        self.expression_to_define_type(ta.typ.as_ref()).unwrap()
                    }
                    _ => {
                        unimplemented!("array element type: {:#?}", ta.typ)
                    }
                };

                let mut el_t = None;
                let key_required = clit
                    .val
                    .values
                    .first()
                    .map(|a| a.key.is_some())
                    .unwrap_or_default();

                for v in &clit.val.values {
                    //todo replace this with error handling
                    //this makes sure keyed and unkeyed slice values aren't mixed
                    assert_eq!(key_required, v.key.is_some());

                    match &v.val {
                        Element::Expr(el_expr) => {
                            let expr_t = self.compile_expression(el_expr)?;
                            assert_eq!(slice_t.strip_type(), expr_t);
                            if let Some(expected_t) = &el_t {
                                assert_eq!(expected_t, &expr_t);
                            } else {
                                el_t = Some(expr_t);
                            }
                        }
                        _ => {
                            panic!("123");
                        }
                    }
                }

                self.emit_opcode(OpCode::MakeArray);
                self.emit_u16(clit.val.values.len().try_into().unwrap());

                return Ok(DefineType::Array {
                    inner_type: Box::new(slice_t),
                    len: clit.val.values.len(),
                });
            }

            // anonymous struct literal
            if let Expression::TypeStruct(ts) = clit.typ.as_ref() {
                let mut field_types = vec![];

                //todo tags
                for field in &ts.fields {
                    let (inner_t, is_ref) = match &field.typ {
                        Expression::TypePointer(p) => (p.typ.as_ident().unwrap(), true),
                        _ => (field.typ.as_ident().unwrap(), false),
                    };

                    let r = self.symbols.resolve(&inner_t.name).unwrap().get_type();

                    let dt = if is_ref {
                        DefineType::Ref(Box::new(r.strip_type()))
                    } else {
                        r.strip_type()
                    };

                    for name in &field.name {
                        field_types.push(ContextType::Named(
                            name.name.as_str().to_string(),
                            dt.clone(),
                        ));
                    }
                }

                let ftl = field_types.len();

                let mut field_values = Vec::with_capacity(ftl);

                for _ in 0..ftl {
                    field_values.push(Object::null());
                }

                let name = format!("anonymous_struct {}", self.anonymous_struct);

                let symbol = self.symbols.define(
                    &name,
                    DefineType::Struct {
                        name: name.to_string(),
                        fields: field_types.clone(),
                        methods: vec![],
                    },
                    false,
                );

                for field_type in &mut field_types {
                    let (s, dt) = field_type.as_named().unwrap();
                    let resolved = match dt {
                        DefineType::Ref(_) => DefineType::Ref(Box::new(dt)),
                        _ => dt,
                    };

                    *field_type = ContextType::Named(s, resolved);
                }

                let updated = self.symbols.update_dt(
                    &name,
                    DefineType::Struct {
                        name: name.to_string(),
                        fields: field_types,
                        methods: vec![],
                    },
                );

                assert!(updated);

                let obj = Struct::object(name.to_string(), field_values, vec![], true);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                let opcode = if symbol.scope == Scope::Global {
                    OpCode::SetGlobal
                } else {
                    OpCode::SetLocal
                };
                self.emit_opcode(opcode);
                self.emit_u16(symbol.index);

                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                let rt = self.compile_expression(&Expression::CompositeLit(CompositeLit {
                    typ: Box::new(Expression::Ident(Ident { pos: 0, name })),
                    val: clit.val.clone(),
                }))?;

                self.anonymous_struct += 1;
                return Ok(rt);
            }

            panic!("unknown composite lit {:#?}", clit);
        }
        Expression::Index(ind) => {
            let t = self.compile_expression(&ind.left)?;
            self.compile_expression(&ind.index)?;
            self.emit_opcode(OpCode::IndexGet);

            fn check_t(i: usize, t: DefineType) -> DefineType {
                match t.strip_var() {
                    DefineType::Array { inner_type, .. } => *inner_type,
                    DefineType::Slice(inner_type) => *inner_type,
                    DefineType::Map(_, v) => DefineType::Tuple(vec![*v, DefineType::Bool]),
                    DefineType::Struct { fields, .. } => fields[i].get_type(),
                    DefineType::Ref(r) => DefineType::Ref(Box::new(check_t(i, *r))),
                    k => unimplemented!("i: {:#?} k: {:#?}", i, k),
                }
            }
            //println!("{:#?} {:#?}", t, ind);
            let i = if ind.index.is_int_lit() {
                ind.index.as_int_lit().unwrap_or_default() as usize
            } else {
                0
            };
            let rt = check_t(i, t.strip_var());

            return Ok(rt);
        }
        Expression::Ident(ident) => {
            if &ident.name == "true" {
                self.emit_opcode(OpCode::True);
                return Ok(DefineType::Bool);
            } else if &ident.name == "false" {
                self.emit_opcode(OpCode::False);
                return Ok(DefineType::Bool);
            }

            if ident.name == "iota" {
                return self.compile_expression(&Expression::BasicLit(BasicLit {
                    pos: 0,
                    kind: LitKind::Integer,
                    value: self.iota.to_string(),
                }));
            }

            // panic!("{:#?}", self.symbols);
            return match self.symbols.resolve(&ident.name) {
                Some(Resolved::Local((symbol, dt))) => {
                    let opcode = if symbol.scope == Scope::Global {
                        if is_builtin_const(&ident.name) {
                            OpCode::Const
                        } else {
                            OpCode::GetGlobal
                        }
                    } else {
                        OpCode::GetLocal
                    };

                    self.emit_opcode(opcode);
                    self.emit_u16(symbol.index);
                    //panic!("{:#?}", symbol.index);

                    Ok(dt)
                }
                Some(Resolved::Enclosed((s, t))) => {
                    // enclosed symbols cannot be global
                    self.emit_opcode(OpCode::GetCaptured);
                    self.emit_u16(s.index);
                    //panic!("{:#?}", 2);
                    Ok(t)
                }
                None => Err(Error::ReferenceError(format!(
                    "ident: `{}` is not defined",
                    ident.name
                ))),
            };
        }
        Expression::Selector(sel) => {
            let dt = self
                .compile_expression(sel.x.as_ref())?
                .strip_var()
                .strip_ref();

            let (_, inner_types) = match dt.strip_ref() {
                DefineType::Struct {
                    name,
                    fields: inner_types,
                    ..
                } => (name, inner_types),
                _ => panic!("{:#?}", dt),
            };

            // breadth first search find field name
            // necessary because of embedding
            fn find_field(it: &[ContextType], target: &str) -> Option<(Vec<usize>, ContextType)> {
                use std::collections::VecDeque;

                let mut queue = VecDeque::new();

                for (i, item) in it.iter().enumerate() {
                    queue.push_back((vec![i], item.clone()));
                }

                while let Some((path, current)) = queue.pop_front() {
                    match &current {
                        ContextType::Named(s, _) => {
                            if s == target {
                                return Some((path, current.clone()));
                            }
                        }
                        ContextType::Embedded(s, dt) => {
                            if s == target {
                                return Some((path, current.clone()));
                            }
                            if dt.is_struct() {
                                let (_, children, _) = dt.as_struct().unwrap();
                                for (i, child) in children.iter().enumerate() {
                                    let mut child_path = path.clone();
                                    child_path.push(i);
                                    queue.push_back((child_path, child.clone()));
                                }
                            }
                        }
                        _ => unimplemented!(),
                    }
                }

                None
            }

            let (path, rt) = find_field(inner_types.as_ref(), sel.sel.name.as_str()).expect(
                &format!("field not found: {} in struct: {:#?}", sel.sel.name, dt),
            );

            for p in path {
                self.compile_expression(&Expression::BasicLit(BasicLit {
                    pos: 0,
                    kind: LitKind::Integer,
                    value: format!("{}", p),
                }))?;
                self.emit_opcode(OpCode::IndexGet);
            }

            return Ok(rt.get_type());
        }
        Expression::FuncLit(f) => {
            let pos_jump = self.instructions.len();

            self.func_contexts.push(FuncContext::new(pos_jump));

            self.emit_opcode(OpCode::Jump);
            self.emit_u16(JUMP_PLACEHOLDER);

            let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

            //println!("{:#?}", f.typ.params.list);
            // Compile function in a new scope
            self.symbols.new_context(true);
            for p in &f.typ.params.list {
                let t = self.expression_to_define_type(&p.typ).unwrap();
                for name in &p.name {
                    decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

                    self.symbols.define(
                        &name.name,
                        DefineType::Var(Box::new(t.clone())),
                        t.is_invar(),
                    );
                }
            }

            let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());

            for el in &f.typ.result.list {
                let t = self.expression_to_define_type(&el.typ).unwrap();
                decl_r_types.push(t);
            }

            let r_t = if decl_r_types.is_empty() {
                DefineType::Null
            } else if decl_r_types.len() == 1 {
                decl_r_types[0].clone()
            } else {
                DefineType::Tuple(decl_r_types.clone())
            };

            let pos_start_function = self.instructions.len();

            //todo ugly

            // type checking if all returns are correct types
            let mut has_top_return = false;
            for stmt in &f.body.list {
                if let Statement::Return(_) = stmt {
                    has_top_return = true;
                    break;
                }
            }

            let terminates = self.compile_block_statement(&f.body.list)?;

            let ctx = self.func_contexts.pop().unwrap();

            if !decl_r_types.is_empty() {
                let sorted_decl_r_types: Vec<DefineType> = decl_r_types
                    .iter()
                    .map(|b| {
                        if let DefineType::Type(inner, _) = b.clone() {
                            return *inner;
                        }

                        b.clone()
                    })
                    .collect();

                //todo use terminates to assert if top scope level return is needed

                let expected_t = if sorted_decl_r_types.is_empty() {
                    DefineType::Null
                } else if sorted_decl_r_types.len() == 1 {
                    sorted_decl_r_types[0].clone()
                } else {
                    DefineType::Tuple(sorted_decl_r_types)
                };

                if !terminates.unwrap_or_default()
                    && expected_t != DefineType::Null
                    && !has_top_return
                {
                    panic!("expected return");
                }

                for (mut ret_type, is_type_assert) in ctx.ret_types {
                    if ret_type.is_var() {
                        ret_type = ret_type.as_var();
                    }
                    if !(terminates.unwrap_or_default() && ret_type == DefineType::Null) {
                        if is_type_assert && !expected_t.is_tuple() && ret_type.is_tuple() {
                            let tuple = ret_type.as_tuple();
                            assert_eq!(expected_t, tuple[0]);
                        } else {
                            assert_eq!(expected_t.strip_type(), ret_type.strip_type());
                        }
                    }
                }
            } else {
                for (ret_type, _is_type_assert) in &ctx.ret_types {
                    assert_eq!(ret_type, &DefineType::Null);
                }
            }
            // end type checking on return types

            if self.last_instruction_is(OpCode::Pop) && !decl_r_types.is_empty() {
                self.remove_last_instruction();
                assert!(decl_r_types.len() < u16::MAX as usize);
                let num_r_types = decl_r_types.len() as u16;

                self.emit_opcode(OpCode::ReturnValue);
                self.emit_u16(num_r_types);
            } else if self.last_instruction_is(OpCode::Pop) && decl_r_types.is_empty() {
                self.remove_last_instruction();
                self.emit_opcode(OpCode::Return);
            } else if !self.last_instruction_is(OpCode::ReturnValue) {
                self.emit_opcode(OpCode::Return);
            }

            self.change_jump_operand_at(pos_jump, self.instructions.len().try_into().unwrap());

            // Switch back to previous scope again
            let ctx = self.symbols.leave_context();

            let num_locals = ctx.max_size();

            // Create function object and store as constant
            let obj = Closure::object(
                pos_start_function.try_into().unwrap(),
                num_locals.try_into().unwrap(),
                vec![Object::null(); ctx.captured.len()],
            );
            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            for (i, v) in ctx.captured.iter().enumerate() {
                if let Some(r) = self.symbols.resolve(&v) {
                    match r {
                        Resolved::Local((s, _t)) => {
                            let op = match s.scope {
                                Scope::Local => OpCode::GetLocal,
                                Scope::Global => OpCode::GetGlobal,
                            };
                            self.emit_opcode(op);
                            self.emit_u16(s.index);

                            self.emit_opcode(OpCode::Propagate);
                            self.emit_u16(i.try_into().unwrap());
                        }
                        Resolved::Enclosed((s, _t)) => {
                            self.emit_opcode(OpCode::GetCaptured);
                            self.emit_u16(s.index);

                            self.emit_opcode(OpCode::Propagate);
                            self.emit_u16(i.try_into().unwrap());
                        }
                    }
                }
            }

            return Ok(DefineType::Func {
                name: "".to_string(),
                recv: None,
                args: decl_arg_types,
                rt: Box::new(r_t),
            });
        }
        Expression::Invar(invar) => {
            let rt = self.compile_expression(&invar.expr)?;
            return Ok(DefineType::Invar(Box::new(rt.strip_var())));
        }
        Expression::TypeAssert(type_assert) => {
            let ident = type_assert.left.as_ident().unwrap();
            let r = self.symbols.resolve(&ident.name).unwrap();
            let t = r.get_type();

            assert!(t.is_var());
            assert!(t.as_var().is_interface());

            let rt = self.compile_expression(&type_assert.left)?;

            match &type_assert.right {
                Some(right) => {
                    match right.as_ref() {
                        Expression::Ident(ident) => {
                            let r = self.symbols.resolve(&ident.name).unwrap();
                            let t = r.get_type();

                            match t {
                                //sidecast from interface to interface
                                // 1. downcast to T and upcast to the interface
                                DefineType::Interface { .. } => {
                                    let (s, _) = r.as_local();

                                    self.emit_opcode(OpCode::Downcast);
                                    self.emit_opcode(OpCode::Upcast);
                                    self.emit_u16(s.index);
                                    self.emit_opcode(OpCode::TypeCmp);
                                }
                                DefineType::Struct { .. } | DefineType::Type(_, _) => {
                                    self.emit_opcode(OpCode::Downcast);
                                    self.compile_expression(&type_assert.left)?;
                                    self.emit_opcode(OpCode::Downcast);
                                    self.compile_expression(right)?;
                                    self.emit_opcode(OpCode::TypeCmp);
                                }
                                _ => unimplemented!(),
                            }
                            return Ok(DefineType::Tuple(vec![t, DefineType::Bool]));
                        }
                        _ => unimplemented!("{:#?}", right),
                    }
                }
                None => {
                    self.emit_opcode(OpCode::Downcast);
                    //todo this should return DefineType::Type
                    return Ok(rt);
                }
            }
        }
        Expression::TypeSlice(_ts) => {
            let rt = self.expression_to_define_type(expr).unwrap();
            let obj = rt.clone().to_object();
            //panic!("{:#?}", rt);
            let idx = self.add_constant(obj);
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);

            return Ok(rt);
        }
        Expression::Slice(slice) => {
            let t = self.compile_expression(&slice.left)?;
            match t.strip_var() {
                DefineType::Slice(_) | DefineType::Array { .. } => {}
                tt => panic!("expected slice or array got {:#?}", tt),
            }

            let mut index_iter = slice.index.iter();

            let mut index = 0;
            if let Some(from) = index_iter.next().unwrap() {
                let ind_t = self.compile_expression(from.as_ref())?;

                if !ind_t.is_numeric() {
                    panic!("slicing can be done with integers only");
                }

                index = 1;
            }

            if let Some(from) = index_iter.next().unwrap() {
                let ind_t = self.compile_expression(from.as_ref())?;

                if !ind_t.is_numeric() {
                    panic!("slicing can be done with integers only");
                }

                if index == 0 {
                    index = 2;
                } else {
                    index = 3;
                }
            }

            self.emit_opcode(OpCode::Slice);
            self.emit_u16(index);

            let obj = t.to_object();
            let type_value = obj.as_type_value();
            let mut ind = None;
            for (i, c) in self.constants.iter().enumerate() {
                if c.tag() == Type::Type {
                    let ctv = c.as_type_value();

                    if ctv == type_value {
                        ind = Some(i);
                        break;
                    }
                }
            }
            self.emit_u16(ind.unwrap().try_into().unwrap());
        }
        _ => {
            return Err(Error::SyntaxError(format!(
                "unsupported expression:  {:#?}",
                expr
            )))
        }
    }

    Ok(DefineType::Null)
}
