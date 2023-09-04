use crate::parser::ast::BranchStmt;
use crate::parser::ast::{
    AssignStmt, BasicLit, CompositeLit, DeclStmt, Declaration, Element, ExprStmt, Expression,
    FieldList, File, Ident, Index, KeyedElement, LiteralValue, Operation, Statement,
};
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::parser::Parser;
use crate::vm::gc::GC;
use crate::vm::object::{Closure, FromString, Struct, Type};
use crate::vm::symbols::*;
use crate::vm::{builtin, Error, Object};
use ahash::AHashMap;
use std::fmt::Display;
use std::fmt::Write;

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub(crate) enum OpCode {
    Const = 0,
    Pop,
    True,
    False,
    Add,
    Subtract,
    Divide,
    Multiply,
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
    And,
    Or,
    Not,
    Modulo,
    Negate,
    Jump,
    JumpIfFalse,
    Null,
    Return,
    ReturnValue,
    Call,
    CallBuiltin,
    GetEnclosed,
    SetEnclosed,
    GetLocal,
    SetLocal,
    GetGlobal,
    SetGlobal,
    GtLocalConst,
    GteLocalConst,
    LtLocalConst,
    LteLocalConst,
    EqLocalConst,
    NeqLocalConst,
    AddLocalConst,
    SubtractLocalConst,
    MultiplyLocalConst,
    DivideLocalConst,
    ModuloLocalConst,
    Array,
    IndexGet,
    IndexSet,
    Ref,
    Map,
    Range,
    IntoIter,
    Struct,
    EnclosedPtrWrite,
    LocalPtrWrite,
    GlobalPtrWrite,
    CopyGG,
    CopyLL,
    CopyGL,
    CopyLG,
    SwapLL,
    SwapGL,
    SwapLG,
    SwapGG,
    Escape,
    Deref,
    Halt,
}

const JUMP_PLACEHOLDER: u16 = 1337;

impl From<u8> for OpCode {
    #[inline(always)]
    fn from(value: u8) -> Self {
        unsafe { std::mem::transmute(value) }
    }
}

impl OpCode {
    fn operands(&self) -> &[usize] {
        match self {
            // OpCodes with 1 operand of 2 bytes
            OpCode::Const
            | OpCode::Jump
            | OpCode::JumpIfFalse
            | OpCode::Array
            | OpCode::Map
            | OpCode::ReturnValue
            | OpCode::Struct => &[2],

            // OpCodes with 2 operands of 2 bytes
            OpCode::GtLocalConst
            | OpCode::GteLocalConst
            | OpCode::LtLocalConst
            | OpCode::LteLocalConst
            | OpCode::EqLocalConst
            | OpCode::NeqLocalConst
            | OpCode::AddLocalConst
            | OpCode::SubtractLocalConst
            | OpCode::MultiplyLocalConst
            | OpCode::DivideLocalConst
            | OpCode::ModuloLocalConst
            | OpCode::Range
            | OpCode::CopyLL
            | OpCode::CopyGL
            | OpCode::CopyLG
            | OpCode::CopyGG
            | OpCode::SwapLL
            | OpCode::SwapLG
            | OpCode::SwapGG
            | OpCode::SwapGL => &[2, 2],

            // OpCodes with 2 operands of 1 bytes each
            OpCode::CallBuiltin => &[1, 1],

            // OpCodes with 1 operand op 1 byte:
            OpCode::Call => &[1],

            OpCode::SetLocal
            | OpCode::GetGlobal
            | OpCode::SetGlobal
            | OpCode::GetLocal
            | OpCode::GetEnclosed
            | OpCode::SetEnclosed
            | OpCode::LocalPtrWrite
            | OpCode::GlobalPtrWrite
            | OpCode::EnclosedPtrWrite
            | OpCode::Escape => &[2],

            // OpCodes with no operands
            OpCode::Pop
            | OpCode::True
            | OpCode::False
            | OpCode::Add
            | OpCode::Subtract
            | OpCode::Divide
            | OpCode::Multiply
            | OpCode::Gt
            | OpCode::Gte
            | OpCode::Lt
            | OpCode::Lte
            | OpCode::Eq
            | OpCode::Neq
            | OpCode::And
            | OpCode::Or
            | OpCode::Not
            | OpCode::Modulo
            | OpCode::Negate
            | OpCode::Null
            | OpCode::Return
            | OpCode::IndexGet
            | OpCode::IndexSet
            | OpCode::Halt
            | OpCode::Ref
            | OpCode::IntoIter
            | OpCode::Deref => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Vec<u8>,
}

pub struct Compiler {
    symbols: SymbolTable,
    constants: Vec<Object>,
    instructions: Vec<u8>,
    last_instruction: Option<OpCode>,
    contexts: Vec<Context>,
    func_contexts: Vec<FuncContext>,
    label_contexts: AHashMap<(usize, usize), String>,
    gc: GC,
}

enum Context {
    Switch(SwitchContext),
    For(LoopContext),
}

impl Context {
    fn to_switch(self) -> SwitchContext {
        match self {
            Self::Switch(sw) => sw,
            _ => panic!("expected switch"),
        }
    }

    fn to_for(self) -> LoopContext {
        match self {
            Self::For(sw) => sw,
            _ => panic!("expected switch"),
        }
    }

    fn push_break(&mut self, pos: usize) {
        match self {
            Self::Switch(sw) => sw.break_instructions.push(pos),
            Self::For(f) => f.break_instructions.push(pos),
        }
    }

    fn start(&self) -> usize {
        match self {
            Self::Switch(sw) => sw.start,
            Self::For(f) => f.start,
        }
    }

    fn label(&self) -> Option<&String> {
        match self {
            Self::Switch(sw) => sw.label.as_ref(),
            Self::For(f) => f.label.as_ref(),
        }
    }
}

/// Type to keep track of switch constructs so we can emit the proper jump instructions
struct SwitchContext {
    /// Points to the first instruction of the (current) loop condition
    /// This is where continue statements should jump to
    start: usize,

    /// Stores the index of all JUMP instructions within the current switch context that originate from a break statement
    /// Once this loop context ends, these instructions should have their operands updated to the first instruction that follows this switch
    break_instructions: Vec<usize>,

    label: Option<String>,
}

impl SwitchContext {
    fn new(start: usize, label: Option<String>) -> Self {
        Self {
            start,
            break_instructions: Vec::new(),
            label,
        }
    }
}

/// Type to keep track of loop constructs so we can emit the proper jump instructions
struct LoopContext {
    /// Points to the first instruction of the (current) loop condition
    /// This is where continue statements should jump to
    start: usize,

    /// Stores the index of all JUMP instructions within the current loop context that originate from a break statement
    /// Once this loop context ends, these instructions should have their operands updated to the first instruction that follows this loop
    break_instructions: Vec<usize>,

    label: Option<String>,
}

impl LoopContext {
    fn new(start: usize, label: Option<String>) -> Self {
        Self {
            start,
            break_instructions: Vec::new(),
            label,
        }
    }
}

/// Type to keep track of function constructs so we can emit the proper jump instructions
struct FuncContext {
    /// Points to the first instruction of the (current) loop condition
    /// This is where continue statements should jump to
    start: usize,

    /// Stores the index of all JUMP instructions within the current function context that originate from a return statement
    /// Once this function context ends, these instructions should have their operands updated to the first instruction that follows this function
    ret_instructions: Vec<usize>,
    ret_types: Vec<DefineType>,
}

impl FuncContext {
    fn new(start: usize) -> Self {
        Self {
            start,
            ret_instructions: Vec::new(),
            ret_types: Vec::new(),
        }
    }
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
            gc: GC::new(),
        }
    }

    /// Compiles the given AST into executable Bytecode
    pub fn compile_ast(&mut self, ast: &File) -> Result<Bytecode, Error> {
        //insert builtin values
        self.constants.push(Object::null());
        let nil_symbol = self.symbols.define(
            "nil",
            DefineType::Type(Box::new(DefineType::Null), Type::Null),
        );
        assert_eq!(0, nil_symbol.index);

        self.constants.push(Object::null());
        let nil_symbol = self
            .symbols
            .define("_", DefineType::Var(Box::new(DefineType::Null)));
        assert_eq!(1, nil_symbol.index);

        let _ = self.symbols.define(
            "int",
            DefineType::Type(Box::new(DefineType::Int), Type::Int),
        );

        let _ = self.symbols.define(
            "float",
            DefineType::Type(Box::new(DefineType::Float), Type::Float),
        );

        let _ = self.symbols.define(
            "string",
            DefineType::Type(Box::new(DefineType::String), Type::String),
        );

        let _ = self.symbols.define(
            "rune",
            DefineType::Type(Box::new(DefineType::Rune), Type::Rune),
        );

        let _ = self.symbols.define(
            "bool",
            DefineType::Type(Box::new(DefineType::Bool), Type::Bool),
        );

        //todo define error interface properly
        // implement interfaces

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

        // instruct GC to stop managing any of the constants
        // TODO: Implement custom Clone for object instead?
        for c in &self.constants {
            self.gc.untrace(*c);
        }

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

        for el in &fl.list {
            let t = match &el.typ {
                Expression::Ident(id) => self.symbols.resolve(id.name.as_str()).unwrap().get_type(),
                Expression::TypePointer(pt) => {
                    let id = pt.typ.as_ident().unwrap();
                    let t = self.symbols.resolve(id.name.as_str()).unwrap().get_type();
                    DefineType::Ref(Box::new(t))
                }
                Expression::TypeFunction(f) => {
                    let (_, t_vec) = self.field_list_to_define_type(&f.params);
                    let (dt, _) = self.field_list_to_define_type(&f.result);

                    DefineType::Func(
                        "".to_string(),
                        self.define_type_to_context_type(t_vec.as_ref()),
                        Box::new(dt),
                    )
                }
                _ => panic!("function: unsupported parameter expression: {:#?}", el.typ),
            };

            decl_r_types.push(t);
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
                DefineType::Func(
                    "".to_string(),
                    self.define_type_to_context_type(args.as_ref()),
                    Box::new(ret),
                )
            }
            Expression::TypePointer(tp) => {
                DefineType::Ref(Box::new(self.expression_to_define_type(&tp.typ)))
            }
            Expression::TypeMap(map) => {
                let mut k = self.expression_to_define_type(map.key.as_ref());
                let mut v = self.expression_to_define_type(map.val.as_ref());

                if k.is_type() {
                    k = k.as_type().0;
                }

                if v.is_type() {
                    v = v.as_type().0;
                }

                DefineType::Map(Box::new(k), Box::new(v))
            }
            _ => panic!("expression_to_define_type: unsupported expr {:#?}", expr),
        }
    }

    fn compile_declaration(&mut self, decl: &Declaration) -> Result<(), Error> {
        match decl {
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    let values = if spec.values.is_empty() {
                        let tp = self.expression_to_define_type(spec.typ.as_ref().unwrap());
                        let mut defaults = Vec::with_capacity(spec.name.len());
                        for _ in 0..spec.name.len() {
                            defaults.push(self.make_type_default_val(tp.clone()));
                        }
                        defaults
                    } else {
                        spec.values.clone()
                    };

                    for (name, value) in spec.name.iter().zip(values.iter()) {
                        let rt = self.compile_expression(value)?;

                        let symbol = self
                            .symbols
                            .define(name.name.as_str(), DefineType::Var(Box::new(rt)));

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
                //panic!("{:#?}", f);
                //f.recv
                let pos_jump = self.instructions.len();

                self.func_contexts.push(FuncContext::new(pos_jump));

                self.emit_opcode(OpCode::Jump);
                self.emit_u16(JUMP_PLACEHOLDER);

                let symbol = if !f.name.name.is_empty() {
                    Some(self.symbols.define(
                        &f.name.name,
                        DefineType::Func(f.name.name.clone(), vec![], Box::new(DefineType::Null)),
                    ))
                } else {
                    None
                };

                let mut decl_arg_types = Vec::with_capacity(f.typ.params.list.len());

                // Compile function in a new scope
                self.symbols.new_context(false);
                for p in &f.typ.params.list {
                    let t = self.expression_to_define_type(&p.typ);
                    for name in &p.name {
                        decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

                        self.symbols
                            .define(&name.name, DefineType::Var(Box::new(t.clone())));
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

                if symbol.is_some() {
                    let updated = self.symbols.update_dt(
                        f.name.name.as_str(),
                        DefineType::Func(f.name.name.clone(), decl_arg_types, Box::new(r_t)),
                    );
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
                    decl_r_types.sort();
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
                    if !terminates.unwrap_or_default()
                        && expected_t != DefineType::Null
                        && !has_top_return
                    {
                        panic!("expected return");
                    }

                    for mut ret_type in ctx.ret_types {
                        if ret_type.is_var() {
                            ret_type = ret_type.as_var();
                        }

                        if ret_type.is_type() {
                            ret_type = ret_type.as_type().0;
                        }

                        if !(terminates.unwrap_or_default() && ret_type == DefineType::Null) {
                            assert_eq!(expected_t, ret_type, "{:#?}", f.name.name);
                        }
                    }
                } else {
                    for ret_type in &ctx.ret_types {
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

                // Create function object and store as constant
                let obj = Object::function(
                    pos_start_function.try_into().unwrap(),
                    ctx.max_size().try_into().unwrap(),
                );
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                // If this function received a name, define it in the scope
                if let Some(symbol) = symbol {
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
            }
            Declaration::Const(c) => {
                for spec in &c.specs {
                    for (name, value) in spec.name.iter().zip(spec.values.iter()) {
                        let rt = self.compile_expression(value)?;

                        let symbol = self
                            .symbols
                            .define(name.name.as_str(), DefineType::Var(Box::new(rt)));

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

                        let mut field_types = vec![];
                        if let Expression::TypeStruct(ta) = &spec.typ {
                            //todo tags
                            for field in &ta.fields {
                                let (inner_t, is_ref) = match &field.typ {
                                    Expression::TypePointer(p) => (p.typ.as_ident().unwrap(), true),
                                    _ => (field.typ.as_ident().unwrap(), false),
                                };

                                if !is_ref && t.name == inner_t.name {
                                    panic!("recursive definition");
                                }

                                let dt = if is_ref {
                                    DefineType::Ref(Box::new(DefineType::Null))
                                } else {
                                    DefineType::Null
                                };

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
                        let symbol = self.symbols.define(
                            name,
                            DefineType::Struct(name.to_string(), field_types.clone()),
                        );

                        for field_type in &mut field_types {
                            let (s, dt) = field_type.as_named();
                            let resolved = match dt {
                                DefineType::Ref(_) => DefineType::Ref(Box::new(dt)),
                                _ => dt,
                            };

                            *field_type = ContextType::Named(s, resolved);
                        }

                        let updated = self
                            .symbols
                            .update_dt(name, DefineType::Struct(name.to_string(), field_types));

                        assert!(updated);

                        //todo add struct name
                        let obj = Struct::object(name.to_string(), field_values);
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
                }
            }
        }
        Ok(())
    }

    fn compile_block_statement(&mut self, block: &[Statement]) -> Result<Option<bool>, Error> {
        // if block statement does not contain any other statements or expressions
        // simply push a NULL onto the stack
        if block.is_empty() {
            self.emit_opcode(OpCode::Null);
            return Ok(Some(false));
        }

        self.symbols.enter_scope();
        let mut terminates = true;
        let mut last_term = false;

        for s in block {
            let term = self.compile_statement(s)?;
            //println!("{:#?}", term);

            let is_empty = if let Statement::Empty(_) = s {
                true
            } else {
                false
            };

            if last_term && !is_empty {
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

    fn compile_statement(&mut self, stmt: &Statement) -> Result<Option<bool>, Error> {
        match stmt {
            Statement::For(forstmt) => {
                self.emit_opcode(OpCode::Null);

                let label = self.label_contexts.get(&(forstmt.pos, 0)).cloned();
                self.contexts.push(Context::For(LoopContext::new(
                    self.instructions.len(),
                    label,
                )));

                if let Some(init) = &forstmt.init {
                    self.compile_statement(init.as_ref())?;
                }

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

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                    //todo
                    // need to propagate info to add the post condition
                    // to places where there are breaks/continues
                    if let Some(post) = &forstmt.post {
                        //panic!("{:#?}", post);
                        self.compile_statement(post.as_ref())?;
                    }
                } else {
                    if let Some(post) = &forstmt.post {
                        self.compile_statement(post.as_ref())?;
                    }
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
                for ip in &ctx.break_instructions {
                    self.change_jump_operand_at(*ip, self.instructions.len().try_into().unwrap());
                }

                let loop_terminates = (terminate.unwrap_or_default()
                    || forstmt.body.list.is_empty())
                    && forstmt.cond.is_none()
                    && ctx.break_instructions.is_empty();

                return Ok(Some(loop_terminates));
            }
            Statement::If(ifstmt) => {
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
                        _ => panic!("else should be a block"),
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
                // a, err := call()
                if assign.left.len() > 1 && assign.right.len() == 1 {
                    let dt = self.compile_expression(assign.right.first().unwrap())?;
                    let (_ident, _args, ret) = match dt {
                        DefineType::Func(ident, args, ret) => (ident, args, ret),
                        _ => panic!("expected a func"),
                    };

                    let tuple = ret.as_tuple();
                    assert_eq!(assign.left.len(), tuple.len());

                    for (left, ct) in assign.left.iter().zip(tuple).rev() {
                        match &assign.op {
                            Operator::Define => {
                                let name = match left {
                                    Expression::Ident(ident) => &ident.name,
                                    _ => panic!("only identifiers can be defined: {:#?}", left),
                                };

                                let symbol = self
                                    .symbols
                                    .define(name.as_str(), DefineType::Var(Box::new(ct)));
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
                                    Resolved::Enclosed {
                                        addr,
                                        level,
                                        heap_addr,
                                        ..
                                    } => {
                                        self.symbols.add_escaped(addr, level);
                                        (heap_addr, OpCode::SetEnclosed)
                                    }
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

                            let rt = self.compile_expression(right)?;

                            let symbol = self
                                .symbols
                                .define(name.as_str(), DefineType::Var(Box::new(rt)));

                            let op = if symbol.scope == Scope::Global {
                                OpCode::SetGlobal
                            } else {
                                OpCode::SetLocal
                            };
                            self.emit_opcode(op);
                            self.emit_u16(symbol.index);
                        }
                        Operator::Assign => 'assign: {
                            let (name, is_deref) = match &left {
                                Expression::Ident(name) => (name.name.to_string(), false),
                                Expression::Index(ind) => {
                                    self.compile_expression(ind.left.as_ref())?;
                                    self.compile_expression(ind.index.as_ref())?;
                                    self.compile_expression(right)?;
                                    self.emit_opcode(OpCode::IndexSet);
                                    return Ok(None);
                                }
                                Expression::Operation(op) => {
                                    //deref
                                    if op.y.is_none() && op.op == Operator::Star {
                                        let ident = op.x.as_ident().unwrap();
                                        (ident.name.to_string(), true)
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

                            let (index, setop, expect_t) = match resolved {
                                Resolved::Enclosed { heap_addr, t, .. } => {
                                    let write_op = if is_deref {
                                        OpCode::EnclosedPtrWrite
                                    } else {
                                        OpCode::SetEnclosed
                                    };
                                    (heap_addr, write_op, t.strip_var())
                                }
                                Resolved::Local((symbol, t)) => match symbol.scope {
                                    Scope::Local => {
                                        let write_op = if is_deref {
                                            OpCode::LocalPtrWrite
                                        } else {
                                            OpCode::SetLocal
                                        };
                                        (symbol.index, write_op, t.strip_var())
                                    }
                                    Scope::Global => {
                                        let write_op = if is_deref {
                                            OpCode::GlobalPtrWrite
                                        } else {
                                            OpCode::SetGlobal
                                        };
                                        (symbol.index, write_op, t.strip_var())
                                    }
                                },
                            };

                            let got_t = self.compile_expression(right)?;

                            if is_deref {
                                assert!(expect_t.is_ref());
                                let inner = expect_t.as_ref().strip_type();
                                assert_eq!(inner, got_t);
                            } else {
                                assert_eq!(expect_t, got_t);
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

                let rts_len = rts.len();
                let rt = if rts_len == 0 {
                    DefineType::Null
                } else if rts_len == 1 {
                    rts[0].clone()
                } else {
                    DefineType::Tuple(rts)
                };

                self.func_contexts.last_mut().unwrap().ret_types.push(rt);

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

                    let mut pos = None;
                    if let Some(l) = branch.ident.clone() {
                        for ctx in self.contexts.iter().rev() {
                            if let Some(label) = ctx.label() {
                                if &l.name == label {
                                    pos = Some(ctx.start());
                                }
                            }
                        }
                        panic!("label not found: {:#?}", branch.ident);
                    } else {
                        pos = Some(match self.contexts.iter().last() {
                            Some(ctx) => Ok(ctx.start()),
                            None => Err(Error::SyntaxError("bad call 2".to_string())),
                        }?);
                    };

                    self.emit_opcode(OpCode::Jump);
                    self.emit_u16(pos.unwrap().try_into().unwrap());
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

                let op = match incdec.op {
                    Operator::Inc => Operator::Add,
                    Operator::Dec => Operator::Sub,
                    _ => panic!("invalid op"),
                };

                self.compile_statement(&Statement::Assign(AssignStmt {
                    pos: 0,
                    op: Operator::Assign,
                    left: vec![Expression::Ident(name.clone())],
                    right: vec![Expression::Operation(Operation {
                        pos: 0,
                        op,
                        x: Box::new(Expression::Ident(name)),
                        y: Some(Box::new(Expression::BasicLit(BasicLit {
                            pos: 0,
                            kind: LitKind::Integer,
                            value: "1".to_string(),
                        }))),
                    })],
                }))?;
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
                let iter_ident = Expression::Ident(Ident {
                    pos: 0,
                    name: "__iter__".to_string(),
                });
                {
                    let name = "__iter__";
                    iter_sym = self
                        .symbols
                        .define(name, DefineType::Var(Box::new(DefineType::Null)));
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
                        );
                        let value_symbol = self.symbols.define(
                            value_id.name.as_str(),
                            DefineType::Var(Box::new(DefineType::Null)),
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
            _ => panic!("unexpected operator of type {operator:?}"),
        };
        self.emit_opcode(opcode);
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
            DefineType::Int => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Integer,
                value: "0".to_string(),
            }),
            //todo interface
            DefineType::Ref(_)
            | DefineType::Func(_, _, _)
            | DefineType::Map(_, _)
            | DefineType::Null
            | DefineType::Array(_) => Expression::Ident(Ident {
                pos: 0,
                name: "nil".to_string(),
            }),
            DefineType::Bool => Expression::Ident(Ident {
                pos: 0,
                name: "false".to_string(),
            }),
            DefineType::Float => Expression::BasicLit(BasicLit {
                pos: 0,
                kind: LitKind::Float,
                value: "0.0".to_string(),
            }),
            DefineType::Struct(n, inner_types) => {
                let mut lit_val = LiteralValue {
                    pos: (0, 0),
                    values: vec![],
                };

                for inner_type in inner_types {
                    let (key, it) = inner_type.as_named();
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
            _ => unimplemented!("{:#?}", t),
        }
    }

    fn compile_expression(&mut self, expr: &Expression) -> Result<DefineType, Error> {
        match expr {
            //todo this is a total mess: fix me
            Expression::TypeMap(tm) => {
                panic!("{:#?}", tm);
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
                    Operator::Add | Operator::Sub | Operator::Rem | Operator::Equal => {
                        match &op.y {
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
                            None => unimplemented!(),
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
                let obj = Object::float(lit.value.parse().unwrap(), &mut self.gc);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(DefineType::Float);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Integer => {
                // add to gc
                let idx = self.add_constant(Object::int(lit.value.parse().unwrap()));
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(DefineType::Int);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::String => {
                let obj = Object::string(lit.value.clone(), &mut self.gc);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(DefineType::String);
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
                    Resolved::Enclosed {
                        addr,
                        level,
                        heap_addr,
                        ..
                    } => {
                        self.symbols.add_escaped(addr, level);
                        (heap_addr, OpCode::GetEnclosed)
                    }
                    Resolved::Local((symbol, _)) => match symbol.scope {
                        Scope::Local => (symbol.index, OpCode::GetLocal),
                        Scope::Global => (symbol.index, OpCode::GetGlobal),
                    },
                };

                self.emit_opcode(getop);
                self.emit_u16(index);
            }

            Expression::Call(call) => 'compile_call: {
                //todo type check the arguments
                for a in &call.args {
                    self.compile_expression(a)?;
                }

                if let Expression::Ident(name) = call.func.as_ref() {
                    if let Some(builtin) = builtin::resolve(&name.name) {
                        let is_void = builtin.is_void();
                        self.emit_opcode(OpCode::CallBuiltin);
                        self.emit_u8(builtin as u8);
                        self.emit_u8(call.args.len().try_into().unwrap());

                        if is_void {
                            self.emit_opcode(OpCode::Pop);
                        }

                        break 'compile_call;
                    }
                }

                self.compile_expression(call.func.as_ref())?;

                self.emit_opcode(OpCode::Call);
                self.emit_u8(call.args.len().try_into().unwrap());

                // todo this can be a closure
                let mut dt = self
                    .symbols
                    .resolve(call.func.as_ident().unwrap().name.as_str())
                    .unwrap()
                    .get_type();

                if let DefineType::Var(inner) = &dt {
                    if inner.is_func() {
                        dt = *inner.clone();
                    }
                }

                if !dt.is_func() {
                    panic!("tried to call not a function: {:#?}", dt);
                }

                let rt = match dt {
                    DefineType::Func(_, _, rts) => rts.type_to_val_t(),
                    _ => unreachable!(),
                };

                return Ok(rt);
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

                    let inner_t = match ta.typ.as_ref() {
                        Expression::Ident(ident) => ident.clone(),
                        _ => unimplemented!(),
                    };

                    let slice_t = self
                        .symbols
                        .resolve(inner_t.name.as_str())
                        .unwrap()
                        .as_local()
                        .1;
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
                                assert_eq!(slice_t, expr_t);
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
                    return Ok(DefineType::Array(Box::new(slice_t)));
                }

                //struct
                if let Expression::Ident(name) = clit.typ.as_ref() {
                    //todo this can be locally defined type
                    let (s, dt) = self.symbols.resolve(name.name.as_str()).unwrap().as_local();

                    let (name, inner_types) = match dt {
                        DefineType::Struct(name, fields) => (name, fields),
                        _ => panic!("expect struct"),
                    };

                    if let Some(ct) = inner_types.first() {
                        let _ = ct.as_named();
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
                    clit_values.sort_by_key(|val| {
                        inner_types
                            .iter()
                            .map(|inner_type| inner_type.as_named())
                            .position(|x| {
                                let k_el = val.key.as_ref().unwrap();
                                let k = match k_el {
                                    Element::Expr(expr) => expr.as_ident().unwrap().clone(),
                                    _ => panic!("ident"),
                                };

                                x.0 == k.name.as_str()
                            })
                    });

                    let key_required = clit
                        .val
                        .values
                        .first()
                        .map(|a| a.key.is_some())
                        .unwrap_or_default();

                    for inner_type in inner_types.iter().rev() {
                        let (kk, inner_type) = inner_type.as_named();

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
                                assert_eq!(key_required, kel.key.is_some());

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
                                    assert_eq!(in_t, rt, "{:#?}", el_expr);
                                }
                            }
                            None => {
                                let def_val = self.make_type_default_val(inner_type);
                                let _ = self.compile_expression(&def_val)?;
                            }
                        }
                    }

                    let obj = Object::string(name.clone(), &mut self.gc);
                    let idx = self.add_constant(obj);
                    self.emit_opcode(OpCode::Const);
                    self.emit_u16(idx);

                    self.emit_opcode(OpCode::Struct);
                    self.emit_u16(inner_types.len().try_into().unwrap());
                    return Ok(DefineType::Struct(name, inner_types));
                }
                panic!("unknown composite lit {:#?}", clit);
            }
            Expression::Index(ind) => {
                self.compile_expression(&ind.left)?;
                self.compile_expression(&ind.index)?;
                self.emit_opcode(OpCode::IndexGet);
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
                match self.symbols.resolve(&ident.name) {
                    Some(Resolved::Local((symbol, dt))) => {
                        let opcode = if symbol.scope == Scope::Global {
                            OpCode::GetGlobal
                        } else {
                            OpCode::GetLocal
                        };
                        self.emit_opcode(opcode);
                        self.emit_u16(symbol.index);

                        return Ok(dt);
                    }
                    Some(Resolved::Enclosed {
                        addr,
                        level,
                        heap_addr,
                        t,
                    }) => {
                        self.symbols.add_escaped(addr, level);
                        // enclosed symbols cannot be global
                        self.emit_opcode(OpCode::GetEnclosed);
                        self.emit_u16(heap_addr);

                        return Ok(t);
                    }
                    None => {
                        return Err(Error::ReferenceError(format!(
                            "ident: `{}` is not defined",
                            ident.name
                        )))
                    }
                }
            }
            Expression::Selector(sel) => {
                let name = sel.x.as_ident().unwrap();
                let (_, dt) = self.symbols.resolve(name.name.as_str()).unwrap().as_local();
                let inner = match dt {
                    DefineType::Var(inner) => *inner,
                    _ => panic!(),
                };

                let (_, inner_types) = match inner {
                    DefineType::Struct(name, inner_types) => (name, inner_types),
                    _ => panic!(),
                };

                for (i, inner_type) in inner_types.into_iter().enumerate() {
                    let (key, dt) = inner_type.as_named();
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

                // Compile function in a new scope
                self.symbols.new_context(true);
                for p in &f.typ.params.list {
                    let t = self.expression_to_define_type(&p.typ);
                    for name in &p.name {
                        decl_arg_types.push(ContextType::Named(name.name.clone(), t.clone()));

                        self.symbols
                            .define(&name.name, DefineType::Var(Box::new(t.clone())));
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
                let mut terminates = None;
                let mut has_top_return = false;
                for stmt in &f.body.list {
                    if let Statement::Return(_) = stmt {
                        has_top_return = true;
                        break;
                    }
                }
                terminates = self.compile_block_statement(&f.body.list)?;

                let ctx = self.func_contexts.pop().unwrap();

                if !decl_r_types.is_empty() {
                    decl_r_types.sort();
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

                    for mut ret_type in ctx.ret_types {
                        if ret_type.is_var() {
                            ret_type = ret_type.as_var();
                        }
                        if !(terminates.unwrap_or_default() && ret_type == DefineType::Null) {
                            assert_eq!(expected_t, ret_type);
                        }
                    }
                } else {
                    for ret_type in &ctx.ret_types {
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
                for addr in self.symbols.contexts.last().unwrap().escaped.clone() {
                    self.emit_opcode(OpCode::Escape);
                    self.emit_u16(addr);
                }

                // Create function object and store as constant
                let obj = Closure::object(
                    pos_start_function.try_into().unwrap(),
                    num_locals.try_into().unwrap(),
                );
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(DefineType::Func(
                    "".to_string(),
                    decl_arg_types,
                    Box::new(r_t),
                ));
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

/// We use a string representation of OpCodes to make testing a little easier
impl Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use OpCode::*;
        let s = match &self {
            Const => "Const",
            Pop => "Pop",
            True => "True",
            False => "False",
            Add => "Add",
            Subtract => "Subtract",
            Divide => "Divide",
            Multiply => "Multiply",
            Gt => "Gt",
            Gte => "Gte",
            Lt => "Lt",
            Lte => "Lte",
            Eq => "Eq",
            Neq => "Neq",
            And => "And",
            Or => "Or",
            Not => "Not",
            Modulo => "Modulo",
            Negate => "Negate",
            Jump => "Jump",
            JumpIfFalse => "JumpIfFalse",
            Null => "Null",
            Return => "Return",
            ReturnValue => "ReturnValue",
            Call => "Call",
            CallBuiltin => "CallBuiltin",
            GetLocal => "GetLocal",
            SetLocal => "SetLocal",
            GetEnclosed => "GetEnclosed",
            SetEnclosed => "SetEnclosed",
            GetGlobal => "GetGlobal",
            SetGlobal => "SetGlobal",
            GtLocalConst => "GtLocalConst",
            GteLocalConst => "GteLocalConst",
            LtLocalConst => "LtLocalConst",
            LteLocalConst => "LteLocalConst",
            EqLocalConst => "EqLocalConst",
            NeqLocalConst => "NeqLocalConst",
            AddLocalConst => "AddLocalConst",
            SubtractLocalConst => "SubtractLocalConst",
            MultiplyLocalConst => "MultiplyLocalConst",
            DivideLocalConst => "DivideLocalConst",
            ModuloLocalConst => "ModuloLocalConst",
            Array => "Array",
            Ref => "Ref",
            IndexGet => "IndexGet",
            IndexSet => "IndexSet",
            Map => "Map",
            Range => "Range",
            IntoIter => "IntoIter",
            Struct => "Struct",
            CopyEnclosed => "CopyEnclosed",
            LocalPtrWrite => "LocalPtrWrite",
            GlobalPtrWrite => "GlobalPtrWrite",
            EnclosedPtrWrite => "GlobalPtrWrite",
            CopyLL => "CopyLL",
            CopyLG => "CopyLG",
            CopyGG => "CopyGG",
            CopyGL => "CopyGL",
            SwapLL => "SwapLL",
            SwapGL => "SwapGL",
            SwapLG => "SwapLG",
            SwapGG => "SwapGG",
            Halt => "Halt",
        };
        f.write_str(s)
    }
}

// Converts an array of bytes to a string representation consisting of the OpCode along with their u16 values
// For example: [OpCode::Const, 1, 0] -> "Const(1)"
#[allow(dead_code)]
pub fn bytecode_to_human(code: &[u8], positions: bool) -> String {
    let mut ip = 0;
    let mut str = String::with_capacity(256);

    while ip < code.len() {
        if ip > 0 {
            str.push(' ');
        }
        let op = OpCode::from(code[ip]);
        if positions {
            write!(str, "\n{ip:4} ").unwrap();
        }
        str.push_str(&op.to_string());

        if !op.operands().is_empty() {
            str.push('(');
        }
        for (i, width) in op.operands().iter().enumerate() {
            if i > 0 {
                str.push(',');
            }

            match width {
                2 => write!(
                    str,
                    "{}",
                    (code[ip + 1] as u16) | ((code[ip + 2] as u16) << 8)
                )
                .unwrap(),
                1 => write!(str, "{}", code[ip + 1]).unwrap(),
                _ => panic!("invalid operand width"),
            };
            ip += width;
        }
        if !op.operands().is_empty() {
            str.push(')');
        }

        ip += 1;
    }

    str
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::Ident;
    use crate::parser::Parser;

    fn run(program: &str) -> String {
        let mut p = Parser::from(program);
        let ast = p.parse_file().unwrap();
        let program = Compiler::new().compile_ast(&ast).unwrap();
        bytecode_to_human(&program.instructions, false)
    }

    fn assert_bytecode_eq(program: &str, expected: &str) {
        let mut p = Parser::from(program);
        let ast = p.parse_file().unwrap();
        let code = Compiler::new().compile_ast(&ast).unwrap();
        assert_eq!(
            bytecode_to_human(&code.instructions, false),
            expected,
            "\nInput: \t{program}\nBytecode: \t{}",
            bytecode_to_human(&code.instructions, true)
        );
    }

    #[test]
    fn test_add_assignment_expression() {
        let left = "a";
        let expr = Statement::Assign(AssignStmt {
            pos: 0,
            op: Operator::Assign,
            left: vec![Expression::Ident(Ident {
                pos: 0,
                name: left.to_string(),
            })],
            right: vec![Expression::Operation(Operation {
                pos: 0,
                op: Operator::Add,
                x: Box::new(Expression::Ident(Ident {
                    pos: 0,
                    name: left.to_string(),
                })),
                y: Some(Box::new(Expression::BasicLit(BasicLit {
                    pos: 0,
                    kind: LitKind::Integer,
                    value: "1".to_string(),
                }))),
            })],
        });

        let mut c = Compiler::new();
        c.symbols
            .define(left, DefineType::Var(Box::new(DefineType::Int)));
        let r = c.compile_statement(&expr).unwrap();

        println!("{}", bytecode_to_human(&c.instructions, true));
    }

    #[test]
    fn test_int_expression() {
        assert_eq!(run("5"), "Const(0) Pop Halt");
        assert_eq!(run("5; 5"), "Const(0) Pop Const(0) Pop Halt");
        assert_eq!(
            run("5; 6; 5"),
            "Const(0) Pop Const(1) Pop Const(0) Pop Halt"
        );
    }

    #[test]
    fn test_bool_expression() {
        assert_eq!(run("false"), "True Pop Halt");
        assert_eq!(run("true; true"), "True Pop True Pop Halt");
        assert_eq!(run("false"), "False Pop Halt");
    }

    #[test]
    fn test_float_expression() {
        assert_eq!(run("1.23"), "Const(0) Pop Halt");
        assert_eq!(run("1.23; 1.23"), "Const(0) Pop Const(0) Pop Halt");
        assert_eq!(
            run("5.00; 6.00; 5.00"),
            "Const(0) Pop Const(1) Pop Const(0) Pop Halt"
        );
    }

    #[test]
    fn test_infix_expression() {
        assert_eq!(run("1 + 2"), "Const(0) Const(1) Add Pop Halt");
        assert_eq!(run("1 - 2"), "Const(0) Const(1) Subtract Pop Halt");
        assert_eq!(run("1 * 2"), "Const(0) Const(1) Multiply Pop Halt");
        assert_eq!(run("1 / 2"), "Const(0) Const(1) Divide Pop Halt");
        assert_eq!(
            run("1 * 2 * 3"),
            "Const(0) Const(1) Multiply Const(2) Multiply Pop Halt"
        );
    }

    #[test]
    fn test_block_statements() {
        assert_eq!(run("{ 1 }"), "Const(0) Pop Halt");
    }

    #[test]
    fn test_if_expression() {
        assert_bytecode_eq(
            "als ja { 1 }",
            "True JumpIfFalse(10) Const(0) Jump(11) Null Pop Halt",
        );
        assert_bytecode_eq(
            "als ja { 1 } anders { 2 }",
            "True JumpIfFalse(10) Const(0) Jump(13) Const(1) Pop Halt",
        );
    }

    #[test]
    fn test_if_expression_empty_body() {
        assert_bytecode_eq(
            "als ja { }",
            "True JumpIfFalse(8) Null Jump(9) Null Pop Halt",
        );

        assert_bytecode_eq(
            "als ja { } anders { 1 }",
            "True JumpIfFalse(8) Null Jump(11) Const(0) Pop Halt",
        );
    }

    #[test]
    fn test_if_expression_empty_else() {
        assert_bytecode_eq(
            "als ja { 1 } anders {}",
            "True JumpIfFalse(10) Const(0) Jump(11) Null Pop Halt",
        );
    }

    #[test]
    fn test_function_expression() {
        assert_bytecode_eq(
            "functie() { 1 }",
            "Jump(7) Const(0) ReturnValue Const(1) Pop Halt",
        );

        assert_bytecode_eq(
            "functie() { 1 } functie() { 2 }",
            "Jump(7) Const(0) ReturnValue Const(1) Pop Jump(18) Const(2) ReturnValue Const(3) Pop Halt"
        );
    }

    #[test]
    fn test_call_expression() {
        assert_bytecode_eq(
            "functie(a, b) { 1 }(1, 2)",
            "Const(0) Const(1) Jump(13) Const(0) ReturnValue Const(2) Call(2) Pop Halt",
        );
    }

    #[test]
    fn test_declare_statement() {
        assert_eq!(run("stel a = 1;"), "Const(0) SetGlobal(0) Halt");

        assert_eq!(
            run("stel a = 1; stel b = 2;"),
            "Const(0) SetGlobal(0) Const(1) SetGlobal(1) Halt"
        );

        // TODO: Test scoped variables
    }

    #[test]
    fn test_ident_expressions() {
        assert_eq!(
            run("stel a = 1; a"),
            "Const(0) SetGlobal(0) GetGlobal(0) Pop Halt"
        );

        assert_eq!(
            run("stel a = 1; stel b = 2; a; b;"),
            "Const(0) SetGlobal(0) Const(1) SetGlobal(1) GetGlobal(0) Pop GetGlobal(1) Pop Halt"
        );

        // TODO: Test scoped variables
    }

    #[test]
    fn test_call_builtin() {
        let mut p = Parser::from(
            r#"
        package main

        func fib(n int) {
            if n < 2 {
                return n
            }

            return fib(n - 1) + fib(n - 2)
        }
    "#,
        );
        let mut compiler = Compiler::new();

        let ast = p.parse_file().unwrap();
        let code = compiler.compile_ast(&ast).unwrap();
        //println!("{}", bytecode_to_human(&code.instructions, false))
        // assert_eq!(
        //     run(r#"
        //         package main
        //         func main(){}
        //         var a = print("hallo")
        //     "#),
        //     "Const(0) CallBuiltin(0,1) Pop Halt"
        // )
    }
}
