use crate::parser::ast::InterfaceType;
use crate::parser::ast::{ArrayType, Field};
use crate::parser::ast::{
    AssignStmt, BasicLit, BranchStmt, Call, CompositeLit, Decl, DeclStmt, Declaration, Element,
    ExprStmt, Expression, FieldList, File, Ident, Index, KeyedElement, LiteralValue, Operation,
    Statement, TypeSpec,
};
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::parser::Parser;
use crate::vm::compiler::call::CallType;
use crate::vm::compiler::{
    bytecode_to_human, Bytecode, Context, FuncContext, LoopContext, OpCode, SwitchContext,
    JUMP_PLACEHOLDER,
};

use crate::vm::object::function::Closure;
use crate::vm::object::rune::Rune;
use crate::vm::object::structure::{Interface, Struct, TypeValue};
use crate::vm::object::{is_builtin_const, FromString, Object, Type};
use crate::vm::symbols::{
    is_integer_coerceable_to, is_uint_coerceable_to, ContextType, DefineType, Resolved, Scope,
    SymbolTable,
};
use crate::vm::{builtin, Error};
use ahash::AHashMap;

pub struct Compiler {
    pub(crate) symbols: SymbolTable,
    constants: Vec<Object>,
    pub(crate) instructions: Vec<u8>,
    last_instruction: Option<OpCode>,
    contexts: Vec<Context>,
    func_contexts: Vec<FuncContext>,
    label_contexts: AHashMap<(usize, usize), String>,
    anonymous_struct: usize,
}

impl Compiler {
    /// Create a new compiler
    pub fn new() -> Self {
        Self {
            symbols: SymbolTable::new(),
            instructions: Vec::new(),
            constants: Vec::new(),
            last_instruction: None,
            contexts: Vec::new(),
            func_contexts: Vec::new(),
            label_contexts: AHashMap::new(),
            anonymous_struct: 0,
        }
    }

    /// Compiles the given AST into executable Bytecode
    pub fn compile_ast(&mut self, ast: &File) -> Result<Bytecode, Error> {
        //insert builtin values
        //interface{}
        self.compile_declaration(&Declaration::Type(Decl {
            docs: vec![],
            pos0: 0,
            pos1: None,
            specs: vec![TypeSpec {
                docs: vec![],
                alias: false,
                name: Default::default(),
                params: Default::default(),
                typ: Expression::TypeInterface(InterfaceType {
                    pos: 0,
                    methods: Default::default(),
                }),
            }],
        }))?;

        let s = self.symbols.define(
            "string",
            DefineType::Type(Box::new(DefineType::String), Type::String),
            false,
        );
        let idx = self.add_constant(TypeValue::object(Type::String, None));
        self.emit_opcode(OpCode::Const);
        self.emit_u16(idx);
        self.emit_opcode(OpCode::SetGlobal);
        self.emit_u16(s.index);
        //panic!("{:#?}", self.constants);

        let s = self.symbols.define(
            "bool",
            DefineType::Type(Box::new(DefineType::Bool), Type::Bool),
            false,
        );

        let idx = self.add_constant(TypeValue::object(Type::Bool, None));
        self.emit_opcode(OpCode::Const);
        self.emit_u16(idx);
        self.emit_opcode(OpCode::SetGlobal);
        self.emit_u16(s.index);

        let _ = self.symbols.define(
            "nil",
            DefineType::Type(Box::new(DefineType::Null), Type::Null),
            false,
        );

        self.constants.push(Object::null());

        let _ = self
            .symbols
            .define("_", DefineType::Var(Box::new(DefineType::Null)), false);

        let numbers = vec![
            (
                "int",
                DefineType::Type(Box::new(DefineType::Int), Type::Int),
            ),
            (
                "int8",
                DefineType::Type(Box::new(DefineType::Int8), Type::I8),
            ),
            (
                "int16",
                DefineType::Type(Box::new(DefineType::Int16), Type::I16),
            ),
            (
                "int32",
                DefineType::Type(Box::new(DefineType::Int32), Type::I32),
            ),
            (
                "int64",
                DefineType::Type(Box::new(DefineType::Int64), Type::I64),
            ),
            (
                "uint",
                DefineType::Type(Box::new(DefineType::Uint), Type::UI),
            ),
            (
                "uint8",
                DefineType::Type(Box::new(DefineType::Uint8), Type::UI8),
            ),
            (
                "uint16",
                DefineType::Type(Box::new(DefineType::Uint16), Type::UI16),
            ),
            (
                "uint32",
                DefineType::Type(Box::new(DefineType::Uint32), Type::UI32),
            ),
            (
                "uint64",
                DefineType::Type(Box::new(DefineType::Uint64), Type::UI64),
            ),
            (
                "byte",
                DefineType::Type(Box::new(DefineType::Byte), Type::Byte),
            ),
            (
                "float",
                DefineType::Type(Box::new(DefineType::Float), Type::Float),
            ),
            (
                "float32",
                DefineType::Type(Box::new(DefineType::Float32), Type::Float32),
            ),
            (
                "float64",
                DefineType::Type(Box::new(DefineType::Float64), Type::Float64),
            ),
        ];

        for number in numbers {
            let _ = self.symbols.define(number.0, number.1, false);
            //self.constants.push(Object::int(0));
        }

        let _ = self.symbols.define(
            "rune",
            DefineType::Type(Box::new(DefineType::Rune), Type::Rune),
            false,
        );

        let idx = self.add_constant(Closure::null());
        self.emit_opcode(OpCode::Const);
        self.emit_u16(idx);

        //self.constants.push(Rune::from_char(0 as char));

        // Call compile_statement on each child node directly
        // We don't re-use compile_block_statement here because it exits the global scope
        for s in &ast.decl {
            self.compile_declaration(s)?;
        }

        let entry = Parser::from("main()").expression().unwrap();
        self.compile_expression(&entry)?;

        self.emit_opcode(OpCode::Halt);
        self.instructions.shrink_to_fit();
        self.constants.shrink_to_fit();

        Ok(Bytecode {
            constants: self.constants.clone(),
            instructions: std::mem::take(&mut self.instructions),
        })
    }

    #[inline]
    fn emit_opcode(&mut self, op: OpCode) {
        self.instructions.push(op as u8);
        self.last_instruction = Some(op);
    }

    #[inline]
    fn emit_u8(&mut self, v: u8) {
        self.instructions.push(v)
    }

    #[inline]
    fn emit_u16(&mut self, v: u16) {
        self.instructions.push((v & 0xFF) as u8);
        self.instructions.push(((v >> 8) & 0xFF) as u8);
    }

    #[inline]
    fn change_jump_operand_at(&mut self, idx: usize, v: u16) {
        assert!(
            self.instructions[idx] == OpCode::Jump as u8
                || self.instructions[idx] == OpCode::JumpIfFalse as u8
        );
        self.instructions[idx + 1] = (v & 0xFF) as u8;
        self.instructions[idx + 2] = ((v >> 8) & 0xFF) as u8;
    }

    #[inline]
    fn last_instruction_is(&self, op: OpCode) -> bool {
        self.last_instruction == Some(op)
    }

    #[inline]
    fn remove_last_instruction(&mut self) {
        debug_assert!(self.last_instruction.is_some());
        debug_assert_eq!(self.last_instruction.unwrap().operands().len(), 0);
        self.instructions.pop();
        self.last_instruction = None;
    }

    fn define_type_to_context_type(&self, dts: &[DefineType]) -> Vec<ContextType> {
        let mut decl_arg_types = Vec::with_capacity(dts.len());
        for dt in dts {
            decl_arg_types.push(ContextType::Unnamed(dt.clone()));
        }
        decl_arg_types
    }

    fn field_list_to_define_type(&mut self, fl: &FieldList) -> (DefineType, Vec<DefineType>) {
        let mut decl_r_types = Vec::with_capacity(fl.list.len());

        fn field_to_define_type(c: &mut Compiler, field: &Field) -> DefineType {
            let t = match &field.typ {
                Expression::Ident(id) => c.symbols.resolve(id.name.as_str()).unwrap().get_type(),
                Expression::TypePointer(pt) => {
                    let id = pt.typ.as_ident().unwrap();
                    let t = c.symbols.resolve(id.name.as_str()).unwrap().get_type();
                    DefineType::Ref(Box::new(t))
                }
                Expression::TypeFunction(f) => {
                    let (_, t_vec) = c.field_list_to_define_type(&f.params);
                    let (dt, _) = c.field_list_to_define_type(&f.result);

                    DefineType::Func {
                        name: "".to_string(),
                        recv: None,
                        args: c.define_type_to_context_type(t_vec.as_ref()),
                        rt: Box::new(dt),
                    }
                }
                Expression::TypeStruct(st) => {
                    let fields = st
                        .fields
                        .iter()
                        .map(|f| {
                            ContextType::Named(
                                f.name.first().unwrap().name.clone(),
                                field_to_define_type(c, f),
                            )
                        })
                        .collect();
                    DefineType::Struct {
                        name: format!("anonymous_struct {}", c.anonymous_struct),
                        fields,
                        methods: vec![],
                    }
                }
                _ => panic!("function: unsupported parameter expression: {:#?}", field),
            };
            t
        }

        for el in &fl.list {
            decl_r_types.push(field_to_define_type(self, el));
        }

        let r_t = if decl_r_types.is_empty() {
            DefineType::Null
        } else if decl_r_types.len() == 1 {
            decl_r_types[0].clone()
        } else {
            DefineType::Tuple(decl_r_types.clone())
        };

        (r_t, decl_r_types)
    }

    fn expression_to_define_type(&mut self, expr: &Expression) -> DefineType {
        match expr {
            Expression::Ident(id) => self.symbols.resolve(id.name.as_str()).unwrap().get_type(),
            Expression::TypeFunction(tf) => {
                let (_, args) = self.field_list_to_define_type(&tf.params);
                let (ret, _) = self.field_list_to_define_type(&tf.result);
                DefineType::Func {
                    name: "".to_string(),
                    recv: None,
                    args: self.define_type_to_context_type(args.as_ref()),
                    rt: Box::new(ret),
                }
            }
            Expression::TypePointer(tp) => {
                DefineType::Ref(Box::new(self.expression_to_define_type(&tp.typ)))
            }
            Expression::TypeMap(map) => {
                let k = self.expression_to_define_type(map.key.as_ref());
                let v = self.expression_to_define_type(map.val.as_ref());

                // if k.is_type() {
                //     k = k.as_type().0;
                // }
                //
                // if v.is_type() {
                //     v = v.as_type().0;
                // }

                DefineType::Map(Box::new(k), Box::new(v))
            }
            Expression::Invar(invar) => DefineType::Invar(Box::new(
                self.expression_to_define_type(invar.expr.as_ref()),
            )),
            Expression::TypeInterface(i) => {
                assert!(i.methods.list.is_empty());
                DefineType::Interface {
                    name: "".to_string(),
                    methods: vec![],
                }
            }
            Expression::TypeArray(ta) => {
                let inner = self.expression_to_define_type(&ta.typ);
                let len = ta.len.as_int_lit().unwrap();
                DefineType::Array {
                    inner_type: Box::new(inner),
                    len: len as usize,
                }
            }
            Expression::TypeSlice(ts) => {
                let inner = self.expression_to_define_type(&ts.typ);
                DefineType::Slice(Box::new(inner))
            }
            Expression::Ellipsis(variadic) => {
                let inner = self.expression_to_define_type(variadic.elt.as_ref().unwrap().as_ref());
                DefineType::Variadic(Box::new(inner))
            }
            Expression::TypeStruct(st) => {
                let mut fields = Vec::with_capacity(st.fields.len());
                for field in &st.fields {
                    fields.push(ContextType::Named(
                        field.name.first().unwrap().name.clone(),
                        self.expression_to_define_type(&field.typ),
                    ));
                }
                DefineType::Struct {
                    name: format!("anonymous_struct {}", self.anonymous_struct),
                    fields,
                    methods: vec![],
                }
            }
            _ => panic!("expression_to_define_type: unsupported expr {:#?}", expr),
        }
    }

    fn compile_declaration(&mut self, decl: &Declaration) -> Result<(), Error> {
        //println!("{:#?}", decl);
        match decl {
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    let mut declared_tp = None;
                    let mut value_is_default = false;
                    let values = if spec.values.is_empty() {
                        let tp = self.expression_to_define_type(
                            spec.typ
                                .as_ref()
                                .expect("no declared values requires a declared type"),
                        );
                        declared_tp = Some(tp.clone());
                        let mut defaults = Vec::with_capacity(spec.name.len());
                        for _ in 0..spec.name.len() {
                            defaults.push(self.make_type_default_val(tp.clone()));
                        }
                        value_is_default = true;
                        defaults
                    } else {
                        spec.values.clone()
                    };

                    for (name, value) in spec.name.iter().zip(values.iter()) {
                        let mut rt = self.compile_expression(value)?;

                        if rt.is_invar() {
                            panic!("var cant be invar");
                        }

                        if let Some(dtp) = &declared_tp {
                            if value_is_default && dtp.is_nullable() {
                                self.emit_opcode(OpCode::TypedNull);

                                let mut found_closure = false;
                                for (ind, constant) in self.constants.iter().enumerate() {
                                    if constant.tag() == Type::Closure {
                                        let c = constant.as_closure();
                                        if c.is_null {
                                            self.emit_u16(ind.try_into().unwrap());
                                            found_closure = true;
                                            break;
                                        }
                                    }
                                }
                                if !found_closure {
                                    panic!("could not find closure constant");
                                }
                            }

                            if rt.is_nil() && (dtp.is_ref() || dtp.is_func()) {
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

                        let symbol = self.symbols.define(
                            name.name.as_str(),
                            DefineType::Var(Box::new(rt.clone())),
                            rt.is_invar(),
                        );

                        if should_upcast {
                            if let Some(dtp) = &declared_tp {
                                let (name, _) = dtp.as_interface();
                                let (s, _) = self.symbols.resolve(&name).unwrap().as_local();

                                self.emit_opcode(OpCode::Upcast);
                                self.emit_u16(s.index);
                            }
                        }

                        let op = if symbol.scope == Scope::Global {
                            OpCode::SetGlobal
                        } else {
                            OpCode::SetLocal
                        };
                        self.emit_opcode(op);
                        self.emit_u16(symbol.index);
                    }
                }
            }
            Declaration::Function(f) => {
                let pos_jump = self.instructions.len();

                self.func_contexts.push(FuncContext::new(pos_jump));

                self.emit_opcode(OpCode::Jump);
                self.emit_u16(JUMP_PLACEHOLDER);

                let (f_name, recv, recv_t) = if let Some(recv) = f.recv.as_ref() {
                    let recv = recv.list.first().unwrap();
                    let t = self.expression_to_define_type(&recv.typ);

                    let ret = (
                        Self::make_method_name(t.strip_ref(), &f.name.name),
                        Some(recv),
                        Some(Box::new(t)),
                    );

                    ret
                } else {
                    (f.name.name.clone(), None, None)
                };

                let symbol = self.symbols.define(
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
                self.symbols.new_context(false);

                if let Some(recv) = recv {
                    let t = self.expression_to_define_type(&recv.typ);

                    //check if there is a field with the same name
                    if let DefineType::Struct { fields, .. } = &t.strip_ref() {
                        for field in fields {
                            let (field_name, _) = field.as_named().unwrap();
                            if field_name == f.name.name {
                                panic!("field and method with the same name {}", field_name);
                            }
                        }
                    }

                    self.symbols.define(
                        &recv.name.first().unwrap().name,
                        DefineType::Var(Box::new(t.clone())),
                        t.is_invar(),
                    );
                }

                for p in &f.typ.params.list {
                    let t = self.expression_to_define_type(&p.typ);
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
                    let t = self.expression_to_define_type(&el.typ);
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
                let updated = self.symbols.update_dt(&f_name, func_def.clone());
                assert!(updated);

                self.func_contexts.last_mut().unwrap().expected_ret = r_t;

                //add method to struct symbol
                if let Some(recv) = recv {
                    let t = self.expression_to_define_type(&recv.typ);

                    let (r_name, r_fields, mut r_methods) = self
                        .symbols
                        .resolve(&t.get_type_name())
                        .unwrap()
                        .get_type()
                        .as_struct()
                        .unwrap();

                    r_methods.push(func_def.clone());
                    let updated = self.symbols.update_dt(
                        &r_name,
                        DefineType::Struct {
                            name: r_name.to_string(),
                            fields: r_fields,
                            methods: r_methods,
                        },
                    );
                    println!("update");
                    assert!(updated);
                }

                let pos_start_function = self.instructions.len();

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
                    terminates = self.compile_block_statement(&body.list)?;
                } else {
                    //todo
                    // assert if the function is void but there is a return
                    //assert_eq!(rts.is_none());
                }
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

                    //println!("{:#?}", un_rts);
                    if (!terminates.unwrap_or_default()
                        && expected_t != DefineType::Null
                        && !has_top_return)
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

                //add method start position for dynamic dispatch
                if let Some(recv) = recv {
                    let t = self.expression_to_define_type(&recv.typ);
                    let mut added = false;

                    if let DefineType::Struct { name, .. } = &t.strip_ref() {
                        for constant in &mut self.constants {
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
            }
            Declaration::Const(c) => {
                for spec in &c.specs {
                    for (name, value) in spec.name.iter().zip(spec.values.iter()) {
                        let rt = self.compile_expression(value)?;

                        let symbol = self.symbols.define(
                            name.name.as_str(),
                            DefineType::Var(Box::new(rt.clone())),
                            rt.is_invar(),
                        );

                        let op = if symbol.scope == Scope::Global {
                            OpCode::SetGlobal
                        } else {
                            OpCode::SetLocal
                        };
                        self.emit_opcode(op);
                        self.emit_u16(symbol.index);
                    }
                }
            }
            Declaration::Type(t) => {
                for spec in &t.specs {
                    if !spec.alias {
                        let t = spec.name.clone();

                        match &spec.typ {
                            Expression::TypeInterface(it) => {
                                let mut funcs = Vec::with_capacity(it.methods.list.len());
                                let mut func_names = Vec::with_capacity(it.methods.list.len());

                                for field in &it.methods.list {
                                    let func_name = field.name.first().unwrap();
                                    let (_, _, args, rt) =
                                        self.expression_to_define_type(&field.typ).as_func();

                                    funcs.push(DefineType::Func {
                                        name: func_name.name.to_string(),
                                        recv: None,
                                        args,
                                        rt,
                                    });
                                    func_names.push(func_name.name.to_string());
                                }

                                let s = self.symbols.define(
                                    &spec.name.name.clone(),
                                    DefineType::Interface {
                                        name: spec.name.name.clone(),
                                        methods: funcs,
                                    },
                                    false,
                                );

                                let obj = Interface::object(
                                    spec.name.name.clone(),
                                    func_names,
                                    Object::null(),
                                );
                                let idx = self.add_constant(obj);
                                self.emit_opcode(OpCode::Const);
                                self.emit_u16(idx);

                                let opcode = if s.scope == Scope::Global {
                                    OpCode::SetGlobal
                                } else {
                                    OpCode::SetLocal
                                };
                                self.emit_opcode(opcode);
                                self.emit_u16(s.index);

                                self.emit_opcode(OpCode::Const);
                                self.emit_u16(idx);
                            }
                            Expression::TypeStruct(ta) => {
                                let mut field_types = vec![];

                                //todo tags
                                for field in &ta.fields {
                                    let (inner_t, is_ref) = match &field.typ {
                                        Expression::TypePointer(p) => {
                                            (p.typ.as_ident().unwrap(), true)
                                        }
                                        _ => (field.typ.as_ident().unwrap(), false),
                                    };

                                    if !is_ref && t.name == inner_t.name {
                                        panic!("recursive definition");
                                    }

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

                                let name = spec.name.name.as_str();
                                let symbol = self.symbols.define(
                                    name,
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
                                    name,
                                    DefineType::Struct {
                                        name: name.to_string(),
                                        fields: field_types,
                                        methods: vec![],
                                    },
                                );

                                assert!(updated);

                                let obj =
                                    Struct::object(name.to_string(), field_values, vec![], false);
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
                            }
                            _ => unimplemented!("{:#?}", spec),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compile_block_statement(&mut self, block: &[Statement]) -> Result<Option<bool>, Error> {
        // if block statement does not contain any other statements or expressions
        // simply push a NULL onto the stack
        if block.is_empty() {
            //self.emit_opcode(OpCode::Null);
            return Ok(Some(false));
        }

        self.symbols.enter_scope();
        let mut terminates = true;
        let mut last_term = false;

        for s in block {
            let term = self.compile_statement(s)?;

            let is_empty = if let Statement::Empty(_) = s {
                true
            } else {
                false
            };

            if last_term
                && !is_empty
                && self.func_contexts.last().unwrap().expected_ret != DefineType::Null
            {
                panic!("deadcode: {:#?}", s);
            }

            if let Some(te) = term {
                if !te {
                    terminates = false;
                } else {
                    last_term = true;
                }
            }
            //println!("statement: {:#?}", s);
            //println!("rt: {:#?}", rt);
        }

        self.symbols.leave_scope();

        Ok(Some(terminates))
    }

    pub(crate) fn compile_statement(&mut self, stmt: &Statement) -> Result<Option<bool>, Error> {
        match stmt {
            Statement::For(forstmt) => {
                self.emit_opcode(OpCode::Null);

                self.symbols.enter_scope();
                let label = self.label_contexts.get(&(forstmt.pos, 0)).cloned();

                if let Some(init) = &forstmt.init {
                    self.compile_statement(init.as_ref())?;
                }

                self.contexts.push(Context::For(LoopContext::new(
                    self.instructions.len(),
                    label,
                )));

                let pos_before_condition = self.instructions.len();

                let cond = forstmt
                    .cond
                    .clone()
                    .unwrap_or(Box::from(Statement::Expr(ExprStmt {
                        expr: Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Ident,
                            value: "true".to_string(),
                        }),
                    })));

                self.compile_statement(cond.as_ref())?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                //self.emit_opcode(OpCode::Pop);

                let terminate = self.compile_block_statement(&forstmt.body.list)?;

                let mut post_op_pos = 0;
                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                    if let Some(post) = &forstmt.post {
                        post_op_pos = self.instructions.len();
                        self.compile_statement(post.as_ref())?;
                    }
                } else {
                    if let Some(post) = &forstmt.post {
                        post_op_pos = self.instructions.len();
                        self.compile_statement(post.as_ref())?;
                    }
                    //self.emit_opcode(OpCode::Null);
                }

                // emit jump instruction to loop condition
                self.emit_opcode(OpCode::Jump);
                self.emit_u16(pos_before_condition.try_into().unwrap());

                // Update jump statement for when initial condition evaluated to false (should skip over entire loop)
                self.change_jump_operand_at(
                    pos_jump_if_false,
                    self.instructions.len().try_into().unwrap(),
                );

                // Update jump statements for every break statement inside this loop
                let ctx = self.contexts.pop().unwrap().to_for();
                for ip in &ctx.break_instructions {
                    self.change_jump_operand_at(*ip, self.instructions.len().try_into().unwrap());
                }

                for ip in &ctx.continue_instructions {
                    self.change_jump_operand_at(*ip, post_op_pos.try_into().unwrap());
                }

                let loop_terminates = (terminate.unwrap_or_default()
                    || forstmt.body.list.is_empty())
                    && forstmt.cond.is_none()
                    && ctx.break_instructions.is_empty();

                self.symbols.leave_scope();

                return Ok(Some(loop_terminates));
            }
            Statement::If(ifstmt) => {
                if let Some(init) = &ifstmt.init {
                    self.compile_statement(init.as_ref())?;
                }

                self.compile_expression(&ifstmt.cond)?;
                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);

                let terminates = self.compile_block_statement(&ifstmt.body.list)?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump = self.instructions.len();
                self.emit_opcode(OpCode::Jump);
                self.emit_u16(JUMP_PLACEHOLDER);

                self.change_jump_operand_at(
                    pos_jump_if_false,
                    self.instructions.len().try_into().unwrap(),
                );

                let mut else_terminates = None;

                if let Some(alternative) = &ifstmt.else_ {
                    match alternative.as_ref() {
                        Statement::Block(bl) => {
                            else_terminates = self.compile_block_statement(&bl.list)?;
                        }
                        Statement::If(_elseif) => {
                            self.compile_statement(alternative.as_ref())?;
                        }
                        _ => panic!("else should be a block: {:#?}", alternative),
                    }

                    if self.last_instruction_is(OpCode::Pop) {
                        self.remove_last_instruction();
                    }
                } else {
                    self.emit_opcode(OpCode::Null);
                }

                // Change operand of last JumpIfFalse opcode to where we're currently at
                self.change_jump_operand_at(pos_jump, self.instructions.len().try_into().unwrap());

                let terminates =
                    terminates.unwrap_or_default() && else_terminates.unwrap_or_default();

                return Ok(Some(terminates));
            }
            Statement::Assign(assign) => {
                if assign.left.len() > 1 && assign.right.len() == 1 {
                    let first = assign.right.first().unwrap();
                    let dt = self.compile_expression(first)?;

                    let (ret, is_type_assert) = match dt {
                        DefineType::Func { rt: ret, .. } => (ret, Some(false)),
                        DefineType::Tuple(_) => {
                            if let Expression::Index(_) = first {
                                (Box::new(dt.clone()), None)
                            } else {
                                //assert_eq!(2, assign.left.len());
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

                                let symbol = self.symbols.define(
                                    name.as_str(),
                                    DefineType::Var(Box::new(ct.clone())),
                                    ct.is_invar(),
                                );

                                if is_type_assert.unwrap_or_default() && i == assign.left.len() - 1
                                {
                                    let def_expr = self.make_type_default_val(ct.clone());
                                    self.compile_expression(&def_expr)?;
                                    self.emit_opcode(OpCode::SetDefault);
                                }

                                let op = if symbol.scope == Scope::Global {
                                    OpCode::SetGlobal
                                } else {
                                    OpCode::SetLocal
                                };
                                self.emit_opcode(op);
                                self.emit_u16(symbol.index);
                            }
                            Operator::Assign => 'assign: {
                                let name = match &left {
                                    Expression::Ident(name) => name.name.as_str(),
                                    _ => {
                                        return Err(Error::TypeError(format!(
                                            "cannot assign a value to expressions of type {:?}",
                                            left
                                        )))
                                    }
                                };

                                //todo emit pop for the right expr
                                if name == "_" {
                                    break 'assign;
                                }

                                let resolved =
                                    self.symbols.resolve(name).ok_or(Error::ReferenceError(
                                        format!("assign: `{name}` is not defined"),
                                    ))?;

                                let (index, setop) = match resolved {
                                    Resolved::Enclosed((s, _)) => (s.index, OpCode::SetCaptured),
                                    Resolved::Local((symbol, _)) => match symbol.scope {
                                        Scope::Local => (symbol.index, OpCode::SetLocal),
                                        Scope::Global => (symbol.index, OpCode::SetGlobal),
                                    },
                                };

                                self.emit_opcode(setop);
                                self.emit_u16(index);
                            }
                            _ => unimplemented!(),
                        }
                        i += 1;
                    }
                    return Ok(None);
                }

                assert_eq!(assign.left.len(), assign.right.len());

                for (left, right) in assign.left.iter().zip(assign.right.iter()) {
                    match &assign.op {
                        Operator::AddAssign => {
                            self.compile_statement(&Statement::Assign(AssignStmt {
                                pos: 0,
                                op: Operator::Assign,
                                left: vec![left.clone()],
                                right: vec![Expression::Operation(Operation {
                                    pos: 0,
                                    op: Operator::Add,
                                    x: Box::new(left.clone()),
                                    y: Some(Box::new(right.clone())),
                                })],
                            }))?;
                        }
                        Operator::Define => {
                            let name = match left {
                                Expression::Ident(ident) => &ident.name,
                                _ => panic!("only identifiers can be defined: {:#?}", left),
                            };

                            let mut rt = self.compile_expression(right)?;

                            rt = match rt {
                                //type assertion
                                // s := i.(string)
                                DefineType::Tuple(tuple) => {
                                    assert_eq!(2, tuple.len());
                                    if let Expression::Index(_) = right {
                                        self.emit_opcode(OpCode::Pop);
                                    } else {
                                        self.emit_opcode(OpCode::PanicIfFalse);
                                    }
                                    tuple[0].clone()
                                }
                                _ => rt,
                            };

                            let symbol = self.symbols.define(
                                name.as_str(),
                                DefineType::Var(Box::new(rt.clone())),
                                rt.is_invar(),
                            );

                            let op = if symbol.scope == Scope::Global {
                                OpCode::SetGlobal
                            } else {
                                OpCode::SetLocal
                            };
                            self.emit_opcode(op);
                            self.emit_u16(symbol.index);
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
                                    let _t = self.compile_expression(ind.left.as_ref())?;
                                    self.compile_expression(ind.index.as_ref())?;
                                    self.compile_expression(right)?;
                                    self.emit_opcode(OpCode::IndexSet);
                                    return Ok(None);
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
                                _ => {
                                    return Err(Error::TypeError(format!(
                                        "cannot assign a value to expressions of type {:?}",
                                        left
                                    )))
                                }
                            };

                            //todo pop expression if its not assigned to anything
                            if name == "_" {
                                break 'assign;
                            }

                            let resolved = self.symbols.resolve(&name).ok_or(
                                Error::ReferenceError(format!("assign: `{name}` is not defined")),
                            )?;

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

                                        let i = i.unwrap();

                                        self.compile_expression(&Expression::Ident(Ident {
                                            pos: 0,
                                            name,
                                        }))?;
                                        self.compile_expression(&Expression::BasicLit(BasicLit {
                                            pos: 0,
                                            kind: LitKind::Integer,
                                            value: format!("{}", i),
                                        }))?;
                                        self.compile_expression(right)?;
                                        self.emit_opcode(OpCode::IndexSet);
                                        return Ok(None);
                                    }
                                    _ => unimplemented!("{:#?}", t),
                                }
                            }

                            let (index, setop, expect_t) = match resolved {
                                Resolved::Enclosed((s, t)) => {
                                    let write_op = if is_deref {
                                        OpCode::EnclosedPtrWrite
                                    } else {
                                        OpCode::SetCaptured
                                    };
                                    (s.index, write_op, t.strip_var())
                                }
                                Resolved::Local((symbol, mut t)) => match symbol.scope {
                                    Scope::Local => {
                                        t = t.strip_var();
                                        let write_op = if is_deref || t.is_func() {
                                            OpCode::LocalPtrWrite
                                        } else {
                                            OpCode::SetLocal
                                        };
                                        (symbol.index, write_op, t)
                                    }
                                    Scope::Global => {
                                        let write_op = if is_deref || t.is_func() {
                                            OpCode::GlobalPtrWrite
                                        } else {
                                            OpCode::SetGlobal
                                        };
                                        (symbol.index, write_op, t)
                                    }
                                },
                            };

                            let got_t = self.compile_expression(right)?;

                            if is_deref {
                                if expect_t.is_invar() {
                                    panic!("cannot write to an invar reference");
                                }
                                //panic!("name: {:#?} type: {:#?}", name, expect_t);
                                assert!(expect_t.is_ref());
                                let inner = expect_t.as_ref().strip_type();
                                assert_eq!(inner, got_t);
                            } else {
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

                            self.emit_opcode(setop);
                            self.emit_u16(index);
                        }
                        _ => unimplemented!(),
                    }
                }
                return Ok(None);
            }
            Statement::Expr(expr) => {
                self.compile_expression(&expr.expr)?;
                if !self.last_instruction_is(OpCode::Pop) {
                    self.emit_opcode(OpCode::Pop);
                }
            }
            Statement::Block(stmts) => {
                return self.compile_block_statement(&stmts.list);
            }
            Statement::Declaration(declr) => match declr {
                DeclStmt::Type(t) => {
                    self.compile_declaration(&Declaration::Type(t.clone()))?;
                }
                DeclStmt::Const(t) => {
                    self.compile_declaration(&Declaration::Const(t.clone()))?;
                }
                DeclStmt::Variable(t) => {
                    self.compile_declaration(&Declaration::Variable(t.clone()))?;
                }
            },
            Statement::Return(expr) => {
                let mut rts = Vec::with_capacity(expr.ret.len());

                for r in &expr.ret {
                    let t = self.compile_expression(&r)?;
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

                let expect_t = self.func_contexts.last().unwrap().expected_ret.clone();

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
                    self.emit_opcode(OpCode::PanicIfFalse);
                }

                self.func_contexts
                    .last_mut()
                    .unwrap()
                    .ret_types
                    .push((rt, is_type_assert));

                assert!(rts_len < u16::MAX as usize);
                self.emit_opcode(OpCode::ReturnValue);
                self.emit_u16(rts_len as u16);

                return Ok(Some(true));
            }
            Statement::Branch(branch) => match branch.key {
                Keyword::Break => {
                    self.emit_opcode(OpCode::Null);
                    let pos = self.instructions.len();
                    self.emit_opcode(OpCode::Jump);
                    self.emit_u16(JUMP_PLACEHOLDER);

                    if let Some(l) = branch.ident.clone() {
                        for ctx in self.contexts.iter_mut().rev() {
                            if let Some(label) = ctx.label() {
                                if &l.name == label {
                                    ctx.push_break(pos);
                                    return Ok(Some(false));
                                }
                            }
                        }
                        panic!("label not found: {:#?}", branch.ident);
                    } else {
                        let ctx = match self.contexts.last_mut() {
                            Some(ctx) => ctx,
                            None => return Err(Error::SyntaxError("bad call 1".to_string())),
                        };
                        ctx.push_break(pos);
                    }
                }
                Keyword::Continue => {
                    self.emit_opcode(OpCode::Null);
                    let pos = self.instructions.len();
                    self.emit_opcode(OpCode::Jump);
                    self.emit_u16(JUMP_PLACEHOLDER);

                    if let Some(l) = branch.ident.clone() {
                        for ctx in self.contexts.iter_mut().rev() {
                            if let Some(label) = ctx.label() {
                                if &l.name == label {
                                    ctx.push_break(pos);
                                    return Ok(Some(false));
                                }
                            }
                        }
                        panic!("label not found: {:#?}", branch.ident);
                    } else {
                        let ctx = match self.contexts.last_mut() {
                            Some(ctx) => ctx,
                            None => return Err(Error::SyntaxError("bad call 1".to_string())),
                        };
                        ctx.push_continue(pos);
                    }
                }
                Keyword::FallThrough => {
                    // already handled in the switch logic
                    // do nothing
                }
                _ => panic!("key: {:#?}", branch.key),
            },
            Statement::IncDec(incdec) => {
                let name = match &incdec.expr {
                    Expression::Ident(ident) => ident.clone(),
                    _ => panic!("only ident allowed inc/dec"),
                };

                let r = self.symbols.resolve(&name.name).unwrap();

                let (index, setop, incop, t) = match r {
                    Resolved::Enclosed((s, t)) => {
                        (s.index, OpCode::GetCaptured, OpCode::IncLocal, t)
                    }
                    Resolved::Local((symbol, t)) => match symbol.scope {
                        Scope::Local => (symbol.index, OpCode::SetLocal, OpCode::IncLocal, t),
                        Scope::Global => (symbol.index, OpCode::SetGlobal, OpCode::IncGlobal, t),
                    },
                };

                //todo
                // self.emit_opcode(setop);
                // self.emit_u16(index);

                if !t.strip_var().is_numeric() {
                    panic!("cannot use inc/dec operators on {:#?}", t);
                }

                self.emit_opcode(incop);
                self.emit_u16(index);
            }
            Statement::Empty(_) => {}
            Statement::Range(rng) => {
                self.emit_opcode(OpCode::Null);

                let label = self.label_contexts.get(&rng.pos).cloned();
                self.contexts.push(Context::For(LoopContext::new(
                    self.instructions.len(),
                    label,
                )));
                let iter_sym;

                // __iter__ := into_iter X
                let _iter_ident = Expression::Ident(Ident {
                    pos: 0,
                    name: "__iter__".to_string(),
                });
                {
                    let name = "__iter__";
                    iter_sym = self.symbols.define(
                        name,
                        DefineType::Var(Box::new(DefineType::Null)),
                        false,
                    );
                    self.compile_expression(&rng.expr)?;
                    self.emit_opcode(OpCode::IntoIter);
                    self.emit_opcode(OpCode::SetLocal);
                    self.emit_u16(iter_sym.index);
                }

                let pos_before_condition = self.instructions.len();

                // k, v := range __iter__
                let key = rng.key.clone().unwrap_or(Expression::Ident(Ident {
                    pos: 0,
                    name: "_".to_string(),
                }));
                let value = rng.value.clone().unwrap_or(Expression::Ident(Ident {
                    pos: 0,
                    name: "_".to_string(),
                }));
                match (&key, &value) {
                    (Expression::Ident(key_id), Expression::Ident(value_id)) => {
                        let key_symbol = self.symbols.define(
                            key_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
                            false,
                        );
                        let value_symbol = self.symbols.define(
                            value_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
                            false,
                        );

                        self.emit_opcode(OpCode::GetLocal);
                        self.emit_u16(iter_sym.index);

                        self.emit_opcode(OpCode::Range);
                        self.emit_u16(key_symbol.index);
                        self.emit_u16(value_symbol.index);
                    }
                    _ => panic!("invalid"),
                }

                self.compile_statement(&Statement::Expr(ExprStmt {
                    expr: Expression::Operation(Operation {
                        pos: 0,
                        op: Operator::NotEqual,
                        x: Box::new(Expression::Operation(Operation {
                            pos: 0,
                            op: Operator::And,
                            x: Box::new(key),
                            y: None,
                        })),
                        y: Some(Box::new(Expression::Ident(Ident {
                            pos: 0,
                            name: "nil".to_string(),
                        }))),
                    }),
                }))?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                //self.emit_opcode(OpCode::Pop);

                self.compile_block_statement(&rng.body.list)?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                } else {
                    self.emit_opcode(OpCode::Null);
                }

                // emit jump instruction to loop condition
                self.emit_opcode(OpCode::Jump);
                self.emit_u16(pos_before_condition.try_into().unwrap());

                // Update jump statement for when initial condition evaluated to false (should skip over entire loop)
                self.change_jump_operand_at(
                    pos_jump_if_false,
                    self.instructions.len().try_into().unwrap(),
                );

                // Update jump statements for every break statement inside this loop
                let ctx = self.contexts.pop().unwrap().to_for();
                for ip in ctx.break_instructions {
                    self.change_jump_operand_at(ip, self.instructions.len().try_into().unwrap());
                }
                return Ok(Some(false));
            }
            Statement::Label(lstmt) => {
                if self.symbols.resolve(lstmt.name.name.as_str()).is_some() {
                    panic!("label already defined: {:#?}", lstmt.name.name);
                }

                let pos = match lstmt.stmt.as_ref() {
                    Statement::For(f) => f.pos,
                    Statement::Switch(sw) => sw.pos,
                    _ => panic!("expected for or switch statement"),
                };

                self.label_contexts
                    .insert((pos, 0), lstmt.name.name.clone());
                let r = self.compile_statement(lstmt.stmt.as_ref());
                self.label_contexts.remove(&(pos, 0));

                return r;
            }
            Statement::Switch(switch) => {
                let label = self.label_contexts.get(&(switch.pos, 0)).cloned();
                self.contexts.push(Context::Switch(SwitchContext::new(
                    self.instructions.len(),
                    label,
                )));

                if let Some(init) = &switch.init {
                    self.compile_statement(&init)?;
                }
                let internal_tag = Ident {
                    pos: 0,
                    name: "__tag__".to_string(),
                };
                let tag = switch.tag.clone().unwrap_or(Expression::Ident(Ident {
                    pos: 0,
                    name: "nil".to_string(),
                }));

                self.compile_statement(&Statement::Assign(AssignStmt {
                    pos: 0,
                    op: Operator::Define,
                    left: vec![Expression::Ident(internal_tag.clone())],
                    right: vec![tag.clone()],
                }))?;

                let mut terminates = true;
                let mut has_default = false;

                for clause in &switch.block.body {
                    match clause.tok {
                        Keyword::Default => {
                            if has_default {
                                panic!("only one default allowed within a switch");
                            }
                            has_default = true;
                            let cond = Expression::BasicLit(BasicLit {
                                pos: 0,
                                kind: LitKind::Ident,
                                value: "true".to_string(),
                            });

                            self.compile_expression(&cond)?;

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            }

                            let pos_jump_if_false = self.instructions.len();
                            self.emit_opcode(OpCode::JumpIfFalse);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            terminates = terminates
                                && self
                                    .compile_block_statement(&clause.body)?
                                    .unwrap_or_default();

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            } else {
                                self.emit_opcode(OpCode::Null);
                            }

                            let pos_jump = self.instructions.len();
                            self.emit_opcode(OpCode::Jump);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            self.change_jump_operand_at(
                                pos_jump_if_false,
                                self.instructions.len().try_into().unwrap(),
                            );

                            self.change_jump_operand_at(
                                pos_jump,
                                self.instructions.len().try_into().unwrap(),
                            );
                        }
                        Keyword::Case => {
                            for expr in &clause.list {
                                let cond = match expr {
                                    Expression::Ident(id) => {
                                        assert!(switch.tag.is_some());
                                        Expression::Operation(Operation {
                                            pos: 0,
                                            op: Operator::Equal,
                                            x: Box::new(Expression::Ident(internal_tag.clone())),
                                            y: Some(Box::new(Expression::Ident(id.clone()))),
                                        })
                                    }
                                    Expression::BasicLit(bl) => {
                                        assert!(switch.tag.is_some());
                                        Expression::Operation(Operation {
                                            pos: 0,
                                            op: Operator::Equal,
                                            x: Box::new(Expression::Ident(internal_tag.clone())),
                                            y: Some(Box::new(Expression::BasicLit(bl.clone()))),
                                        })
                                    }
                                    _ => {
                                        assert!(switch.tag.is_none());
                                        expr.clone()
                                    }
                                };
                                //println!("{:#?}", cond);
                                self.compile_expression(&cond)?;

                                if self.last_instruction_is(OpCode::Pop) {
                                    self.remove_last_instruction();
                                }

                                let pos_jump_if_false = self.instructions.len();
                                self.emit_opcode(OpCode::JumpIfFalse);
                                self.emit_u16(JUMP_PLACEHOLDER);

                                let mut has_fallthrough = false;
                                let bl = clause.body.len();
                                for (i, stmt) in clause.body.iter().enumerate() {
                                    let is_fallthrough = if let Statement::Branch(br) = stmt {
                                        br.key == Keyword::FallThrough
                                    } else {
                                        false
                                    };
                                    if is_fallthrough {
                                        if i == bl - 1 {
                                            has_fallthrough = true;
                                        } else {
                                            panic!("misplaced fallthrough");
                                        }
                                    }
                                }

                                let mut clause_body = clause.body.clone();

                                if !has_fallthrough {
                                    clause_body.push(Statement::Branch(BranchStmt {
                                        pos: 0,
                                        key: Keyword::Break,
                                        ident: None,
                                    }));
                                }

                                terminates = terminates
                                    && self
                                        .compile_block_statement(&clause_body)?
                                        .unwrap_or_default();

                                if self.last_instruction_is(OpCode::Pop) {
                                    self.remove_last_instruction();
                                } else {
                                    self.emit_opcode(OpCode::Null);
                                }

                                let pos_jump = self.instructions.len();
                                self.emit_opcode(OpCode::Jump);
                                self.emit_u16(JUMP_PLACEHOLDER);

                                self.change_jump_operand_at(
                                    pos_jump_if_false,
                                    self.instructions.len().try_into().unwrap(),
                                );

                                self.change_jump_operand_at(
                                    pos_jump,
                                    self.instructions.len().try_into().unwrap(),
                                );
                            }
                        }
                        _ => unimplemented!(),
                    }
                }

                let ctx = self.contexts.pop().unwrap().to_switch();

                for ip in &ctx.break_instructions {
                    self.change_jump_operand_at(*ip, self.instructions.len().try_into().unwrap());
                }

                return Ok(Some(terminates));
            }
            Statement::TypeSwitch(switch) => {
                let label = self.label_contexts.get(&(switch.pos, 0)).cloned();
                self.contexts.push(Context::Switch(SwitchContext::new(
                    self.instructions.len(),
                    label,
                )));

                if let Some(init) = &switch.init {
                    self.compile_statement(&init)?;
                }

                let internal_tag = Ident {
                    pos: 0,
                    name: "__tag__".to_string(),
                };

                let (mut left_ass, mut left_ass_type) = (None, None);

                match switch.tag.clone().map(|a| *a) {
                    Some(Statement::Expr(expr)) => {
                        self.compile_statement(&Statement::Assign(AssignStmt {
                            pos: 0,
                            op: Operator::Define,
                            left: vec![Expression::Ident(internal_tag.clone())],
                            right: vec![expr.expr],
                        }))?;
                    }
                    Some(Statement::Assign(ass)) => {
                        let left = ass.left.first().unwrap().as_ident().unwrap().clone();
                        left_ass = Some(left.clone());

                        self.compile_statement(&Statement::Assign(ass))?;
                        self.compile_statement(&Statement::Assign(AssignStmt {
                            pos: 0,
                            op: Operator::Define,
                            left: vec![Expression::Ident(internal_tag.clone())],
                            right: vec![Expression::Ident(left.clone())],
                        }))?;
                        left_ass_type = Some(self.symbols.resolve(&left.name).unwrap().get_type());
                    }
                    Some(_) => unreachable!(),
                    None => unreachable!(),
                }

                let mut terminates = true;
                let mut has_default = false;

                for clause in &switch.block.body {
                    match clause.tok {
                        Keyword::Default => {
                            if has_default {
                                panic!("only one default allowed within a switch");
                            }

                            if let (Some(lat), Some(la)) = (&left_ass_type, &left_ass) {
                                let updated = self.symbols.update_dt(&la.name, lat.clone());
                                assert!(updated);
                            }

                            has_default = true;
                            let cond = Expression::BasicLit(BasicLit {
                                pos: 0,
                                kind: LitKind::Ident,
                                value: "true".to_string(),
                            });

                            self.compile_expression(&cond)?;

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            }

                            let pos_jump_if_false = self.instructions.len();
                            self.emit_opcode(OpCode::JumpIfFalse);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            terminates = terminates
                                && self
                                    .compile_block_statement(&clause.body)?
                                    .unwrap_or_default();

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            } else {
                                self.emit_opcode(OpCode::Null);
                            }

                            let pos_jump = self.instructions.len();
                            self.emit_opcode(OpCode::Jump);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            self.change_jump_operand_at(
                                pos_jump_if_false,
                                self.instructions.len().try_into().unwrap(),
                            );

                            self.change_jump_operand_at(
                                pos_jump,
                                self.instructions.len().try_into().unwrap(),
                            );
                        }
                        Keyword::Case => {
                            for expr in &clause.list {
                                match expr {
                                    Expression::Ident(id) => {
                                        // in each case the tag identifier has to be updated to the case's asserted type
                                        // only if there is a single clause in the case
                                        if clause.list.len() == 1 {
                                            if let Some(la) = &left_ass {
                                                let r = self
                                                    .symbols
                                                    .resolve(&id.name)
                                                    .unwrap()
                                                    .get_type();

                                                let updated = self.symbols.update_dt(
                                                    &la.name,
                                                    DefineType::Var(Box::new(r)),
                                                );
                                                assert!(updated);
                                            }
                                        }

                                        assert!(switch.tag.is_some());
                                        self.compile_expression(&Expression::Ident(
                                            internal_tag.clone(),
                                        ))?;
                                        self.compile_expression(&Expression::Ident(id.clone()))?;
                                        self.emit_opcode(OpCode::TypeCmp);
                                    }
                                    Expression::TypePointer(pt) => {
                                        let id = pt.typ.as_ident().unwrap();

                                        // in each case the tag identifier has to be updated to the case's asserted type
                                        // only if there is a single clause in the case
                                        if clause.list.len() == 1 {
                                            if let Some(la) = &left_ass {
                                                let r = self
                                                    .symbols
                                                    .resolve(&id.name)
                                                    .unwrap()
                                                    .get_type();

                                                let updated = self.symbols.update_dt(
                                                    &la.name,
                                                    DefineType::Var(Box::new(r)),
                                                );

                                                assert!(updated);
                                            }
                                        }

                                        self.compile_expression(&Expression::Ident(
                                            internal_tag.clone(),
                                        ))?;
                                        self.compile_expression(&Expression::Ident(id.clone()))?;
                                        self.emit_opcode(OpCode::Ref);
                                        self.emit_opcode(OpCode::TypeCmp);
                                    }
                                    _ => {
                                        assert!(switch.tag.is_none());
                                        self.compile_expression(&expr)?;
                                    }
                                };

                                if self.last_instruction_is(OpCode::Pop) {
                                    self.remove_last_instruction();
                                }

                                let pos_jump_if_false = self.instructions.len();
                                self.emit_opcode(OpCode::JumpIfFalse);
                                self.emit_u16(JUMP_PLACEHOLDER);

                                let mut has_fallthrough = false;
                                let bl = clause.body.len();
                                for (i, stmt) in clause.body.iter().enumerate() {
                                    let is_fallthrough = if let Statement::Branch(br) = stmt {
                                        br.key == Keyword::FallThrough
                                    } else {
                                        false
                                    };
                                    if is_fallthrough {
                                        if i == bl - 1 {
                                            has_fallthrough = true;
                                        } else {
                                            panic!("misplaced fallthrough");
                                        }
                                    }
                                }

                                let mut clause_body = clause.body.clone();

                                if !has_fallthrough {
                                    clause_body.push(Statement::Branch(BranchStmt {
                                        pos: 0,
                                        key: Keyword::Break,
                                        ident: None,
                                    }));
                                }

                                terminates = terminates
                                    && self
                                        .compile_block_statement(&clause_body)?
                                        .unwrap_or_default();

                                if self.last_instruction_is(OpCode::Pop) {
                                    self.remove_last_instruction();
                                } else {
                                    self.emit_opcode(OpCode::Null);
                                }

                                let pos_jump = self.instructions.len();
                                self.emit_opcode(OpCode::Jump);
                                self.emit_u16(JUMP_PLACEHOLDER);

                                self.change_jump_operand_at(
                                    pos_jump_if_false,
                                    self.instructions.len().try_into().unwrap(),
                                );

                                self.change_jump_operand_at(
                                    pos_jump,
                                    self.instructions.len().try_into().unwrap(),
                                );
                            }
                        }
                        _ => unimplemented!(),
                    }
                }

                let ctx = self.contexts.pop().unwrap().to_switch();

                for ip in &ctx.break_instructions {
                    self.change_jump_operand_at(*ip, self.instructions.len().try_into().unwrap());
                }

                return Ok(Some(terminates));
            }
            _ => {
                return Err(Error::ReferenceError(format!(
                    "stmt not supported: {:#?}",
                    stmt
                )))
            }
        }

        Ok(None)
    }

    fn compile_operator(&mut self, operator: &Operator) {
        let opcode = match operator {
            Operator::Add => OpCode::Add,
            Operator::Sub => OpCode::Subtract,
            Operator::Quo => OpCode::Divide,
            Operator::Star => OpCode::Multiply,
            Operator::Greater => OpCode::Gt,
            Operator::GreaterEqual => OpCode::Gte,
            Operator::Less => OpCode::Lt,
            Operator::LessEqual => OpCode::Lte,
            Operator::Equal => OpCode::Eq,
            Operator::NotEqual => OpCode::Neq,
            Operator::Rem => OpCode::Modulo,
            Operator::Not => OpCode::Not,
            // its not clear if `-` is negate or minus
            // need to add more context where it is
            //Operator::Negate => OpCode::Negate,
            Operator::And => OpCode::And,
            Operator::Or => OpCode::Or,
            Operator::OrOr => OpCode::Or,
            Operator::AndAnd => OpCode::And,
            _ => panic!("unexpected operator of type {operator:?}"),
        };
        self.emit_opcode(opcode);
    }

    fn make_method_name(dt: DefineType, f_name: &str) -> String {
        format!("0x{:#?}{}", dt, f_name)
    }

    fn compile_const_var_infix_expression(
        &mut self,
        varname: &str,
        const_value: isize,
        operator: &Operator,
    ) -> Result<DefineType, Error> {
        let idx_constant = self.add_constant(Object::int(const_value));
        let (symbol, _) = self
            .symbols
            .resolve(varname)
            .ok_or(Error::ReferenceError(format!("{varname} is not defined")))?
            .as_local();

        let opcode = match (operator, symbol.scope) {
            (Operator::Add, Scope::Local) => OpCode::AddLocalConst,
            (Operator::Sub, Scope::Local) => OpCode::SubtractLocalConst,
            (Operator::Less, Scope::Local) => OpCode::LtLocalConst,
            (Operator::LessEqual, Scope::Local) => OpCode::LteLocalConst,
            (Operator::Greater, Scope::Local) => OpCode::GtLocalConst,
            (Operator::GreaterEqual, Scope::Local) => OpCode::GteLocalConst,
            (Operator::Equal, Scope::Local) => OpCode::EqLocalConst,
            (Operator::NotEqual, Scope::Local) => OpCode::NeqLocalConst,
            // (Operator::Multiply, Scope::Local) => OpCode::MultiplyLocalConst,
            // (Operator::Divide, Scope::Local) => OpCode::DivideLocalConst,
            // (Operator::Modulo, Scope::Local) => OpCode::ModuloLocalConst,
            _ => {
                // This is just for other part of compiler to signal it should emit a normal instruction sequence
                return Err(Error::ReferenceError(
                    "Optimized variant of this operator & scope type is not yet implemented."
                        .to_string(),
                ));
            }
        };

        self.emit_opcode(opcode);
        self.emit_u16(symbol.index);
        self.emit_u16(idx_constant);

        Ok(DefineType::Int)
    }

    fn make_type_default_val(&mut self, t: DefineType) -> Expression {
        match t {
            DefineType::Type(inner, _) => self.make_type_default_val(*inner),
            DefineType::String => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::String,
                value: "".to_string(),
            }),
            DefineType::Rune => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Char,
                value: "".to_string(),
            }),
            DefineType::Int
            | DefineType::Int8
            | DefineType::Int16
            | DefineType::Int32
            | DefineType::Int64
            | DefineType::Uint
            | DefineType::Uint8
            | DefineType::Uint16
            | DefineType::Uint32
            | DefineType::Uint64
            | DefineType::Byte => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "0".to_string(),
            }),
            //todo interface
            DefineType::Ref(_)
            | DefineType::Func { .. }
            | DefineType::Map(_, _)
            | DefineType::Null
            | DefineType::Slice(_) => Expression::Ident(Ident {
                pos: 0,
                name: "nil".to_string(),
            }),

            DefineType::Array { len, inner_type } => {
                let mut keyed_elements = Vec::with_capacity(len);

                for i in 0..len {
                    let el = KeyedElement {
                        key: Some(Element::Expr(Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Integer,
                            value: format!("{}", i),
                        }))),
                        val: Element::Expr(self.make_type_default_val(*inner_type.clone())),
                    };
                    keyed_elements.push(el);
                }

                Expression::CompositeLit(CompositeLit {
                    typ: Box::new(Expression::TypeArray(ArrayType {
                        pos: (0, 0),
                        len: Box::new(Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Integer,
                            value: format!("{}", len),
                        })),
                        typ: Box::new(inner_type.clone().to_expression()),
                    })),
                    val: LiteralValue {
                        pos: (0, 0),
                        values: keyed_elements,
                    },
                })
            }
            DefineType::Bool => Expression::Ident(Ident {
                pos: 0,
                name: "false".to_string(),
            }),
            DefineType::Float | DefineType::Float32 | DefineType::Float64 => {
                Expression::BasicLit(BasicLit {
                    pos: 0,
                    kind: LitKind::Float,
                    value: "0.0".to_string(),
                })
            }
            DefineType::Struct {
                name: n,
                fields: inner_types,
                ..
            } => {
                let mut lit_val = LiteralValue {
                    pos: (0, 0),
                    values: vec![],
                };

                for inner_type in inner_types {
                    let (key, it) = inner_type.as_named().unwrap();
                    let ex = self.make_type_default_val(it);

                    lit_val.values.push(KeyedElement {
                        key: Some(Element::Expr(Expression::Ident(Ident {
                            pos: 0,
                            name: key,
                        }))),
                        val: Element::Expr(ex),
                    });
                }

                let expr = Expression::CompositeLit(CompositeLit {
                    typ: Box::new(Expression::Ident(Ident {
                        pos: 0,
                        name: n.clone(),
                    })),
                    val: lit_val,
                });
                expr
            }
            _ => unimplemented!("make_type_default_val: {:#?}", t),
        }
    }

    #[allow(unused)]
    fn typecheck_call_func_sig(&mut self, call: &Call) -> Result<(), Error> {
        let f_name = if let Expression::Selector(sel) = call.func.as_ref() {
            let sellt = self
                .symbols
                .resolve(&sel.x.as_ident().unwrap().name)
                .unwrap()
                .get_type()
                .strip_var();

            Self::make_method_name(sellt, &sel.sel.name)
        } else {
            call.func.as_ident().unwrap().name.to_string()
        };

        let mut dt = self.symbols.resolve(&f_name).unwrap().get_type();

        if let DefineType::Var(inner) = &dt {
            if inner.is_func() {
                dt = *inner.clone();
            }
        }

        if !dt.is_func() {
            return Err(Error::SyntaxError(format!(
                "tried to call not a function: {:#?}",
                dt
            )));
        }

        let arg_types = match dt {
            DefineType::Func {
                args: arg_types, ..
            } => arg_types,
            _ => unreachable!(),
        };

        assert_eq!(arg_types.len(), call.args.len());
        Ok(())
    }

    fn compile_expression(&mut self, expr: &Expression) -> Result<DefineType, Error> {
        match expr {
            //todo this is a total mess: fix me
            Expression::Call(call) => {
                //todo typecheck return and args on builtins
                if let Expression::Ident(name) = call.func.as_ref() {
                    if let Some(builtin) = builtin::resolve(&name.name) {
                        let mut first = None;
                        for a in &call.args {
                            let t = self.compile_expression(a)?;
                            if first.is_none() {
                                first = Some(t);
                            }
                        }

                        let is_void = builtin.is_void();
                        self.emit_opcode(OpCode::CallBuiltin);
                        self.emit_u8(builtin as u8);
                        self.emit_u8(call.args.len().try_into().unwrap());

                        if is_void {
                            //panic!("{:#?}", 123);
                            self.emit_opcode(OpCode::Pop);
                        }
                        return Ok(first.unwrap());
                    }
                }

                let ct = CallType::from_call(&call, self);

                let rt = match ct {
                    CallType::Func { func_dt, .. } => {
                        let (_, _, mut arg_types, rts) = func_dt.as_func();
                        let rts = rts.type_to_val_t();
                        //println!("{:#?}", arg_types);
                        //assert_eq!(arg_types.len(), call.args.len());
                        let (is_variadic, variadic_len) = if let Some(last) =
                            arg_types.last().cloned()
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
                            let got = self.compile_expression(a)?;
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
                        let (_, _, arg_types, rts) = method_dt.as_func();
                        let rts = rts.type_to_val_t();
                        assert_eq!(arg_types.len(), call.args.len());

                        //here we do automatic passing by reference
                        // if the signature of the function is by ref
                        // and our value is not we emit a ref opcode
                        let got = self.compile_expression(&struct_expr)?;
                        if struct_dt.is_ref() && !got.is_ref() {
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
                let rt = self.expression_to_define_type(expr);
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
                                self.compile_expression(op.x.as_ref())?;
                                self.compile_expression(y.as_ref())?;
                                self.compile_operator(&op.op);

                                return Ok(DefineType::Int);
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
                                self.compile_expression(op.x.as_ref())?;
                                self.compile_expression(y.as_ref())?;
                                self.compile_operator(&op.op);

                                return Ok(DefineType::Int);
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
                let obj = Object::float(lit.value.parse().unwrap());
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(DefineType::Float);
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

                return Ok(DefineType::Int);
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

                    let slice_t = self.expression_to_define_type(ta.typ.as_ref());
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
                    self.emit_opcode(OpCode::Array);
                    self.emit_u16(clit.val.values.len().try_into().unwrap());
                    self.emit_u16(1);
                    return Ok(DefineType::Array {
                        inner_type: Box::new(slice_t),
                        len: clit.val.values.len(),
                    });
                }

                //struct
                if let Expression::Ident(name) = clit.typ.as_ref() {
                    //todo this can be locally defined type
                    let (s, dt) = match self.symbols.resolve(name.name.as_str()) {
                        Some(s) => s.as_local(),
                        None => panic!("struct `{}` does not exist", name.name),
                    };

                    let (name, inner_types) = match dt {
                        DefineType::Struct { name, fields, .. } => (name, fields),
                        _ => panic!("expect struct"),
                    };

                    if let Some(ct) = inner_types.first() {
                        let _ = ct.as_named().unwrap();
                    }

                    let opcode = if s.scope == Scope::Global {
                        OpCode::GetGlobal
                    } else {
                        OpCode::GetLocal
                    };
                    self.emit_opcode(opcode);
                    self.emit_u16(s.index);

                    //sort by order of definition
                    let mut clit_values = clit.val.values.clone();

                    let key_required = clit
                        .val
                        .values
                        .first()
                        .map(|a| a.key.is_some())
                        .unwrap_or_default();

                    if key_required {
                        clit_values.sort_by_key(|val| {
                            inner_types
                                .iter()
                                .map(|inner_type| inner_type.as_named().unwrap())
                                .position(|x| {
                                    assert_eq!(key_required, val.key.is_some(), "val: {:#?}", val);
                                    let k_el = val.key.as_ref().unwrap();
                                    let k = match k_el {
                                        Element::Expr(expr) => expr.as_ident().unwrap().clone(),
                                        _ => panic!("ident"),
                                    };

                                    x.0 == k.name.as_str()
                                })
                        });
                    } else {
                        assert_eq!(inner_types.len(), clit_values.len());

                        for (clit_value, ct) in clit_values.iter_mut().zip(inner_types.clone()) {
                            clit_value.key = Some(Element::Expr(Expression::Ident(Ident {
                                pos: 0,
                                name: ct.as_named().unwrap().0,
                            })));
                        }
                    }

                    for inner_type in inner_types.iter().rev() {
                        let (kk, inner_type) = inner_type.as_named().unwrap();

                        let found = clit_values.iter().find(|a| {
                            let k = a.key.as_ref().unwrap();

                            let id = match k {
                                Element::Expr(expr) => expr.clone(),
                                _ => panic!("expr"),
                            }
                            .as_ident()
                            .unwrap()
                            .clone();
                            id.name == kk
                        });

                        match found {
                            Some(kel) => {
                                //compile values
                                let el_expr = match &kel.val {
                                    Element::Expr(expr) => expr.clone(),
                                    _ => panic!("expr"),
                                };

                                let rt = self.compile_expression(&el_expr)?;

                                let in_t = match inner_type {
                                    DefineType::Type(a, _b) => *a,
                                    _ => inner_type.clone(),
                                };

                                if !(in_t.is_nullable() && rt.is_nil()) {
                                    assert_eq!(in_t, rt.strip_var().strip_type(), "{:#?}", el_expr);
                                }
                            }
                            None => {
                                let def_val = self.make_type_default_val(inner_type);
                                let _ = self.compile_expression(&def_val)?;
                            }
                        }
                    }

                    // let obj = Object::string(name.clone(), &mut self.gc);
                    // let idx = self.add_constant(obj);
                    // self.emit_opcode(OpCode::Const);
                    // self.emit_u16(idx);

                    self.emit_opcode(OpCode::Struct);
                    self.emit_u16(inner_types.len().try_into().unwrap());
                    return Ok(DefineType::Struct {
                        name,
                        fields: inner_types,
                        methods: vec![],
                    });
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
                            self.expression_to_define_type(ta.typ.as_ref())
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
                    self.emit_opcode(OpCode::Array);
                    self.emit_u16(clit.val.values.len().try_into().unwrap());
                    self.emit_u16(0);

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
                        k => unimplemented!("{:#?}", k),
                    }
                }

                let i = ind.index.as_int_lit().unwrap() as usize;

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

                        Ok(dt)
                    }
                    Some(Resolved::Enclosed((s, t))) => {
                        // enclosed symbols cannot be global
                        self.emit_opcode(OpCode::GetCaptured);
                        self.emit_u16(s.index);

                        Ok(t)
                    }
                    None => Err(Error::ReferenceError(format!(
                        "ident: `{}` is not defined",
                        ident.name
                    ))),
                };
            }
            Expression::Selector(sel) => {
                let name = sel.x.as_ident().unwrap();
                let (_, dt) = self.symbols.resolve(name.name.as_str()).unwrap().as_local();
                let inner = match dt {
                    DefineType::Var(inner) => *inner,
                    _ => panic!("{:#?}", dt),
                };

                let (_, inner_types) = match inner.strip_ref() {
                    DefineType::Struct {
                        name,
                        fields: inner_types,
                        ..
                    } => (name, inner_types),
                    _ => panic!("{:#?}", inner),
                };

                for (i, inner_type) in inner_types.into_iter().enumerate() {
                    let (key, dt) = inner_type.as_named().unwrap();
                    if key == sel.sel.name {
                        self.compile_expression(&Expression::Index(Index {
                            pos: (0, 0),
                            left: Box::new(Expression::Ident(name.clone())),
                            index: Box::new(Expression::BasicLit(BasicLit {
                                pos: 0,
                                kind: LitKind::Integer,
                                value: format!("{}", i),
                            })),
                        }))?;
                        return Ok(dt);
                    }
                }

                panic!("cannot find field");
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
                    let t = self.expression_to_define_type(&p.typ);
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
                    let t = self.expression_to_define_type(&el.typ);
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
                let rt = self.expression_to_define_type(expr);
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
                self.emit_u8(index);
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

    fn add_constant(&mut self, obj: Object) -> u16 {
        // re-use already defined constants
        // if let Some(pos) = self
        //     .constants
        //     .iter()
        //     .position(|c| c.tag() == obj.tag() && c == &obj)
        // {
        //     return pos.try_into().unwrap();
        // }

        let idx = self.constants.len();
        self.constants.push(obj);
        idx.try_into().unwrap()
    }
}
