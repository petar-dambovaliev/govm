use crate::vm::symbols::*;
use std::fmt::Display;
use std::fmt::Write;
use crate::parser::ast::{AssignStmt, BasicLit, BlockStmt, Declaration, DeclStmt, Element, Expression, ExprStmt, File, Ident, Operation, Statement};
use crate::parser::Parser;
use crate::parser::token::{Keyword, LitKind, Operator};
use crate::vm::{builtin, Error, Object};
use crate::vm::gc::GC;
use crate::vm::object::{FromString, Type};

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
            OpCode::Const | OpCode::Jump | OpCode::JumpIfFalse | OpCode::Array | OpCode::Map => &[2],

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
            | OpCode::Range => &[2, 2],

            // OpCodes with 2 operands of 1 bytes each
            OpCode::CallBuiltin => &[1, 1],

            // OpCodes with 1 operand op 1 byte:
            OpCode::Call  => &[1],

            OpCode::SetLocal | OpCode::GetGlobal | OpCode::SetGlobal | OpCode::GetLocal => &[2],

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
            | OpCode::ReturnValue
            | OpCode::IndexGet
            | OpCode::IndexSet
            | OpCode::Halt
            | OpCode::Ref
            | OpCode::IntoIter => &[],
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
    loop_contexts: Vec<LoopContext>,
    gc: GC,
}

/// Type to keep track of loop constructs so we can emit the proper jump instructions
struct LoopContext {
    /// Points to the first instruction of the (current) loop condition
    /// This is where continue statements should jump to
    start: usize,

    /// Stores the index of all JUMP instructions within the current loop context that originate from a break statement
    /// Once this loop context ends, these instructions should have their operands updated to the first instruction that follows this loop
    break_instructions: Vec<usize>,
}

impl LoopContext {
    fn new(start: usize) -> Self {
        Self {
            start,
            break_instructions: Vec::new(),
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
            loop_contexts: Vec::new(),
            gc: GC::new(),
        }
    }

    /// Compiles the given AST into executable Bytecode
    pub fn compile_ast(&mut self, ast: &File) -> Result<Bytecode, Error> {
        //insert builtin values
        self.constants.push(Object::null());
        let nil_symbol = self.symbols.define("nil");
        assert_eq!(0, nil_symbol.index);

        self.constants.push(Object::null());
        let nil_symbol = self.symbols.define("_");
        assert_eq!(1, nil_symbol.index);


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

    fn compile_declaration(&mut self, decl: &Declaration) -> Result<(), Error> {
        match decl {
            Declaration::Variable(v) => {
                for spec in &v.specs {
                    for (name, value) in spec.name.iter().zip(spec.values.iter()) {
                        let symbol = self.symbols.define(name.name.as_str());
                        self.compile_expression(value)?;
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
                let symbol = if !f.name.name.is_empty() {
                    Some(self.symbols.define(&f.name.name))
                } else {
                    None
                };

                let pos_jump = self.instructions.len();
                self.emit_opcode(OpCode::Jump);
                self.emit_u16(JUMP_PLACEHOLDER);

                // Compile function in a new scope
                self.symbols.new_context();
                for p in &f.typ.params.list {
                    for name in &p.name {
                        self.symbols.define(&name.name);
                    }
                }

                let pos_start_function = self.instructions.len();

                if let Some(body) = &f.body {
                    self.compile_block_statement(body)?;
                }

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                    self.emit_opcode(OpCode::ReturnValue);
                } else if !self.last_instruction_is(OpCode::ReturnValue) {
                    self.emit_opcode(OpCode::Return);
                }

                self.change_jump_operand_at(pos_jump, self.instructions.len().try_into().unwrap());

                // Switch back to previous scope again
                let num_locals = self.symbols.leave_context();

                // Create function object and store as constant
                let obj = Object::function(
                    pos_start_function.try_into().unwrap(),
                    num_locals.try_into().unwrap(),
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
                        let symbol = self.symbols.define(name.name.as_str());
                        self.compile_expression(value)?;
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
            Declaration::Type(_t) => {
                //todo implement struct
                // for spec in &t.specs {
                //     spec.name
                // }
                //
            }
        }
        Ok(())
    }

    fn compile_block_statement(&mut self, block: &BlockStmt) -> Result<(), Error> {
        // if block statement does not contain any other statements or expressions
        // simply push a NULL onto the stack
        if block.list.is_empty() {
            self.emit_opcode(OpCode::Null);
            return Ok(());
        }

        self.symbols.enter_scope();
        for s in &block.list {
            self.compile_statement(s)?;
        }
        self.symbols.leave_scope();
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), Error> {
        match stmt {
            Statement::For(forstmt) => {
                self.emit_opcode(OpCode::Null);
                self.loop_contexts
                    .push(LoopContext::new(self.instructions.len()));

                if let Some(init) = &forstmt.init {
                    self.compile_statement(init.as_ref())?;
                }

                let pos_before_condition = self.instructions.len();

                if let Some(cond) = &forstmt.cond {
                    self.compile_statement(cond.as_ref())?;
                }

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                self.emit_opcode(OpCode::Pop);

                self.compile_block_statement(&forstmt.body)?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                    //todo
                    // need to propagate info to add the post condition
                    // to places where there are breaks/continues
                    if let Some(post) = &forstmt.post {
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
                let ctx = self.loop_contexts.pop().unwrap();
                for ip in ctx.break_instructions {
                    self.change_jump_operand_at(ip, self.instructions.len().try_into().unwrap());
                }
            }
            Statement::If(ifstmt) => {
                self.compile_expression(&ifstmt.cond)?;
                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);

                self.compile_block_statement(&ifstmt.body)?;

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

                if let Some(alternative) = &ifstmt.else_ {
                    match alternative.as_ref() {
                        Statement::Block(bl) => {
                            self.compile_block_statement(bl)?;
                        }
                        _ => panic!("else should be a block")
                    }

                    if self.last_instruction_is(OpCode::Pop) {
                        self.remove_last_instruction();
                    }
                } else {
                    self.emit_opcode(OpCode::Null);
                }

                // Change operand of last JumpIfFalse opcode to where we're currently at
                self.change_jump_operand_at(pos_jump, self.instructions.len().try_into().unwrap());
            }
            Statement::Assign(assign) => {
                //todo this isn't going to work for `a,b := call()`
                for (left, right) in assign.left.iter().zip(assign.right.iter()) {
                    match &assign.op {
                        Operator::Define => {
                            let name = match left {
                                Expression::Ident(ident) => &ident.name,
                                _=> panic!("only identifiers can be defined: {:#?}", left)
                            };

                            let symbol = self.symbols.define(name.as_str());
                            self.compile_expression(right)?;
                            let op = if symbol.scope == Scope::Global {
                                OpCode::SetGlobal
                            } else {
                                OpCode::SetLocal
                            };
                            self.emit_opcode(op);
                            self.emit_u16(symbol.index);
                        }
                        //Operator::Define
                        Operator::Assign => 'assign: {
                            let name = match &left {
                                Expression::Ident(name) => name.name.as_str(),
                                Expression::Index(ind) => {
                                    self.compile_expression(ind.left.as_ref())?;
                                    self.compile_expression(ind.index.as_ref())?;
                                    self.compile_expression(right)?;
                                    self.emit_opcode(OpCode::IndexSet);
                                    return Ok(());
                                }
                                _ => {
                                    return Err(Error::TypeError(format!(
                                        "cannot assign a value to expressions of type {:?}",
                                        left
                                    )))
                                }
                            };

                            if name == "_" {
                                break 'assign;
                            }

                            let symbol = self.symbols.resolve(name);
                            match symbol {
                                Some(symbol) => {
                                    self.compile_expression(right)?;

                                    match symbol.scope {
                                        Scope::Global => {
                                            self.emit_opcode(OpCode::SetGlobal);
                                            self.emit_u16(symbol.index);
                                            self.emit_opcode(OpCode::GetGlobal);
                                            self.emit_u16(symbol.index);
                                        }

                                        Scope::Local => {
                                            self.emit_opcode(OpCode::SetLocal);
                                            self.emit_u16(symbol.index);
                                            self.emit_opcode(OpCode::GetLocal);
                                            self.emit_u16(symbol.index);
                                        }
                                    }
                                }
                                None => {
                                    return Err(Error::ReferenceError(format!(
                                        "assign: `{name}` is not defined"
                                    )))
                                }
                            }
                        }
                        _ => unimplemented!()
                    }
                }
            }
            Statement::Expr(expr) => {
                self.compile_expression(&expr.expr)?;
                self.emit_opcode(OpCode::Pop);
            }
            Statement::Block(stmts) => self.compile_block_statement(stmts)?,
            Statement::Declaration(declr) => {
                match declr {
                    DeclStmt::Type(t) => {
                        self.compile_declaration(&Declaration::Type(t.clone()))?;
                    }
                    DeclStmt::Const(t) => {
                        self.compile_declaration(&Declaration::Const(t.clone()))?;
                    }
                    DeclStmt::Variable(t) => {
                        self.compile_declaration(&Declaration::Variable(t.clone()))?;
                    }
                }

            }
            Statement::Return(expr) => {
                for r in &expr.ret {
                    self.compile_expression(&r)?;
                }
                self.emit_opcode(OpCode::ReturnValue);
            }
            Statement::Branch(branch) => {
                match branch.key {
                    Keyword::Break => {
                        self.emit_opcode(OpCode::Null);
                        let pos = self.instructions.len();
                        self.emit_opcode(OpCode::Jump);
                        self.emit_u16(JUMP_PLACEHOLDER);
                        let ctx = match self.loop_contexts.last_mut() {
                            Some(ctx) => ctx,
                            None => {
                                return Err(Error::SyntaxError("bad call 1".to_string()))
                            }
                        };
                        ctx.break_instructions.push(pos);
                    }
                    Keyword::Continue => {
                        self.emit_opcode(OpCode::Null);

                        let pos = match self.loop_contexts.iter().last() {
                            Some(ctx) => Ok(ctx.start),
                            None => Err(Error::SyntaxError(
                                "bad call 2".to_string(),
                            )),
                        }?;
                        self.emit_opcode(OpCode::Jump);
                        self.emit_u16(pos.try_into().unwrap());
                    }
                    _ => panic!("key: {:#?}", branch.key)
                }
            }
            Statement::IncDec(incdec) => {
                let name = match &incdec.expr {
                    Expression::Ident(ident) => ident.clone(),
                    _=> panic!("only ident allowed inc/dec")
                };

                let op = match incdec.op {
                    Operator::Inc => Operator::Add,
                    Operator::Dec => Operator::Sub,
                    _ => panic!("invalid op")
                };

                self.compile_statement(&Statement::Assign(AssignStmt{
                    pos: 0,
                    op: Operator::Assign,
                    left: vec![Expression::Ident(name.clone())],
                    right: vec![
                        Expression::Operation(Operation{
                            pos: 0,
                            op,
                            x: Box::new(Expression::Ident(name)),
                            y: Some(Box::new(Expression::BasicLit(BasicLit {
                                pos: 0,
                                kind: LitKind::Integer,
                                value: "1".to_string(),
                            }))),
                        })
                    ],
                }))?;
            }
            Statement::Empty(_) => {}
            Statement::Range(rng) => {
                self.emit_opcode(OpCode::Null);
                self.loop_contexts
                    .push(LoopContext::new(self.instructions.len()));
                let iter_sym;

                // __iter__ := into_iter X
                let iter_ident = Expression::Ident(Ident{ pos: 0, name: "__iter__".to_string() });
                {
                    let name = "__iter__";
                    iter_sym = self.symbols.define(name);
                    self.compile_expression(&rng.expr)?;
                    self.emit_opcode(OpCode::IntoIter);
                    self.emit_opcode(OpCode::SetLocal);
                    self.emit_u16(iter_sym.index);
                }

                let pos_before_condition = self.instructions.len();

                // k, v := range __iter__
                let key = rng.key.clone().unwrap_or(Expression::Ident(Ident{ pos: 0, name: "_".to_string() }));
                let value = rng.value.clone().unwrap_or(Expression::Ident(Ident{ pos: 0, name: "_".to_string() }));
                match (&key, &value) {
                    (Expression::Ident(key_id), Expression::Ident(value_id)) => {
                        let key_symbol = self.symbols.define(key_id.name.as_str());
                        let value_symbol = self.symbols.define(value_id.name.as_str());

                        self.emit_opcode(OpCode::GetLocal);
                        self.emit_u16(iter_sym.index);

                        self.emit_opcode(OpCode::Range);
                        self.emit_u16(key_symbol.index);
                        self.emit_u16(value_symbol.index);
                    }
                    _ => panic!("invalid")
                }

                self.compile_statement(&Statement::Expr(ExprStmt{ expr: Expression::Operation(Operation{
                    pos: 0,
                    op: Operator::NotEqual,
                    x: Box::new(Expression::Operation(Operation{
                        pos: 0,
                        op: Operator::And,
                        x: Box::new(key),
                        y: None,
                    })),
                    y: Some(Box::new(Expression::Ident(Ident{ pos: 0, name: "nil".to_string() }))),
                }) }))?;

                if self.last_instruction_is(OpCode::Pop) {
                    self.remove_last_instruction();
                }

                let pos_jump_if_false = self.instructions.len();
                self.emit_opcode(OpCode::JumpIfFalse);
                self.emit_u16(JUMP_PLACEHOLDER);
                self.emit_opcode(OpCode::Pop);

                self.compile_block_statement(&rng.body)?;

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
                let ctx = self.loop_contexts.pop().unwrap();
                for ip in ctx.break_instructions {
                    self.change_jump_operand_at(ip, self.instructions.len().try_into().unwrap());
                }
            }
            _ => return Err(Error::ReferenceError(format!(
                "`{:#?}` stmt not supported:", stmt
            ))),
        }

        Ok(())
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
    ) -> Result<Type, Error> {
        let idx_constant = self.add_constant(Object::int(const_value));
        let symbol = self.symbols.resolve(varname);
        match symbol {
            Some(symbol) => {
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
                        return Err(Error::ReferenceError("Optimized variant of this operator & scope type is not yet implemented.".to_string()));
                    }
                };

                self.emit_opcode(opcode);
                self.emit_u16(symbol.index);
                self.emit_u16(idx_constant);
            }
            None => {
                return Err(Error::ReferenceError(format!(
                    "{varname} is not defined"
                )))
            }
        }

        Ok(Type::Int)
    }

    fn compile_expression(&mut self, expr: &Expression) -> Result<Type, Error> {
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
                                    | (Expression::BasicLit(lit), Expression::Ident(name)) if lit.kind == LitKind::Integer => {
                                        let value: isize = lit.value.parse().unwrap();
                                        let res = self.compile_const_var_infix_expression(&name.name, value, &op.op);
                                        if res.is_ok() {
                                            return res;
                                        }
                                    }
                                    _ => (),
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                self.compile_expression(op.x.as_ref())?;
                                self.compile_expression(y.as_ref())?;
                                self.compile_operator(&op.op);

                                return Ok(Type::Int);
                            }
                            // *a // deref
                            None => {
                                unimplemented!();
                                //self.compile_expression(op.x.as_ref())?;

                                // match operator {
                                //     Operator::Negate | Operator::Subtract => {
                                //         self.emit_opcode(OpCode::Negate);
                                //     }
                                //     Operator::Not => {
                                //         self.emit_opcode(OpCode::Not);
                                //     }
                                //
                                //     _ => {
                                //         return Err(Error::TypeError(format!(
                                //             "foutieve operator voor prefix expressie: {:?}",
                                //             operator
                                //         )))
                                //     }
                                // }
                            }
                        }
                    }
                    Operator::Less | Operator::LessEqual | Operator::NotEqual => {
                        match &op.y {
                            // a * b // multiplication
                            Some(y) => {
                                match (op.x.as_ref(), y.as_ref()) {
                                    (Expression::Ident(name), Expression::BasicLit(lit))
                                    | (Expression::BasicLit(lit), Expression::Ident(name)) if lit.kind == LitKind::Integer => {
                                        let value: isize = lit.value.parse().unwrap();
                                        let res = self.compile_const_var_infix_expression(&name.name, value, &op.op);
                                        if res.is_ok() {
                                            return res;
                                        }
                                    }
                                    _ => {}
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                self.compile_expression(op.x.as_ref())?;
                                self.compile_expression(y.as_ref())?;
                                self.compile_operator(&op.op);

                                return Ok(Type::Int);
                            }
                            _ => unimplemented!()
                        }
                    }
                    Operator::Add | Operator::Sub | Operator::Rem | Operator::Equal => {
                        match &op.y {
                            Some(y) => {
                                match (op.x.as_ref(), y.as_ref()) {
                                    (Expression::Ident(name), Expression::BasicLit(lit))
                                    | (Expression::BasicLit(lit), Expression::Ident(name)) if lit.kind == LitKind::Integer => {
                                        let value: isize = lit.value.parse().unwrap();
                                        let res = self.compile_const_var_infix_expression(&name.name, value, &op.op);
                                        if res.is_ok() {
                                            return res;
                                        }
                                    }
                                    _ => {}
                                }

                                // If that failed because we haven't implemented a specialized instruction yet, compile it as a sequence of normal instructions
                                self.compile_expression(op.x.as_ref())?;
                                self.compile_expression(y.as_ref())?;
                                self.compile_operator(&op.op);

                                return Ok(Type::Int);
                            }
                            None => unimplemented!()
                        }
                    }
                    Operator::And => {
                    match &op.y {
                        Some(_y) => {
                            // a & b
                        }
                        //reference expression
                        None => {
                            let _ = self.compile_expression(&op.x)?;
                            self.emit_opcode(OpCode::Ref);
                            // if t != Type::Array {
                            //     panic!("not implemented: {:#?}", op.x);
                            // }
                        }
                    }
                    }
                    _ => panic!("unsupported op: {:#?}", op)
                }
                //
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Ident && (lit.value == "true" || lit.value == "false") => {
                let opcode = if lit.value == "true" { OpCode::True } else { OpCode::False };
                self.emit_opcode(opcode);

                return Ok(Type::Bool);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Float => {
                let obj = Object::float(lit.value.parse().unwrap(), &mut self.gc);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(Type::Float);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Integer => {
                // add to gc
                let idx = self.add_constant(Object::int(lit.value.parse().unwrap()));
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(Type::Int);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::String => {
                let obj = Object::string(lit.value.clone(), &mut self.gc);
                let idx = self.add_constant(obj);
                self.emit_opcode(OpCode::Const);
                self.emit_u16(idx);

                return Ok(Type::String);
            }
            Expression::BasicLit(lit) if lit.kind == LitKind::Ident => {
                let symbol = self.symbols.resolve(&lit.value);
                match symbol {
                    Some(symbol) => {
                        let opcode = if symbol.scope == Scope::Global {
                            OpCode::GetGlobal
                        } else {
                            OpCode::GetLocal
                        };
                        self.emit_opcode(opcode);
                        self.emit_u16(symbol.index);
                    }
                    None => {
                        return Err(Error::ReferenceError(format!(
                            "identifier: {} not found"
                            , lit.value)))
                    }
                }
            }

            Expression::Call(call) => 'compile_call: {
                for a in &call.args {
                    self.compile_expression(a)?;
                }

                if let Expression::Ident(name) = call.func.as_ref() {
                    if let Some(builtin) = builtin::resolve(&name.name) {
                        self.emit_opcode(OpCode::CallBuiltin);
                        self.emit_u8(builtin as u8);
                        self.emit_u8(call.args.len().try_into().unwrap());
                        break 'compile_call;
                    }
                }
                self.compile_expression(call.func.as_ref())?;
                self.emit_opcode(OpCode::Call);
                self.emit_u8(call.args.len().try_into().unwrap());
            }
            Expression::CompositeLit(clit) => {
                //map
                if let Expression::TypeMap(mp) = clit.typ.as_ref() {
                    let inner_key_t = match mp.key.as_ref() {
                        Expression::Ident(ident) => ident.clone(),
                        _ => unimplemented!()
                    };

                    let inner_val_t = match mp.val.as_ref() {
                        Expression::Ident(ident) => ident.clone(),
                        _ => unimplemented!()
                    };

                    let map_key_t = Type::try_from(inner_key_t.name.as_str()).unwrap();
                    let map_val_t = Type::try_from(inner_val_t.name.as_str()).unwrap();

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
                    return Ok(Type::Array);
                }

                //slice
                if let Expression::TypeSlice(ta) = clit.typ.as_ref() {
                    //todo assert length
                    //if ta.len != clit.val.values.len() { }

                    let inner_t = match ta.typ.as_ref() {
                        Expression::Ident(ident) => ident.clone(),
                        _ => unimplemented!()
                    };

                    let slice_t = Type::try_from(inner_t.name.as_str()).unwrap();
                    let mut el_t = None;
                    let key_required = clit.val.values.first().map(|a| a.key.is_some()).unwrap_or_default();

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
                    return Ok(Type::Array);
                }
            }
            Expression::Index(ind) => {
                self.compile_expression(&ind.left)?;
                self.compile_expression(&ind.index)?;
                self.emit_opcode(OpCode::IndexGet);
            }
            Expression::Ident(ident) => 'Ident: {
                if &ident.name == "true" {
                    self.emit_opcode(OpCode::True);
                    break 'Ident;
                } else if &ident.name == "false" {
                    self.emit_opcode(OpCode::False);
                    break 'Ident;
                }

                let symbol = self.symbols.resolve(&ident.name);

                match symbol.as_ref() {
                    Some(symbol) => {
                        let opcode = if symbol.scope == Scope::Global {
                            OpCode::GetGlobal
                        } else {
                            OpCode::GetLocal
                        };
                        self.emit_opcode(opcode);
                        self.emit_u16(symbol.index);
                    }
                    None => {
                        return Err(Error::ReferenceError(format!(
                            "ident: `{}` is not defined", ident.name
                        )))
                    }
                }
            }
            _ => {
                return Err(Error::SyntaxError(format!(
                    "unsupported expression:  {:#?}", expr
                )))
            }
        }

        Ok(Type::Null)
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
                2 => {
                    write!(
                        str,
                        "{}",
                        (code[ip + 1] as u16) | ((code[ip + 2] as u16) << 8)
                    )
                        .unwrap()
                },
                1 => {

                    write!(str, "{}", code[ip + 1]).unwrap()
                },
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
        assert_eq!(run("ja"), "True Pop Halt");
        assert_eq!(run("ja; ja"), "True Pop True Pop Halt");
        assert_eq!(run("nee"), "False Pop Halt");
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
        let mut p = Parser::from(r#"
        package main

        func fib(n int) {
            if n < 2 {
                return n
            }

            return fib(n - 1) + fib(n - 2)
        }
    "#);
        let mut compiler = Compiler::new();

        let ast = p.parse_file().unwrap();
        let code = compiler.compile_ast(&ast).unwrap();
        println!("{}", bytecode_to_human(&code.instructions, false))
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
