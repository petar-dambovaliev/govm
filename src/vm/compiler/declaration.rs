use crate::parser::ast::{
    ConstSpec, Decl, Expression, FuncDecl, InterfaceType, Statement, StructType, TypeSpec, VarSpec,
};
use crate::vm::compiler::compiler::Compiler;
use crate::vm::compiler::{FuncContext, OpCode, JUMP_PLACEHOLDER};
use crate::vm::object::structure::{Interface, Struct};
use crate::vm::object::{Object, Type};
use crate::vm::symbols::{ContextType, DefineType, Scope};
use crate::vm::Error;

pub fn compile_variable(v: &Decl<VarSpec>, c: &mut Compiler) -> Result<(), Error> {
    for spec in &v.specs {
        let mut declared_tp = None;
        let mut value_is_default = false;
        let values = if spec.values.is_empty() {
            let tp = c.expression_to_define_type(
                spec.typ
                    .as_ref()
                    .expect("no declared values requires a declared type"),
            );

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

            let symbol = c.symbols.define(
                name.name.as_str(),
                DefineType::Var(Box::new(rt.clone())),
                rt.is_invar(),
            );

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
        let t = c.expression_to_define_type(&recv.typ);

        let ret = (
            Compiler::make_method_name(t.strip_ref(), &f.name.name),
            Some(recv),
            Some(Box::new(t)),
        );

        ret
    } else {
        (f.name.name.clone(), None, None)
    };

    let symbol = c.symbols.define(
        &f_name,
        DefineType::Func {
            name: f.name.name.clone(),
            recv: recv_t.clone(),
            args: vec![],
            rt: Box::new(DefineType::Null),
        },
        false,
    );

    let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

    // Compile function in a new scope
    c.symbols.new_context(false);

    if let Some(recv) = recv {
        let t = c.expression_to_define_type(&recv.typ);

        //check if there is a field with the same name
        if let DefineType::Struct { fields, .. } = &t.strip_ref() {
            for field in fields {
                let (field_name, _) = field.as_named().unwrap();
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

    for p in &f.typ.params.list {
        let t = c.expression_to_define_type(&p.typ);
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
        let t = c.expression_to_define_type(&el.typ);
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
        let t = c.expression_to_define_type(&recv.typ);

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
                        ret_type.strip_type(),
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
        let t = c.expression_to_define_type(&recv.typ);
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
                DefineType::Var(Box::new(rt.clone())),
                rt.is_invar(),
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
        let (_, _, args, rt) = c.expression_to_define_type(&field.typ).as_func();

        funcs.push(DefineType::Func {
            name: func_name.name.to_string(),
            recv: None,
            args,
            rt,
        });
        func_names.push(func_name.name.to_string());
    }

    let s = c.symbols.define(
        &spec.name.name.clone(),
        DefineType::Interface {
            name: spec.name.name.clone(),
            methods: funcs,
        },
        false,
    );

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
    let symbol = c.symbols.define(
        name,
        DefineType::Struct {
            name: name.to_string(),
            fields: field_types.clone(),
            methods: vec![],
        },
        false,
    );

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

    let updated = c.symbols.update_dt(
        name,
        DefineType::Struct {
            name: name.to_string(),
            fields: field_types,
            methods: vec![],
        },
    );

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
}
