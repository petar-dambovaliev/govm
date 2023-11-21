use crate::parser::ast::{ArrayType, Field};
use crate::parser::ast::{
    AssignStmt, BasicLit, BranchStmt, Call, CompositeLit, Decl, DeclStmt, Declaration, Element,
    ExprStmt, Expression, FieldList, Ident, KeyedElement, LiteralValue, Operation, Statement,
    TypeSpec,
};
use crate::parser::ast::{InterfaceType, Package};
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::parser::Parser;
use crate::vm::compiler::call::CallType;
use crate::vm::compiler::{
    literal, make_method_name, Bytecode, Context, FuncContext, LoopContext, OpCode, SwitchContext,
    JUMP_PLACEHOLDER,
};
use std::io::BufWriter;
use std::path::PathBuf;

use crate::vm::compiler::declaration::type_spec;
use crate::vm::compiler::declaration::{
    compile_const, compile_function, compile_variable, type_interface, type_struct,
};
use crate::vm::compiler::dep_graph::{make_init_dep_graph, make_package_dep_graph};
use crate::vm::object::function::Closure;
use crate::vm::object::rune::Rune;
use crate::vm::object::structure::{Struct, TypeValue};
use crate::vm::object::{is_builtin_const, FromString, Object, Type};
use crate::vm::symbols::{
    is_integer_coerceable_to, is_uint_coerceable_to, ContextType, DefineType, Resolved, Scope,
    SymbolTable,
};
use crate::vm::{builtin, Error};
use ahash::AHashMap;

pub struct Compiler {
    pub(crate) symbols: SymbolTable,
    pub(crate) constants: Vec<Object>,
    pub(crate) instructions: Vec<u8>,
    last_instruction: Option<OpCode>,
    pub(crate) contexts: Vec<Context>,
    pub(crate) func_contexts: Vec<FuncContext>,
    pub(crate) label_contexts: AHashMap<(usize, usize), String>,
    anonymous_struct: usize,
    pub(crate) iota: usize,
}

const BUILTIN: &str = "0xbuiltin";

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
            iota: 0,
        }
    }

    /// Compiles the given AST into executable Bytecode
    pub fn compile(
        &mut self,
        main: PathBuf,
        project_path: PathBuf,
        project: Vec<Package>,
        output_assert: bool,
    ) -> Result<Bytecode, Error> {
        let pkg = project_path
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        //insert builtin values
        //interface{}
        self.compile_declaration(
            &pkg,
            &Declaration::Type(Decl {
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
            }),
        )?;

        let s = self.symbols.define(
            BUILTIN,
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
            BUILTIN,
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
            BUILTIN,
            "nil",
            DefineType::Type(Box::new(DefineType::Null), Type::Null),
            false,
        );

        self.constants.push(Object::null());

        let _ = self.symbols.define(
            BUILTIN,
            "_",
            DefineType::Var(Box::new(DefineType::Null)),
            false,
        );

        let numbers = vec![
            (
                "int",
                DefineType::Type(Box::new(DefineType::Int), Type::Int),
                Type::Int,
            ),
            (
                "int8",
                DefineType::Type(Box::new(DefineType::Int8), Type::I8),
                Type::I8,
            ),
            (
                "int16",
                DefineType::Type(Box::new(DefineType::Int16), Type::I16),
                Type::I16,
            ),
            (
                "int32",
                DefineType::Type(Box::new(DefineType::Int32), Type::I32),
                Type::I32,
            ),
            (
                "int64",
                DefineType::Type(Box::new(DefineType::Int64), Type::I64),
                Type::I64,
            ),
            (
                "uint",
                DefineType::Type(Box::new(DefineType::Uint), Type::UI),
                Type::UI,
            ),
            (
                "uint8",
                DefineType::Type(Box::new(DefineType::Uint8), Type::UI8),
                Type::UI8,
            ),
            (
                "uint16",
                DefineType::Type(Box::new(DefineType::Uint16), Type::UI16),
                Type::UI16,
            ),
            (
                "uint32",
                DefineType::Type(Box::new(DefineType::Uint32), Type::UI32),
                Type::UI32,
            ),
            (
                "uint64",
                DefineType::Type(Box::new(DefineType::Uint64), Type::UI64),
                Type::UI64,
            ),
            (
                "byte",
                DefineType::Type(Box::new(DefineType::Byte), Type::Byte),
                Type::Byte,
            ),
            (
                "float32",
                DefineType::Type(Box::new(DefineType::Float32), Type::Float32),
                Type::Float32,
            ),
            (
                "float64",
                DefineType::Type(Box::new(DefineType::Float64), Type::Float64),
                Type::Float64,
            ),
        ];

        for number in numbers {
            let s = self.symbols.define(BUILTIN, number.0, number.1, false);

            let idx = self.add_constant(TypeValue::object(number.2, None));
            self.emit_opcode(OpCode::Const);
            self.emit_u16(idx);
            self.emit_opcode(OpCode::SetGlobal);
            self.emit_u16(s.index);
        }

        let _ = self.symbols.define(
            BUILTIN,
            "rune",
            DefineType::Type(Box::new(DefineType::Rune), Type::Rune),
            false,
        );

        let idx = self.add_constant(Closure::null());
        self.emit_opcode(OpCode::Const);
        self.emit_u16(idx);

        //self.constants.push(Rune::from_char(0 as char));
        let (pkgs_graph, pkgs_map) = make_package_dep_graph(project);

        let mut adb = None;

        for pkg_id in pkgs_graph.into_iter() {
            let pkg = pkgs_map.get(&pkg_id).unwrap();

            for file in &pkg.files {
                if file.pkg_name.name == "main" {
                    let mut is_output = false;
                    for comment in file.comments.clone() {
                        if output_assert
                            && !is_output
                            && comment
                                .text
                                .to_lowercase()
                                .trim_start_matches("//")
                                .trim_start()
                                == "output:"
                        {
                            is_output = true;
                        } else if is_output {
                            adb = match adb.as_mut() {
                                None => {
                                    Some(format!("{}\n", comment.text.trim_start_matches("//")))
                                }
                                Some(ss) => {
                                    ss.push_str(&format!(
                                        "{}\n",
                                        comment.text.trim_start_matches("//")
                                    ));
                                    Some(ss.clone())
                                }
                            }
                        }
                    }
                }

                for import in &file.imports {
                    let import_path = import.path.value.trim_matches('"');
                    let p: PathBuf = import_path.clone().into();
                    let p = project_path.join(p);

                    let alias = import.name.clone().map(|id| id.name.clone()).unwrap_or(
                        p.file_name()
                            .map(|f| f.to_str().unwrap())
                            .unwrap()
                            .to_string(),
                    );

                    self.symbols.define(
                        "",
                        &alias,
                        DefineType::Package {
                            path: p
                                .canonicalize()
                                .expect(&format!("cannot canonicalize: {:#?}", p))
                                .to_str()
                                .expect(&format!("cannot to_str: {:#?}", p))
                                .to_string(),
                            alias: alias.clone(),
                        },
                        false,
                    );
                }

                let cur_pkg = pkg
                    .path
                    .canonicalize()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string();
                let (graph, map_declr) = make_init_dep_graph(&cur_pkg, &file.decl, self);

                for declr_id in graph.into_iter() {
                    self.compile_declaration(&cur_pkg, map_declr.get(&declr_id).unwrap())?;
                }
            }
        }

        let entry = Parser::from("main()").expression().unwrap();
        self.compile_expression(
            &main.canonicalize().unwrap().to_str().unwrap().to_string(),
            &entry,
        )?;

        self.emit_opcode(OpCode::Halt);
        self.instructions.shrink_to_fit();
        self.constants.shrink_to_fit();

        Ok(Bytecode {
            constants: self.constants.clone(),
            instructions: std::mem::take(&mut self.instructions),
            assert_stdout: adb.map(|a| (a, BufWriter::new(vec![]))),
        })
    }

    #[inline]
    pub(crate) fn emit_opcode(&mut self, op: OpCode) {
        self.instructions.push(op as u8);
        self.last_instruction = Some(op);
    }

    #[inline]
    pub(crate) fn emit_u8(&mut self, v: u8) {
        self.instructions.push(v)
    }

    #[inline]
    pub(crate) fn emit_u16(&mut self, v: u16) {
        self.instructions.push((v & 0xFF) as u8);
        self.instructions.push(((v >> 8) & 0xFF) as u8);
    }

    #[inline]
    pub(crate) fn change_jump_operand_at(&mut self, idx: usize, v: u16) {
        assert!(
            self.instructions[idx] == OpCode::Jump as u8
                || self.instructions[idx] == OpCode::JumpIfFalse as u8
        );
        self.instructions[idx + 1] = (v & 0xFF) as u8;
        self.instructions[idx + 2] = ((v >> 8) & 0xFF) as u8;
    }

    #[inline]
    pub(crate) fn last_instruction_is(&self, op: OpCode) -> bool {
        self.last_instruction == Some(op)
    }

    #[inline]
    pub(crate) fn remove_last_instruction(&mut self) {
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

    fn field_list_to_define_type(
        &mut self,
        pkg: &str,
        fl: &FieldList,
    ) -> (DefineType, Vec<DefineType>) {
        let mut decl_r_types = Vec::with_capacity(fl.list.len());

        fn field_to_define_type(c: &mut Compiler, pkg: &str, field: &Field) -> DefineType {
            let t = match &field.typ {
                Expression::Ident(id) => {
                    c.symbols
                        .resolve(pkg, id.name.as_str())
                        .unwrap()
                        .get_type()
                        .0
                }
                Expression::TypePointer(pt) => {
                    let id = pt.typ.as_ident().unwrap();
                    let t = c.symbols.resolve(pkg, id.name.as_str()).unwrap().get_type();
                    DefineType::Ref(Box::new(t.0))
                }
                Expression::TypeFunction(f) => {
                    let (_, t_vec) = c.field_list_to_define_type(pkg, &f.params);
                    let (dt, _) = c.field_list_to_define_type(pkg, &f.result);

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
                                field_to_define_type(c, pkg, f),
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
            decl_r_types.push(field_to_define_type(self, pkg, el));
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

    pub(crate) fn expression_to_define_type(
        &mut self,
        pkg: &str,
        expr: &Expression,
    ) -> Option<DefineType> {
        match expr {
            Expression::Ident(id) => {
                Some(self.symbols.resolve(pkg, id.name.as_str())?.get_type().0)
            }
            Expression::TypeFunction(tf) => {
                let (_, args) = self.field_list_to_define_type(pkg, &tf.params);
                let (ret, _) = self.field_list_to_define_type(pkg, &tf.result);
                Some(DefineType::Func {
                    name: "".to_string(),
                    recv: None,
                    args: self.define_type_to_context_type(args.as_ref()),
                    rt: Box::new(ret),
                })
            }
            Expression::TypePointer(tp) => Some(DefineType::Ref(Box::new(
                self.expression_to_define_type(pkg, &tp.typ)?,
            ))),
            Expression::TypeMap(map) => {
                let k = self.expression_to_define_type(pkg, map.key.as_ref())?;
                let v = self.expression_to_define_type(pkg, map.val.as_ref())?;

                // if k.is_type() {
                //     k = k.as_type().0;
                // }
                //
                // if v.is_type() {
                //     v = v.as_type().0;
                // }

                Some(DefineType::Map(Box::new(k), Box::new(v)))
            }
            Expression::Invar(invar) => Some(DefineType::Invar(Box::new(
                self.expression_to_define_type(pkg, invar.expr.as_ref())?,
            ))),
            Expression::TypeInterface(i) => {
                assert!(i.methods.list.is_empty());
                Some(DefineType::Interface {
                    name: "".to_string(),
                    methods: vec![],
                })
            }
            Expression::TypeArray(ta) => {
                let inner = self.expression_to_define_type(pkg, &ta.typ)?;
                let len = ta.len.as_int_lit().unwrap();
                Some(DefineType::Array {
                    inner_type: Box::new(inner),
                    len: len as usize,
                })
            }
            Expression::TypeSlice(ts) => {
                let inner = self.expression_to_define_type(pkg, &ts.typ)?;
                Some(DefineType::Slice(Box::new(inner)))
            }
            Expression::Ellipsis(variadic) => {
                let inner =
                    self.expression_to_define_type(pkg, variadic.elt.as_ref().unwrap().as_ref())?;
                Some(DefineType::Variadic(Box::new(inner)))
            }
            Expression::TypeStruct(st) => {
                let mut fields = Vec::with_capacity(st.fields.len());
                for field in &st.fields {
                    fields.push(ContextType::Named(
                        field.name.first().unwrap().name.clone(),
                        self.expression_to_define_type(pkg, &field.typ)?,
                    ));
                }
                Some(DefineType::Struct {
                    name: format!("anonymous_struct {}", self.anonymous_struct),
                    fields,
                    methods: vec![],
                })
            }
            _ => panic!("expression_to_define_type: unsupported expr {:#?}", expr),
        }
    }

    pub(crate) fn compile_declaration(
        &mut self,
        pkg: &str,
        decl: &Declaration,
    ) -> Result<(), Error> {
        match decl {
            Declaration::Variable(v) => {
                compile_variable(pkg, v, self)?;
            }
            Declaration::Function(f) => {
                compile_function(pkg, f, self)?;
            }
            Declaration::Const(c) => {
                compile_const(pkg, c, self)?;
            }
            Declaration::Type(t) => {
                for spec in &t.specs {
                    match &spec.typ {
                        Expression::TypeInterface(it) => {
                            type_interface(pkg, spec, it, self);
                        }
                        Expression::TypeStruct(ta) => {
                            if spec.alias {
                                unimplemented!("type aliases");
                            }
                            type_struct(pkg, spec, ta, self);
                        }
                        Expression::Ident(_id) => {
                            type_spec(pkg, spec, self);
                        }
                        _ => unimplemented!("{:#?}", spec),
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn compile_block_statement(
        &mut self,
        pkg: &str,
        block: &[Statement],
    ) -> Result<Option<bool>, Error> {
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
            let term = self.compile_statement(pkg, s)?;

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

    pub(crate) fn compile_statement(
        &mut self,
        pkg: &str,
        stmt: &Statement,
    ) -> Result<Option<bool>, Error> {
        match stmt {
            Statement::For(forstmt) => {
                self.emit_opcode(OpCode::Null);

                self.symbols.enter_scope();
                let label = self.label_contexts.get(&(forstmt.pos, 0)).cloned();

                if let Some(init) = &forstmt.init {
                    self.compile_statement(pkg, init.as_ref())?;
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

                self.compile_statement(pkg, cond.as_ref())?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                //self.emit_opcode(OpCode::Pop);

                let terminate = self.compile_block_statement(pkg, &forstmt.body.list)?;

                let mut post_op_pos = 0;
                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                    if let Some(post) = &forstmt.post {
                        post_op_pos = self.instructions.len();
                        self.compile_statement(pkg, post.as_ref())?;
                    }
                } else {
                    if let Some(post) = &forstmt.post {
                        post_op_pos = self.instructions.len();
                        self.compile_statement(pkg, post.as_ref())?;
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
                    self.compile_statement(pkg, init.as_ref())?;
                }

                self.compile_expression(pkg, &ifstmt.cond)?;
                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);

                let terminates = self.compile_block_statement(pkg, &ifstmt.body.list)?;

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
                            else_terminates = self.compile_block_statement(pkg, &bl.list)?;
                        }
                        Statement::If(_elseif) => {
                            self.compile_statement(pkg, alternative.as_ref())?;
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
                    let dt = self.compile_expression(pkg, first)?;

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
                                    pkg,
                                    name.as_str(),
                                    DefineType::Var(Box::new(ct.clone())),
                                    ct.is_invar(),
                                );

                                if is_type_assert.unwrap_or_default() && i == assign.left.len() - 1
                                {
                                    let def_expr = self.make_type_default_val(ct.clone());
                                    self.compile_expression(pkg, &def_expr)?;
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

                                let resolved = self.symbols.resolve(pkg, name).ok_or(
                                    Error::ReferenceError(format!(
                                        "assign: `{name}` is not defined"
                                    )),
                                )?;

                                let (index, setop) = match resolved {
                                    Resolved::Enclosed((s, _, _)) => (s.index, OpCode::SetCaptured),
                                    Resolved::Local((symbol, _, _)) => match symbol.scope {
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
                            self.compile_statement(
                                pkg,
                                &Statement::Assign(AssignStmt {
                                    pos: 0,
                                    op: Operator::Assign,
                                    left: vec![left.clone()],
                                    right: vec![Expression::Operation(Operation {
                                        pos: 0,
                                        op: Operator::Add,
                                        x: Box::new(left.clone()),
                                        y: Some(Box::new(right.clone())),
                                    })],
                                }),
                            )?;
                        }
                        Operator::Define => {
                            let name = match left {
                                Expression::Ident(ident) => &ident.name,
                                _ => panic!("only identifiers can be defined: {:#?}", left),
                            };

                            let mut rt = self.compile_expression(pkg, right)?;

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
                                pkg,
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
                                    let _t = self.compile_expression(pkg, ind.left.as_ref())?;
                                    self.compile_expression(pkg, ind.index.as_ref())?;
                                    self.compile_expression(pkg, right)?;
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

                            let pkg = sel
                                .clone()
                                .map(|s| {
                                    if self.symbols.ident_is_package(&s) {
                                        s
                                    } else {
                                        pkg.to_string()
                                    }
                                })
                                .unwrap_or(pkg.to_string());

                            let resolved =
                                self.symbols
                                    .resolve(&pkg, &name)
                                    .ok_or(Error::ReferenceError(format!(
                                        "assign: `{name}` is not defined"
                                    )))?;

                            if let Some(s) = sel {
                                let t = resolved.get_type().0.strip_var().strip_ref();

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

                                        self.compile_expression(
                                            &pkg,
                                            &Expression::Ident(Ident { pos: 0, name }),
                                        )?;
                                        self.compile_expression(
                                            &pkg,
                                            &Expression::BasicLit(BasicLit {
                                                pos: 0,
                                                kind: LitKind::Integer,
                                                value: format!("{}", i),
                                            }),
                                        )?;
                                        self.compile_expression(&pkg, right)?;
                                        self.emit_opcode(OpCode::IndexSet);
                                        return Ok(None);
                                    }
                                    _ => unimplemented!("{:#?}", t),
                                }
                            }

                            let (index, setop, expect_t) = match resolved {
                                Resolved::Enclosed((s, t, _)) => {
                                    let write_op = if is_deref {
                                        OpCode::EnclosedPtrWrite
                                    } else {
                                        OpCode::SetCaptured
                                    };
                                    (s.index, write_op, t.strip_var())
                                }
                                Resolved::Local((symbol, mut t, _)) => match symbol.scope {
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

                            let got_t = self
                                .compile_expression(&pkg, right)?
                                .strip_var()
                                .strip_const();

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
                                            stripped, got_t,
                                            "left:{:#?}---right:{:#?}",
                                            left, right
                                        );
                                    }
                                } else {
                                    if !got_t.is_coerceable_to(&stripped) {
                                        assert_eq!(
                                            stripped, got_t,
                                            "expect:{:#?} got:{:#?}",
                                            left, right
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
                self.compile_expression(pkg, &expr.expr)?;
                if !self.last_instruction_is(OpCode::Pop) {
                    self.emit_opcode(OpCode::Pop);
                }
            }
            Statement::Block(stmts) => {
                return self.compile_block_statement(pkg, &stmts.list);
            }
            Statement::Declaration(declr) => match declr {
                DeclStmt::Type(t) => {
                    self.compile_declaration(pkg, &Declaration::Type(t.clone()))?;
                }
                DeclStmt::Const(t) => {
                    self.compile_declaration(pkg, &Declaration::Const(t.clone()))?;
                }
                DeclStmt::Variable(t) => {
                    self.compile_declaration(pkg, &Declaration::Variable(t.clone()))?;
                }
            },
            Statement::Return(expr) => {
                let mut rts = Vec::with_capacity(expr.ret.len());

                for r in &expr.ret {
                    let t = self.compile_expression(pkg, &r)?;
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

                let r = self.symbols.resolve(pkg, &name.name).unwrap();

                let (index, incop, t) = match r {
                    Resolved::Enclosed((s, t, _)) => (s.index, OpCode::IncCaptured, t),
                    Resolved::Local((symbol, t, _)) => match symbol.scope {
                        Scope::Local => (symbol.index, OpCode::IncLocal, t),
                        Scope::Global => (symbol.index, OpCode::IncGlobal, t),
                    },
                };

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
                        pkg,
                        name,
                        DefineType::Var(Box::new(DefineType::Null)),
                        false,
                    );
                    self.compile_expression(pkg, &rng.expr)?;
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
                            pkg,
                            key_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
                            false,
                        );
                        let value_symbol = self.symbols.define(
                            pkg,
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

                self.compile_statement(
                    pkg,
                    &Statement::Expr(ExprStmt {
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
                    }),
                )?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                //self.emit_opcode(OpCode::Pop);

                self.compile_block_statement(pkg, &rng.body.list)?;

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
                if self
                    .symbols
                    .resolve(pkg, lstmt.name.name.as_str())
                    .is_some()
                {
                    panic!("label already defined: {:#?}", lstmt.name.name);
                }

                let pos = match lstmt.stmt.as_ref() {
                    Statement::For(f) => f.pos,
                    Statement::Switch(sw) => sw.pos,
                    _ => panic!("expected for or switch statement"),
                };

                self.label_contexts
                    .insert((pos, 0), lstmt.name.name.clone());
                let r = self.compile_statement(pkg, lstmt.stmt.as_ref());
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
                    self.compile_statement(pkg, &init)?;
                }
                let internal_tag = Ident {
                    pos: 0,
                    name: "__tag__".to_string(),
                };
                let tag = switch.tag.clone().unwrap_or(Expression::Ident(Ident {
                    pos: 0,
                    name: "nil".to_string(),
                }));

                self.compile_statement(
                    pkg,
                    &Statement::Assign(AssignStmt {
                        pos: 0,
                        op: Operator::Define,
                        left: vec![Expression::Ident(internal_tag.clone())],
                        right: vec![tag.clone()],
                    }),
                )?;

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

                            self.compile_expression(pkg, &cond)?;

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            }

                            let pos_jump_if_false = self.instructions.len();
                            self.emit_opcode(OpCode::JumpIfFalse);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            terminates = terminates
                                && self
                                    .compile_block_statement(pkg, &clause.body)?
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
                                self.compile_expression(pkg, &cond)?;

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
                                        .compile_block_statement(pkg, &clause_body)?
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
                    self.compile_statement(pkg, &init)?;
                }

                let internal_tag = Ident {
                    pos: 0,
                    name: "__tag__".to_string(),
                };

                let (mut left_ass, mut left_ass_type) = (None, None);

                match switch.tag.clone().map(|a| *a) {
                    Some(Statement::Expr(expr)) => {
                        self.compile_statement(
                            pkg,
                            &Statement::Assign(AssignStmt {
                                pos: 0,
                                op: Operator::Define,
                                left: vec![Expression::Ident(internal_tag.clone())],
                                right: vec![expr.expr],
                            }),
                        )?;
                    }
                    Some(Statement::Assign(ass)) => {
                        let left = ass.left.first().unwrap().as_ident().unwrap().clone();
                        left_ass = Some(left.clone());

                        self.compile_statement(pkg, &Statement::Assign(ass))?;
                        self.compile_statement(
                            pkg,
                            &Statement::Assign(AssignStmt {
                                pos: 0,
                                op: Operator::Define,
                                left: vec![Expression::Ident(internal_tag.clone())],
                                right: vec![Expression::Ident(left.clone())],
                            }),
                        )?;
                        left_ass_type =
                            Some(self.symbols.resolve(pkg, &left.name).unwrap().get_type());
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
                                let updated = self.symbols.update_dt(pkg, &la.name, lat.0.clone());
                                assert!(updated);
                            }

                            has_default = true;
                            let cond = Expression::BasicLit(BasicLit {
                                pos: 0,
                                kind: LitKind::Ident,
                                value: "true".to_string(),
                            });

                            self.compile_expression(pkg, &cond)?;

                            if self.last_instruction_is(OpCode::Pop) {
                                self.remove_last_instruction();
                            }

                            let pos_jump_if_false = self.instructions.len();
                            self.emit_opcode(OpCode::JumpIfFalse);
                            self.emit_u16(JUMP_PLACEHOLDER);

                            terminates = terminates
                                && self
                                    .compile_block_statement(pkg, &clause.body)?
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
                                                    .resolve(pkg, &id.name)
                                                    .unwrap()
                                                    .get_type()
                                                    .0;

                                                let updated = self.symbols.update_dt(
                                                    pkg,
                                                    &la.name,
                                                    DefineType::Var(Box::new(r)),
                                                );
                                                assert!(updated);
                                            }
                                        }

                                        assert!(switch.tag.is_some());
                                        self.compile_expression(
                                            pkg,
                                            &Expression::Ident(internal_tag.clone()),
                                        )?;
                                        self.compile_expression(
                                            pkg,
                                            &Expression::Ident(id.clone()),
                                        )?;
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
                                                    .resolve(pkg, &id.name)
                                                    .unwrap()
                                                    .get_type()
                                                    .0;

                                                let updated = self.symbols.update_dt(
                                                    pkg,
                                                    &la.name,
                                                    DefineType::Var(Box::new(r)),
                                                );

                                                assert!(updated);
                                            }
                                        }

                                        self.compile_expression(
                                            pkg,
                                            &Expression::Ident(internal_tag.clone()),
                                        )?;
                                        self.compile_expression(
                                            pkg,
                                            &Expression::Ident(id.clone()),
                                        )?;
                                        self.emit_opcode(OpCode::Ref);
                                        self.emit_opcode(OpCode::TypeCmp);
                                    }
                                    _ => {
                                        assert!(switch.tag.is_none());
                                        self.compile_expression(pkg, &expr)?;
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
                                        .compile_block_statement(pkg, &clause_body)?
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

    fn compile_const_var_infix_expression(
        &mut self,
        pkg: &str,
        varname: &str,
        const_value: isize,
        operator: &Operator,
    ) -> Result<DefineType, Error> {
        let idx_constant = self.add_constant(Object::int(const_value));
        let (symbol, rt, _) = self
            .symbols
            .resolve(pkg, varname)
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

        Ok(rt)
    }

    pub(crate) fn make_type_default_val(&mut self, t: DefineType) -> Expression {
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
            DefineType::Float32 | DefineType::Float64 => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Float,
                value: "0.0".to_string(),
            }),
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
    fn typecheck_call_func_sig(&mut self, pkg: &str, call: &Call) -> Result<(), Error> {
        let f_name = if let Expression::Selector(sel) = call.func.as_ref() {
            let sellt = self
                .symbols
                .resolve(pkg, &sel.x.as_ident().unwrap().name)
                .unwrap()
                .get_type()
                .0
                .strip_var();

            make_method_name(pkg, sellt, &sel.sel.name)
        } else {
            call.func.as_ident().unwrap().name.to_string()
        };

        let mut dt = self.symbols.resolve(pkg, &f_name).unwrap().get_type().0;

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

    pub(crate) fn compile_expression(
        &mut self,
        pkg: &str,
        expr: &Expression,
    ) -> Result<DefineType, Error> {
        match expr {
            //todo this is a total mess: fix me
            Expression::Call(call) => {
                //todo typecheck return and args on builtins
                if let Expression::Ident(name) = call.func.as_ref() {
                    if let Some(builtin) = builtin::resolve(&name.name) {
                        let mut first = None;
                        for a in &call.args {
                            let t = self.compile_expression(BUILTIN, a)?;
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
                        return Ok(first.unwrap_or(DefineType::Null));
                    }
                }

                let (ct, cpkg) = CallType::from_call(&pkg, &call, self);

                let rt = match ct {
                    CallType::Func { func_dt, expr, .. } => {
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
                            let got = self.compile_expression(pkg, a)?.strip_var();
                            let expected = t.get_type();

                            if expected.is_interface() && got.implements(&pkg, &expected, self) {
                                let (name, _) = expected.as_interface();
                                let (s, _, _) =
                                    self.symbols.resolve(pkg, &name).unwrap().as_local();

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
                                    assert_eq!(t.strip_type(), got.strip_type().strip_const());
                                }
                            }
                        }

                        if is_variadic {
                            self.emit_opcode(OpCode::Variadic);
                            //panic!("{}", variadic_len);
                            self.emit_u16(variadic_len as u16);
                        }

                        self.compile_expression(&cpkg, &expr)?;

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
                            let got = self.compile_expression(pkg, a)?;
                            assert_eq!(t.as_named().unwrap().1, got);
                        }

                        self.compile_expression(
                            pkg,
                            &Expression::Ident(Ident {
                                pos: 0,
                                name: mangled_name,
                            }),
                        )?;

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
                        self.compile_expression(pkg, &iface_expr)?;
                        self.emit_opcode(OpCode::Downcast);

                        for (a, t) in call.args.iter().zip(arg_types) {
                            let got = self.compile_expression(pkg, a)?;
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
                        self.compile_expression(pkg, &iface_expr)?;
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
                let rt = self.expression_to_define_type(&pkg, expr).unwrap();
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
                                            pkg, &name.name, value, &op.op,
                                        );
                                        if res.is_ok() {
                                            return Ok(res.unwrap());
                                        }
                                    }
                                    _ => (),
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                let rt_left =
                                    self.compile_expression(pkg, op.x.as_ref())?.strip_var();
                                let rt_right =
                                    self.compile_expression(pkg, y.as_ref())?.strip_var();

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
                                self.compile_expression(pkg, op.x.as_ref())?;
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
                                            pkg, &name.name, value, &op.op,
                                        );
                                        if res.is_ok() {
                                            return Ok(res.unwrap());
                                        }
                                    }
                                    _ => {}
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                self.compile_expression(pkg, op.x.as_ref())?;
                                self.compile_expression(pkg, y.as_ref())?;
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
                                            pkg, &name.name, value, &op.op,
                                        );
                                        if res.is_ok() {
                                            return Ok(res.unwrap());
                                        }
                                    }
                                    _ => {}
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                let rt_left = self.compile_expression(pkg, op.x.as_ref())?;
                                let rt_right = self.compile_expression(pkg, y.as_ref())?;

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
                                    let left = self.compile_expression(pkg, op.x.as_ref())?;
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
                                let t = self.compile_expression(pkg, &op.x)?;
                                self.emit_opcode(OpCode::Ref);
                                return Ok(DefineType::Ref(Box::new(t.strip_var())));
                            }
                        }
                    }
                    Operator::Not => match &op.y {
                        None => {
                            let t = self.compile_expression(pkg, &op.x)?;

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
                return self.compile_expression(
                    pkg,
                    &Expression::BasicLit(BasicLit {
                        pos: 0,
                        kind: LitKind::Integer,
                        value: self.iota.to_string(),
                    }),
                );
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
                let resolved =
                    self.symbols
                        .resolve(pkg, &lit.value)
                        .ok_or(Error::ReferenceError(format!(
                            "identifier: {} not found",
                            lit.value
                        )))?;

                let (index, getop) = match resolved {
                    Resolved::Enclosed((s, _, _)) => (s.index, OpCode::GetCaptured),
                    Resolved::Local((symbol, _, _)) => match symbol.scope {
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

                    let (_, map_key_t, _) = self
                        .symbols
                        .resolve(pkg, inner_key_t.name.as_str())
                        .unwrap()
                        .as_local();
                    let (map_key_t, _) = map_key_t.as_type();

                    let (_, map_val_t, _) = self
                        .symbols
                        .resolve(pkg, inner_val_t.name.as_str())
                        .unwrap()
                        .as_local();
                    let (map_val_t, _) = map_val_t.as_type();

                    for v in &clit.val.values {
                        if let Some(key) = &v.key {
                            match key {
                                Element::Expr(el_expr) => {
                                    let expr_t = self.compile_expression(pkg, el_expr)?;
                                    assert_eq!(map_key_t, expr_t);
                                }
                                _ => {
                                    panic!("TypeMap val");
                                }
                            }
                        }

                        match &v.val {
                            Element::Expr(el_expr) => {
                                let expr_t = self.compile_expression(pkg, el_expr)?;
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

                    let slice_t = self
                        .expression_to_define_type(&pkg, ta.typ.as_ref())
                        .unwrap();
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
                                let expr_t = self.compile_expression(pkg, el_expr)?;
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
                    return Ok(literal::compile_struct(pkg, clit, name, self)?);
                }

                //array
                if let Expression::TypeArray(ta) = clit.typ.as_ref() {
                    //todo assert length
                    //if ta.len != clit.val.values.len() { }

                    let slice_t = match ta.typ.as_ref() {
                        Expression::Ident(ident) => self
                            .symbols
                            .resolve(pkg, ident.name.as_str())
                            .unwrap()
                            .as_local()
                            .1
                            .strip_type(),
                        Expression::TypeArray(_at) => self
                            .expression_to_define_type(pkg, ta.typ.as_ref())
                            .unwrap(),
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
                                let expr_t = self.compile_expression(pkg, el_expr)?;
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

                        let r = self
                            .symbols
                            .resolve(pkg, &inner_t.name)
                            .unwrap()
                            .get_type()
                            .0;

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
                        pkg,
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
                        pkg,
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

                    let rt = self.compile_expression(
                        pkg,
                        &Expression::CompositeLit(CompositeLit {
                            typ: Box::new(Expression::Ident(Ident { pos: 0, name })),
                            val: clit.val.clone(),
                        }),
                    )?;

                    self.anonymous_struct += 1;
                    return Ok(rt);
                }

                panic!("unknown composite lit {:#?}", clit);
            }
            Expression::Index(ind) => {
                let t = self.compile_expression(pkg, &ind.left)?;
                self.compile_expression(pkg, &ind.index)?;
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
                    return self.compile_expression(
                        pkg,
                        &Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Integer,
                            value: self.iota.to_string(),
                        }),
                    );
                }

                // panic!("{:#?}", self.symbols);
                return match self.symbols.resolve(pkg, &ident.name) {
                    Some(Resolved::Local((symbol, dt, _))) => {
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
                        //println!("{}-{}-{}", ident.name, symbol.index, opcode);
                        //panic!("{:#?}", symbol.index);

                        Ok(dt)
                    }
                    Some(Resolved::Enclosed((s, t, _))) => {
                        // enclosed symbols cannot be global
                        self.emit_opcode(OpCode::GetCaptured);
                        self.emit_u16(s.index);
                        //panic!("{:#?}", 2);
                        Ok(t)
                    }
                    None => Err(Error::ReferenceError(format!(
                        "ident: `{}` is not defined in pkg: `{}`",
                        ident.name, pkg,
                    ))),
                };
            }
            Expression::Selector(sel) => {
                if let Some(p) = self.symbols.get_package_path(&sel.sel.name) {
                    let r = self.compile_expression(&p, sel.x.as_ref());
                    return r;
                }

                let dt = self
                    .compile_expression(pkg, sel.x.as_ref())?
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
                fn find_field(
                    it: &[ContextType],
                    target: &str,
                ) -> Option<(Vec<usize>, ContextType)> {
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
                    self.compile_expression(
                        pkg,
                        &Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Integer,
                            value: format!("{}", p),
                        }),
                    )?;
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
                    let t = self.expression_to_define_type(pkg, &p.typ).unwrap();
                    for name in &p.name {
                        decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

                        self.symbols.define(
                            pkg,
                            &name.name,
                            DefineType::Var(Box::new(t.clone())),
                            t.is_invar(),
                        );
                    }
                }

                let mut decl_r_types = Vec::with_capacity(f.typ.result.list.len());

                for el in &f.typ.result.list {
                    let t = self.expression_to_define_type(pkg, &el.typ).unwrap();
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

                let terminates = self.compile_block_statement(pkg, &f.body.list)?;

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
                    if let Some(r) = self.symbols.resolve(pkg, &v) {
                        match r {
                            Resolved::Local((s, _t, _)) => {
                                let op = match s.scope {
                                    Scope::Local => OpCode::GetLocal,
                                    Scope::Global => OpCode::GetGlobal,
                                };
                                self.emit_opcode(op);
                                self.emit_u16(s.index);

                                self.emit_opcode(OpCode::Propagate);
                                self.emit_u16(i.try_into().unwrap());
                            }
                            Resolved::Enclosed((s, _t, _)) => {
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
                let rt = self.compile_expression(pkg, &invar.expr)?;
                return Ok(DefineType::Invar(Box::new(rt.strip_var())));
            }
            Expression::TypeAssert(type_assert) => {
                let ident = type_assert.left.as_ident().unwrap();
                let r = self.symbols.resolve(pkg, &ident.name).unwrap();
                let t = r.get_type().0;

                assert!(t.is_var());
                assert!(t.as_var().is_interface());

                let rt = self.compile_expression(pkg, &type_assert.left)?;

                match &type_assert.right {
                    Some(right) => {
                        match right.as_ref() {
                            Expression::Ident(ident) => {
                                let r = self.symbols.resolve(pkg, &ident.name).unwrap();
                                let t = r.get_type().0;

                                match t {
                                    //sidecast from interface to interface
                                    // 1. downcast to T and upcast to the interface
                                    DefineType::Interface { .. } => {
                                        let (s, _, _) = r.as_local();

                                        self.emit_opcode(OpCode::Downcast);
                                        self.emit_opcode(OpCode::Upcast);
                                        self.emit_u16(s.index);
                                        self.emit_opcode(OpCode::TypeCmp);
                                    }
                                    DefineType::Struct { .. } | DefineType::Type(_, _) => {
                                        self.emit_opcode(OpCode::Downcast);
                                        self.compile_expression(pkg, &type_assert.left)?;
                                        self.emit_opcode(OpCode::Downcast);
                                        self.compile_expression(pkg, right)?;
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
                let rt = self.expression_to_define_type(pkg, expr).unwrap();
                let obj = rt.clone().to_object();
                //panic!("{:#?}", rt);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(rt);
            }
            Expression::Slice(slice) => {
                let t = self.compile_expression(pkg, &slice.left)?;
                match t.strip_var() {
                    DefineType::Slice(_) | DefineType::Array { .. } => {}
                    tt => panic!("expected slice or array got {:#?}", tt),
                }

                let mut index_iter = slice.index.iter();

                let mut index = 0;
                if let Some(from) = index_iter.next().unwrap() {
                    let ind_t = self.compile_expression(pkg, from.as_ref())?;

                    if !ind_t.is_numeric() {
                        panic!("slicing can be done with integers only");
                    }

                    index = 1;
                }

                if let Some(from) = index_iter.next().unwrap() {
                    let ind_t = self.compile_expression(pkg, from.as_ref())?;

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

    pub(crate) fn add_constant(&mut self, obj: Object) -> u16 {
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
