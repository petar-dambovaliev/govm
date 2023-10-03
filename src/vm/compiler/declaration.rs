use crate::parser::ast::{
    ConstSpec, Decl, Declaration, Expression, FuncDecl, InterfaceType, Statement, StructType,
    TypeSpec, VarSpec,
};
use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::{FuncContext, OpCode, JUMP_PLACEHOLDER};
use crate::vm::object::structure::{Interface, Struct};
use crate::vm::object::{is_builtin_const, Object, Type};
use crate::vm::symbols::{ContextType, DefineType, Scope};
use crate::vm::{builtin, Error};
use ahash::{HashMap, HashMapExt};

use crate::parser::token::LitKind;
use dep_graph::{DepGraph, Node};

pub fn make_var_const_dep_graph(
    declrs: &[Declaration],
    c: &mut Compiler,
) -> (
    DepGraph<(String, DefineType)>,
    HashMap<(String, DefineType), Declaration>,
) {
    let mut nodes = vec![];
    let mut declrs_map = HashMap::new();

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
                    let (r_name, r_fields, mut r_methods) = c
                        .symbols
                        .resolve(&recv.get_type_name())
                        .unwrap()
                        .get_type()
                        .as_struct()
                        .unwrap();

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
                }

                let key = (f.name.name.to_string(), func_def.clone());
                let node = Node::new(key.clone());
                nodes.push(node);

                declrs_map.insert(key.clone(), declr.clone());
            }
            _ => {}
        }
    }

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

                        let deps = get_const_idents_from_expr(expr, c);

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

                        let deps = get_const_idents_from_expr(expr, c);

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

                for stmt in &f.body.as_ref().unwrap().list {
                    idents.append(&mut get_const_idents_from_stmt(stmt, c));
                }

                for (name, dt) in idents {
                    if let Some(dep) = nodes.iter().find(|n| n.id().0 == name).cloned() {
                        nodes[pos].add_dep(dep.id().clone());
                    }
                }
            }
            _ => {}
        }
    }

    let graph = DepGraph::new(&nodes);
    (graph, declrs_map)
}

fn get_const_idents_from_stmt(stmt: &Statement, c: &mut Compiler) -> Vec<(String, DefineType)> {
    let mut idents = vec![];
    match stmt {
        Statement::Expr(expr) => {
            idents.append(&mut get_const_idents_from_expr(&expr.expr, c));
        }
        Statement::Return(ret) => {
            for r in &ret.ret {
                idents.append(&mut get_const_idents_from_expr(r, c));
            }
        }
        Statement::If(ifstmt) => {
            for s in &ifstmt.body.list {
                idents.append(&mut get_const_idents_from_stmt(s, c));
            }
            if let Some(init) = &ifstmt.init {
                idents.append(&mut get_const_idents_from_stmt(init.as_ref(), c));
            }
            idents.append(&mut get_const_idents_from_expr(&ifstmt.cond, c));
            if let Some(els) = &ifstmt.else_ {
                idents.append(&mut get_const_idents_from_stmt(els.as_ref(), c));
            }
        }
        t => println!("get_const_idents_from_stmt: not implemented {:#?}", t),
    }
    idents
}

fn get_const_idents_from_expr(expr: &Expression, c: &mut Compiler) -> Vec<(String, DefineType)> {
    let mut idents = vec![];
    match expr {
        Expression::Ident(id) => {
            if builtin::resolve(id.name.as_str()).is_none() {
                let dt = c
                    .symbols
                    .resolve(id.name.as_str())
                    .expect(&format!(
                        "get_const_idents_from_expr: ident not found {}",
                        id.name.as_str()
                    ))
                    .get_type();
                idents.push((id.name.clone(), dt));
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
            ));

            if let Some(s) = &op.y {
                idents.append(&mut get_const_idents_from_expr(&mut s.as_ref().clone(), c));
            }
        }
        Expression::Call(call) => {
            idents.append(&mut get_const_idents_from_expr(&call.func, c));
            for arg in &call.args {
                idents.append(&mut get_const_idents_from_expr(&arg, c));
            }
        }
        t => println!("get_const_idents_from_expr: not implemented {:#?}", t),
    }
    idents
}

pub fn register_global_types(declrs: &[Declaration], c: &mut Compiler) -> (Vec<Declaration>) {
    let mut strcts = vec![];

    for declr in declrs {
        match declr {
            Declaration::Type(tspec) => {
                for spec in &tspec.specs {
                    if !spec.alias {
                        match &spec.typ {
                            Expression::TypeInterface(_it) => {
                                strcts.push(declr.clone());
                                let _ = c.symbols.define(
                                    &spec.name.name.clone(),
                                    DefineType::Interface {
                                        name: spec.name.name.clone(),
                                        methods: vec![],
                                    },
                                    false,
                                );
                            }
                            Expression::TypeStruct(_ta) => {
                                strcts.push(declr.clone());
                                let name = spec.name.name.as_str();
                                println!("register struct: {}", name);
                                let _ = c.symbols.define(
                                    name,
                                    DefineType::Struct {
                                        name: name.to_string(),
                                        fields: vec![],
                                        methods: vec![],
                                    },
                                    false,
                                );
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

    strcts
}

pub fn compile_variable(v: &Decl<VarSpec>, c: &mut Compiler) -> Result<(), Error> {
    for spec in &v.specs {
        let mut declared_tp = None;
        let mut value_is_default = false;
        let values = if spec.values.is_empty() {
            let tp = c
                .expression_to_define_type(
                    spec.typ
                        .as_ref()
                        .expect("no declared values requires a declared type"),
                )
                .unwrap();

            declared_tp = Some(tp.clone());
            let mut defaults = Vec::with_capacity(spec.name.len());
            for _ in 0..spec.name.len() {
                defaults.push(c.make_type_default_val(tp.clone()));
            }
            value_is_default = true;
            defaults
        } else {
            spec.values.clone()
        };

        for (name, value) in spec.name.iter().zip(values.iter()) {
            let mut rt = c.compile_expression(value)?;

            if rt.is_invar() {
                panic!("var cant be invar");
            }

            if let Some(dtp) = &declared_tp {
                if value_is_default && dtp.is_nullable() {
                    c.emit_opcode(OpCode::TypedNull);

                    //todo register all typed nulls
                    if dtp.is_func() {
                        let mut found_closure = false;
                        for (ind, constant) in c.constants.iter().enumerate() {
                            if constant.tag() == Type::Closure {
                                let closure = constant.as_closure();
                                if closure.is_null {
                                    c.emit_u16(ind.try_into().unwrap());
                                    found_closure = true;
                                    break;
                                }
                            }
                        }
                        if !found_closure {
                            panic!("could not find closure constant");
                        }
                    } else if dtp.is_slice() {
                        let cid = c.add_constant(dtp.clone().to_object());
                        c.emit_u16(cid);
                    } else {
                        unimplemented!("typed null: {:#?}", dtp);
                    }
                }

                if rt.is_nil() && (dtp.is_ref() || dtp.is_func() || dtp.is_slice()) {
                    rt = dtp.clone();
                }
            }

            let mut should_upcast = false;

            if let Some(dtp) = &declared_tp {
                if rt != *dtp {
                    if value.is_int_lit() {
                        let i = value.as_int_lit().unwrap();
                        if i >= u8::MIN as isize && i <= u8::MAX as isize {
                            rt = dtp.clone();
                        }
                    } else if dtp.is_interface() {
                        rt = dtp.clone();
                        should_upcast = true;
                    }
                }
            }

            let symbol = if c.symbols.current_context().scope == Scope::Global {
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
                if let Some(dtp) = &declared_tp {
                    let (name, _) = dtp.as_interface();
                    let (s, _) = c.symbols.resolve(&name).unwrap().as_local();

                    c.emit_opcode(OpCode::Upcast);
                    c.emit_u16(s.index);
                }
            }

            let op = if symbol.scope == Scope::Global {
                OpCode::SetGlobal
            } else {
                OpCode::SetLocal
            };
            c.emit_opcode(op);
            c.emit_u16(symbol.index);
        }
    }
    Ok(())
}

pub fn compile_function(f: &FuncDecl, c: &mut Compiler) -> Result<(), Error> {
    let pos_jump = c.instructions.len();

    c.func_contexts.push(FuncContext::new(pos_jump));

    c.emit_opcode(OpCode::Jump);
    c.emit_u16(JUMP_PLACEHOLDER);

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

    let symbol = match c.symbols.resolve(&f_name) {
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

        let (r_name, r_fields, mut r_methods) = c
            .symbols
            .resolve(&t.get_type_name())
            .unwrap()
            .get_type()
            .as_struct()
            .unwrap();

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
    }

    let pos_start_function = c.instructions.len();

    //todo ugly

    // type checking if all returns are correct types
    let mut terminates = None;
    let mut has_top_return = false;
    if let Some(body) = &f.body {
        for stmt in &body.list {
            if let Statement::Return(_) = stmt {
                has_top_return = true;
                break;
            }
        }

        terminates = c.compile_block_statement(&body.list)?;
    } else {
        //todo
        // assert if the function is void but there is a return
        //assert_eq!(rts.is_none());
    }
    let ctx = c.func_contexts.pop().unwrap();

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

        //println!("{:#?}", un_rts);
        if (!terminates.unwrap_or_default() && expected_t != DefineType::Null && !has_top_return)
            || (expected_t != DefineType::Null && ctx.ret_types.is_empty())
        {
            panic!("expected return");
        }

        for (mut ret_type, is_type_assert) in ctx.ret_types {
            if ret_type.is_var() {
                ret_type = ret_type.as_var();
            }

            if ret_type.is_type() {
                ret_type = ret_type.as_type().0;
            }

            if let DefineType::Tuple(tuple) = ret_type {
                let mut res_tuple = vec![];

                for el in tuple {
                    let ell = match el {
                        DefineType::Type(a, _) => a,
                        _ => Box::new(el),
                    };
                    res_tuple.push(*ell);
                }

                ret_type = DefineType::Tuple(res_tuple);
            }

            if !(terminates.unwrap_or_default() && ret_type == DefineType::Null) {
                if is_type_assert && !expected_t.is_tuple() && ret_type.is_tuple() {
                    let tuple = ret_type.as_tuple();
                    assert_eq!(expected_t, tuple[0]);
                } else {
                    assert_eq!(
                        expected_t.strip_type(),
                        ret_type.strip_type().strip_const(),
                        "{:#?}",
                        f.name.name
                    );
                }
            }
        }
    } else {
        for (ret_type, _) in &ctx.ret_types {
            assert_eq!(ret_type, &DefineType::Null);
        }
    }
    // end type checking on return types

    if c.last_instruction_is(OpCode::Pop) && !decl_r_types.is_empty() {
        c.remove_last_instruction();
        assert!(decl_r_types.len() < u16::MAX as usize);
        let num_r_types = decl_r_types.len() as u16;

        c.emit_opcode(OpCode::ReturnValue);
        c.emit_u16(num_r_types);
    } else if c.last_instruction_is(OpCode::Pop) && decl_r_types.is_empty() {
        c.remove_last_instruction();
        c.emit_opcode(OpCode::Return);
    } else if !c.last_instruction_is(OpCode::ReturnValue) {
        c.emit_opcode(OpCode::Return);
    }

    c.change_jump_operand_at(pos_jump, c.instructions.len().try_into().unwrap());

    // Switch back to previous scope again
    let ctx = c.symbols.leave_context();

    //add method start position for dynamic dispatch
    if let Some(recv) = recv {
        let t = c.expression_to_define_type(&recv.typ).unwrap();
        let mut added = false;

        if let DefineType::Struct { name, .. } = &t.strip_ref() {
            for constant in &mut c.constants {
                if constant.tag() == Type::Struct {
                    let strct = constant.as_struct_mut();
                    if &strct.name == name {
                        added = true;
                        strct
                            .method_dispatch
                            .push((f.name.name.clone(), pos_start_function));
                        //println!("add {:#?} to {:#?}", f.name.name, strct.name);
                    }
                    //println!("{:#?}", constant);
                }
            }

            if !added {
                panic!("internal error: could not added method to struct");
            }
        }
    }

    // Create function object and store as constant
    let obj = Object::function(
        pos_start_function.try_into().unwrap(),
        ctx.max_size().try_into().unwrap(),
    );
    let idx = c.add_constant(obj);
    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let opcode = if symbol.scope == Scope::Global {
        OpCode::SetGlobal
    } else {
        OpCode::SetLocal
    };
    c.emit_opcode(opcode);
    c.emit_u16(symbol.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    Ok(())
}

pub fn compile_const(c: &Decl<ConstSpec>, compiler: &mut Compiler) -> Result<(), Error> {
    for spec in &c.specs {
        for (name, value) in spec.name.iter().zip(spec.values.iter()) {
            let rt = compiler.compile_expression(value)?;

            let symbol = compiler.symbols.define(
                name.name.as_str(),
                DefineType::Const(Box::new(rt.clone())),
                false,
            );

            let op = if symbol.scope == Scope::Global {
                OpCode::SetGlobal
            } else {
                OpCode::SetLocal
            };
            compiler.emit_opcode(op);
            compiler.emit_u16(symbol.index);
        }
    }
    Ok(())
}

pub fn type_interface(spec: &TypeSpec, it: &InterfaceType, c: &mut Compiler) {
    let mut funcs = Vec::with_capacity(it.methods.list.len());
    let mut func_names = Vec::with_capacity(it.methods.list.len());

    for field in &it.methods.list {
        let func_name = field.name.first().unwrap();
        let (_, _, args, rt) = c.expression_to_define_type(&field.typ).unwrap().as_func();

        funcs.push(DefineType::Func {
            name: func_name.name.to_string(),
            recv: None,
            args,
            rt,
        });
        func_names.push(func_name.name.to_string());
    }

    let s = match c.symbols.resolve(&spec.name.name) {
        Some(s) => {
            let updated = c.symbols.update_dt(
                &spec.name.name.clone(),
                DefineType::Interface {
                    name: spec.name.name.clone(),
                    methods: funcs,
                },
            );

            assert!(updated);
            s.get_symbol()
        }
        None => c.symbols.define(
            &spec.name.name.clone(),
            DefineType::Interface {
                name: spec.name.name.clone(),
                methods: funcs,
            },
            false,
        ),
    };

    let obj = Interface::object(spec.name.name.clone(), func_names, Object::null());
    let idx = c.add_constant(obj);
    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let opcode = if s.scope == Scope::Global {
        OpCode::SetGlobal
    } else {
        OpCode::SetLocal
    };
    c.emit_opcode(opcode);
    c.emit_u16(s.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);
}

pub fn type_struct(spec: &TypeSpec, ta: &StructType, c: &mut Compiler) {
    let t = spec.name.clone();
    let mut field_types = vec![];

    //todo tags
    for field in &ta.fields {
        let (inner_t, is_ref) = match &field.typ {
            Expression::TypePointer(p) => (p.typ.as_ident().unwrap(), true),
            _ => (field.typ.as_ident().unwrap(), false),
        };

        if !is_ref && t.name == inner_t.name {
            panic!("recursive definition");
        }

        let r = c.symbols.resolve(&inner_t.name).unwrap().get_type();

        let dt = if is_ref {
            DefineType::Ref(Box::new(r.strip_type()))
        } else {
            r.strip_type()
        };

        if field.name.is_empty() {
            field_types.push(ContextType::Embedded(inner_t.name.clone(), dt.clone()));
        } else {
            for name in &field.name {
                field_types.push(ContextType::Named(
                    name.name.as_str().to_string(),
                    dt.clone(),
                ));
            }
        }
    }

    let ftl = field_types.len();

    let mut field_values = Vec::with_capacity(ftl);

    for _ in 0..ftl {
        field_values.push(Object::null());
    }

    let name = spec.name.name.as_str();

    let symbol = match c.symbols.resolve(&spec.name.name) {
        Some(s) => s.get_symbol(),
        None => c.symbols.define(
            name,
            DefineType::Struct {
                name: name.to_string(),
                fields: field_types.clone(),
                methods: vec![],
            },
            false,
        ),
    };

    for field_type in &mut field_types {
        match field_type {
            ContextType::Named(s, dt) => {
                let resolved = match dt {
                    DefineType::Ref(_) => DefineType::Ref(Box::new(dt.clone())),
                    _ => dt.clone(),
                };

                *field_type = ContextType::Named(s.clone(), resolved);
            }
            ContextType::Embedded(s, dt) => {
                let resolved = match dt {
                    DefineType::Ref(_) => DefineType::Ref(Box::new(dt.clone())),
                    _ => dt.clone(),
                };

                *field_type = ContextType::Embedded(s.clone(), resolved);
            }
            _ => unimplemented!(),
        };
    }

    let updated = c.symbols.update_struct_fields(name, field_types.clone());

    assert!(updated);

    let obj = Struct::object(name.to_string(), field_values, vec![], false);
    let idx = c.add_constant(obj);
    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let opcode = if symbol.scope == Scope::Global {
        OpCode::SetGlobal
    } else {
        OpCode::SetLocal
    };
    c.emit_opcode(opcode);
    c.emit_u16(symbol.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let strct = c.symbols.resolve(name).unwrap().get_type();

    for field_type in &mut field_types {
        if let ContextType::Embedded(s, dt) = field_type {
            if dt.is_struct() {
                let (_, _, methods) = dt.as_struct().unwrap();
                let r = if dt.is_ref() { "*" } else { "" };

                for method in methods {
                    let (f_name, _recv, args, rets) = method.as_func();
                    if !strct.struct_has_method(&f_name) {
                        let args_def: Vec<String> = args
                            .iter()
                            .map(|a| {
                                let (n, t) = a.as_named().unwrap();
                                format!("{n} {t}")
                            })
                            .collect();

                        let args_def_str = args_def.join(",");

                        let args_pass: Vec<String> = args
                            .iter()
                            .map(|a| {
                                let (n, _t) = a.as_named().unwrap();
                                n
                            })
                            .collect();

                        let args_pass_str = args_pass.join(",");
                        let rets_str = rets.fmt_rt();

                        let gen_m_str = format!(
                            r#"
                            func (a {r}{name}) {f_name}({args_def_str}) ({rets_str}) {{
                                return a.{s}.{f_name}({args_pass_str})  
                            }}
                        "#,
                        );

                        println!("gen_m_str: {}", gen_m_str);
                        let mut p = Parser::from(gen_m_str);

                        let gen_m = p.parse_func_decl().unwrap();

                        c.compile_declaration(&Declaration::Function(gen_m))
                            .unwrap();
                    }
                }
            }
        }
    }
}
