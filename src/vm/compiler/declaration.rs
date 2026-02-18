use crate::parser::ast::{
    ConstSpec, Decl, Declaration, Expression, FuncDecl, Ident, InterfaceType, Statement,
    StructType, TypeSpec, VarSpec,
};
use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::{make_method_name, FuncContext, OpCode, JUMP_PLACEHOLDER};
use crate::vm::object::structure::{Alias, Interface, Struct};
use crate::vm::object::{Object, Type};
use crate::vm::symbols::{ContextType, DefineType, Scope};
use crate::vm::Error;

pub fn compile_variable(pkg: &str, v: &Decl<VarSpec>, c: &mut Compiler) -> Result<(), Error> {
    for spec in &v.specs {
        let declared_tp = c
            .expression_to_define_type(
                &pkg,
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
            let mut rt = c.compile_expression(pkg, value)?;
            if value_is_default && declared_tp.is_nullable() {
                c.emit_opcode(OpCode::TypedNull);

                //todo register all typed nulls
                if declared_tp.is_func() {
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
                } else if declared_tp.is_slice() {
                    let cid = c.add_constant(declared_tp.clone().to_object());
                    c.emit_u16(cid);
                } else {
                    unimplemented!("typed null: {:#?}", declared_tp);
                }
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

            let symbol = if c.symbols.current_context().scope == Scope::Global {
                let updated = c.symbols.update_dt(
                    pkg,
                    name.name.as_str(),
                    DefineType::Var(Box::new(rt.clone())),
                );

                assert!(updated, "{}", name.name);

                c.symbols
                    .resolve(pkg, name.name.as_str())
                    .unwrap()
                    .get_symbol()
            } else {
                c.symbols.define(
                    pkg,
                    name.name.as_str(),
                    DefineType::Var(Box::new(rt.clone())),
                    rt.is_invar(),
                )
            };

            if should_upcast {
                let (name, _) = declared_tp.as_interface();
                let (s, _, _) = c.symbols.resolve(pkg, &name).unwrap().as_local();

                c.emit_opcode(OpCode::Upcast);
                c.emit_u16(s.index);
            } else if should_cast_alias {
                let (name, _, _, _) = declared_tp.as_spec().unwrap();
                let (s, _, _) = c.symbols.resolve(pkg, &name).unwrap().as_local();

                c.emit_opcode(OpCode::CastToAlias);
                c.emit_u16(s.index);
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

pub fn compile_function(pkg: &str, f: &FuncDecl, c: &mut Compiler) -> Result<(), Error> {
    let pos_jump = c.instructions.len();

    c.func_contexts.push(FuncContext::new(pos_jump));

    c.emit_opcode(OpCode::Jump);
    c.emit_u16(JUMP_PLACEHOLDER);

    let (f_name, recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
        let recv = recv.list.first().unwrap();
        let t = c.expression_to_define_type(pkg, &recv.typ).unwrap();

        let ret = (
            make_method_name(pkg, t.strip_ref(), &f.name.name),
            Some(recv),
            Some(Box::new(t)),
        );

        ret
    } else {
        (f.name.name.clone(), None, None)
    };

    let symbol = match c.symbols.resolve(pkg, &f_name) {
        Some(s) => {
            let updated = c.symbols.update_dt(
                pkg,
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

    // Compile function in a new scope
    c.symbols.new_context(false);

    if let Some(recv) = recv {
        let t = c.expression_to_define_type(pkg, &recv.typ).unwrap();

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
            pkg,
            &recv.name.first().unwrap().name,
            DefineType::Var(Box::new(t.clone())),
            t.is_invar(),
        );
    }

    let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

    for p in &f.typ.params.list {
        let t = c.expression_to_define_type(pkg, &p.typ).unwrap();
        for name in &p.name {
            decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

            c.symbols.define(
                pkg,
                &name.name,
                DefineType::Var(Box::new(t.clone())),
                t.is_invar(),
            );
        }
    }

    let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());

    for el in &f.typ.result.list {
        let t = c.expression_to_define_type(pkg, &el.typ).unwrap();
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
    let updated = c.symbols.update_dt(pkg, &f_name, func_def.clone());
    assert!(updated);

    c.func_contexts.last_mut().unwrap().expected_ret = r_t;

    //add method to struct symbol
    if let Some(recv) = recv {
        let t = c.expression_to_define_type(pkg, &recv.typ).unwrap();

        let tt = c
            .symbols
            .resolve(pkg, &t.get_type_name())
            .unwrap()
            .get_type()
            .0;

        if tt.is_struct() {
            let (r_name, r_fields, mut r_methods) = tt.as_struct().unwrap();

            r_methods.push(func_def.clone());
            let updated = c.symbols.update_dt(
                pkg,
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
                pkg,
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

        terminates = c.compile_block_statement(pkg, &body.list)?;
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
        let t = c.expression_to_define_type(&pkg, &recv.typ).unwrap();
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
        } else if let DefineType::Spec { name, .. } = &t.strip_ref() {
            for constant in &mut c.constants {
                if constant.tag() == Type::Alias {
                    let alias = constant.as_alias_mut();
                    if &alias.name == name {
                        added = true;
                        alias
                            .method_dispatch
                            .push((f.name.name.clone(), pos_start_function));
                        //println!("add {:#?} to {:#?}", f.name.name, strct.name);
                    }
                    //println!("{:#?}", constant);
                }
            }

            if !added {
                panic!("internal error: could not added method to spec");
            }
        } else {
            unimplemented!("{:#?}", t.strip_ref());
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

    //println!("f: {} op: {} id: {}", f_name, opcode, symbol.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    Ok(())
}

pub fn compile_const(pkg: &str, c: &Decl<ConstSpec>, compiler: &mut Compiler) -> Result<(), Error> {
    let mut c_iter = c.specs.iter();
    let mut grouped_constants = vec![];
    let mut current_group = vec![];

    while let Some(spec) = c_iter.next() {
        // add the first one that has a init expression
        if current_group.is_empty() {
            assert!(!spec.values.is_empty());
            assert_eq!(spec.name.len(), spec.values.len());
            current_group.push(spec);
            continue;
        }

        // start a new group, if the number of constants changes
        let value_spec = current_group.first().expect("should not happen");
        if spec.name.len() != value_spec.name.len() {
            assert!(!spec.values.is_empty());
            assert_eq!(spec.name.len(), spec.values.len());
            grouped_constants.push(current_group.clone());
            current_group = vec![spec];
            continue;
        }

        // if no values, it should be grouped with the current group
        // otherwise, create a new group
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
            compiler.iota += 1;
        }
    }

    Ok(())
}

pub fn type_interface(pkg: &str, spec: &TypeSpec, it: &InterfaceType, c: &mut Compiler) {
    let mut funcs = Vec::with_capacity(it.methods.list.len());
    let mut func_names = Vec::with_capacity(it.methods.list.len());

    for field in &it.methods.list {
        let func_name = field.name.first().unwrap();
        let (_, _, args, rt) = c
            .expression_to_define_type(pkg, &field.typ)
            .unwrap()
            .as_func();

        funcs.push(DefineType::Func {
            name: func_name.name.to_string(),
            recv: None,
            args,
            rt,
        });
        func_names.push(func_name.name.to_string());
    }

    let s = match c.symbols.resolve(pkg, &spec.name.name) {
        Some(s) => {
            let updated = c.symbols.update_dt(
                pkg,
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
            pkg,
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

pub fn type_spec(pkg: &str, spec: &TypeSpec, c: &mut Compiler) {
    let t = spec.name.clone();
    let inner_t = c.expression_to_define_type(pkg, &spec.typ).unwrap();

    let symbol = match c.symbols.resolve(pkg, &t.name) {
        Some(s) => {
            let s = s.get_symbol();
            let updated = c.symbols.update_dt(
                pkg,
                &t.name,
                DefineType::Spec {
                    name: t.name.to_string(),
                    inner: Box::new(inner_t),
                    methods: vec![],
                    is_transparent: spec.alias,
                },
            );
            assert!(updated);

            s
        }
        None => c.symbols.define(
            pkg,
            &t.name,
            DefineType::Spec {
                name: t.name.to_string(),
                inner: Box::new(inner_t),
                methods: vec![],
                is_transparent: spec.alias,
            },
            false,
        ),
    };

    let obj = Alias::object(t.name.to_string(), Object::null(), vec![], spec.alias);
    let idx = c.add_constant(obj);
    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let opcode = if symbol.scope == Scope::Global {
        OpCode::SetGlobal
    } else {
        OpCode::SetLocal
    };
    //panic!("{}", symbol.index);
    c.emit_opcode(opcode);
    c.emit_u16(symbol.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);
}

pub fn type_struct(pkg: &str, spec: &TypeSpec, ta: &StructType, c: &mut Compiler) {
    let t = spec.name.clone();
    let mut field_types = vec![];

    let mut tags: Vec<Option<String>> = vec![];

    for field in &ta.fields {
        let (inner_t, is_ref) = match &field.typ {
            Expression::TypePointer(p) => (p.typ.as_ident().unwrap(), true),
            _ => (field.typ.as_ident().unwrap(), false),
        };

        if !is_ref && t.name == inner_t.name {
            panic!("recursive definition");
        }

        let r = c.symbols.resolve(pkg, &inner_t.name).unwrap().get_type().0;

        let dt = if is_ref {
            DefineType::Ref(Box::new(r.strip_type()))
        } else {
            r.strip_type()
        };

        let tag_str = field.tag.as_ref().map(|t| t.value.clone());

        if field.name.is_empty() {
            field_types.push(ContextType::Embedded(inner_t.name.clone(), dt.clone()));
            tags.push(tag_str);
        } else {
            for name in &field.name {
                field_types.push(ContextType::Named(
                    name.name.as_str().to_string(),
                    dt.clone(),
                ));
                tags.push(tag_str.clone());
            }
        }
    }

    let ftl = field_types.len();

    let mut field_values = Vec::with_capacity(ftl);

    for _ in 0..ftl {
        field_values.push(Object::null());
    }

    let name = spec.name.name.as_str();

    let symbol = match c.symbols.resolve(pkg, &spec.name.name) {
        Some(s) => s.get_symbol(),
        None => c.symbols.define(
            pkg,
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

    let updated = c
        .symbols
        .update_struct_fields(pkg, name, field_types.clone());

    assert!(updated);

    let obj = Struct::object(name.to_string(), field_values, vec![], tags, false);
    let idx = c.add_constant(obj);
    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let opcode = if symbol.scope == Scope::Global {
        OpCode::SetGlobal
    } else {
        OpCode::SetLocal
    };
    //panic!("{}", symbol.index);
    c.emit_opcode(opcode);
    c.emit_u16(symbol.index);

    c.emit_opcode(OpCode::Const);
    c.emit_u16(idx);

    let strct = c.symbols.resolve(pkg, name).unwrap().get_type().0;

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

                        c.compile_declaration(pkg, &Declaration::Function(gen_m))
                            .unwrap();
                    }
                }
            }
        }
    }
}
