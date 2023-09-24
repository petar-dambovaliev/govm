mod call;
pub mod compiler;

use crate::vm::symbols::*;
use crate::vm::Object;
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
    GetCaptured,
    SetCaptured,
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
    Propagate,
    Deref,
    Upcast,
    Downcast,
    DynamicDispatch,
    TypeOf,
    TypeCmp,
    PanicIfFalse,
    SetDefault,
    IncLocal,
    IncGlobal,
    Slice,
    Variadic,
    TypedNull,
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
            | OpCode::Map
            | OpCode::ReturnValue
            | OpCode::Struct
            | OpCode::Upcast
            | OpCode::IncLocal
            | OpCode::IncGlobal
            | OpCode::Variadic
            | OpCode::TypedNull => &[2],

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
            | OpCode::SwapGL
            | OpCode::DynamicDispatch
            | OpCode::Array => &[2, 2],

            // OpCodes with 2 operands of 1 bytes each
            OpCode::CallBuiltin => &[1, 1],

            // OpCodes with 1 operand op 1 byte:
            OpCode::Call | OpCode::Slice => &[1],

            OpCode::SetLocal
            | OpCode::GetGlobal
            | OpCode::SetGlobal
            | OpCode::GetLocal
            | OpCode::GetCaptured
            | OpCode::SetCaptured
            | OpCode::LocalPtrWrite
            | OpCode::GlobalPtrWrite
            | OpCode::EnclosedPtrWrite
            | OpCode::Propagate => &[2],

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
            | OpCode::Downcast
            | OpCode::Deref
            | OpCode::TypeOf
            | OpCode::TypeCmp
            | OpCode::PanicIfFalse
            | OpCode::SetDefault => &[],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Bytecode {
    pub constants: Vec<Object>,
    pub instructions: Vec<u8>,
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

    fn push_continue(&mut self, pos: usize) {
        match self {
            Self::Switch(_) => panic!("no continue on a switch"),
            Self::For(f) => f.continue_instructions.push(pos),
        }
    }

    #[allow(unused)]
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

    /// Stores the index of all JUMP instructions within the current loop context that originate from a break statement
    /// Once this loop context ends, these instructions should have their operands updated to the first instruction that follows this loop
    continue_instructions: Vec<usize>,

    label: Option<String>,
}

impl LoopContext {
    fn new(start: usize, label: Option<String>) -> Self {
        Self {
            start,
            break_instructions: Vec::new(),
            continue_instructions: Vec::new(),
            label,
        }
    }
}

/// Type to keep track of function constructs so we can emit the proper jump instructions
struct FuncContext {
    /// Points to the first instruction of the (current) loop condition
    /// This is where continue statements should jump to
    #[allow(unused)]
    start: usize,

    /// Stores the index of all JUMP instructions within the current function context that originate from a return statement
    /// Once this function context ends, these instructions should have their operands updated to the first instruction that follows this function
    #[allow(unused)]
    ret_instructions: Vec<usize>,
    ret_types: Vec<(DefineType, bool)>,
    pub expected_ret: DefineType,
}

impl FuncContext {
    fn new(start: usize) -> Self {
        Self {
            start,
            ret_instructions: Vec::new(),
            ret_types: Vec::new(),
            expected_ret: DefineType::Null,
        }
    }
}

/// We use a string representation of OpCodes to make testing a little easier
impl Display for OpCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match &self {
            Self::Const => "Const",
            Self::Pop => "Pop",
            Self::True => "True",
            Self::False => "False",
            Self::Add => "Add",
            Self::Subtract => "Subtract",
            Self::Divide => "Divide",
            Self::Multiply => "Multiply",
            Self::Gt => "Gt",
            Self::Gte => "Gte",
            Self::Lt => "Lt",
            Self::Lte => "Lte",
            Self::Eq => "Eq",
            Self::Neq => "Neq",
            Self::And => "And",
            Self::Or => "Or",
            Self::Not => "Not",
            Self::Modulo => "Modulo",
            Self::Negate => "Negate",
            Self::Jump => "Jump",
            Self::JumpIfFalse => "JumpIfFalse",
            Self::Null => "Null",
            Self::Return => "Return",
            Self::ReturnValue => "ReturnValue",
            Self::Call => "Call",
            Self::CallBuiltin => "CallBuiltin",
            Self::GetLocal => "GetLocal",
            Self::SetLocal => "SetLocal",
            Self::GetCaptured => "GetCaptured",
            Self::SetCaptured => "SetCaptured",
            Self::GetGlobal => "GetGlobal",
            Self::SetGlobal => "SetGlobal",
            Self::GtLocalConst => "GtLocalConst",
            Self::GteLocalConst => "GteLocalConst",
            Self::LtLocalConst => "LtLocalConst",
            Self::LteLocalConst => "LteLocalConst",
            Self::EqLocalConst => "EqLocalConst",
            Self::NeqLocalConst => "NeqLocalConst",
            Self::AddLocalConst => "AddLocalConst",
            Self::SubtractLocalConst => "SubtractLocalConst",
            Self::MultiplyLocalConst => "MultiplyLocalConst",
            Self::DivideLocalConst => "DivideLocalConst",
            Self::ModuloLocalConst => "ModuloLocalConst",
            Self::Array => "Array",
            Self::Ref => "Ref",
            Self::IndexGet => "IndexGet",
            Self::IndexSet => "IndexSet",
            Self::Map => "Map",
            Self::Range => "Range",
            Self::IntoIter => "IntoIter",
            Self::Struct => "Struct",
            Self::LocalPtrWrite => "LocalPtrWrite",
            Self::GlobalPtrWrite => "GlobalPtrWrite",
            Self::EnclosedPtrWrite => "GlobalPtrWrite",
            Self::CopyLL => "CopyLL",
            Self::CopyLG => "CopyLG",
            Self::CopyGG => "CopyGG",
            Self::CopyGL => "CopyGL",
            Self::SwapLL => "SwapLL",
            Self::SwapGL => "SwapGL",
            Self::SwapLG => "SwapLG",
            Self::SwapGG => "SwapGG",
            Self::Propagate => "Propagate",
            Self::Deref => "Deref",
            Self::Upcast => "Icast",
            Self::Downcast => "Downcast",
            Self::DynamicDispatch => "DynamicDispatch",
            Self::TypeOf => "TypeOf",
            Self::TypeCmp => "TypeCmp",
            Self::PanicIfFalse => "PanicIfFalse",
            Self::SetDefault => "SetDefault",
            Self::IncLocal => "IncLocal",
            Self::IncGlobal => "IncGlobal",
            Self::Slice => "Slice",
            Self::Variadic => "Variadic",
            Self::TypedNull => "TypedNull",
            Self::Halt => "Halt",
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
    use crate::parser::ast::{AssignStmt, BasicLit, Expression, Ident, Operation, Statement};
    use crate::parser::token::{LitKind, Operator};
    use crate::parser::Parser;
    use crate::vm::compiler::compiler::Compiler;

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
            .define(left, DefineType::Var(Box::new(DefineType::Int)), false);
        let _r = c.compile_statement(&expr).unwrap();

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
        let _code = compiler.compile_ast(&ast).unwrap();
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
