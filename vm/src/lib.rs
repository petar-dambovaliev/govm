mod nodes;
mod symbols;
mod compiler;

use ahash::AHashMap;
use std::default::Default;
use std::fmt::{Debug, Formatter, write};
use std::ops::{Add, Sub};
use crate::nodes::Node;
use broom::prelude::{Trace, Tracer};
use broom::{Handle, Heap, Rooted};
use parser::ast::{Call, Declaration, DeclStmt, Expression, FuncDecl, Operation, Statement, Ident};
use parser::Parser;
use parser::token::{LitKind, Operator};


#[derive(Default)]
pub struct VM {
    opts: Opts,
    stats: Stats,
    parser: Parser,
    heap: Heap<Object>,
    stack: Vec<Object>,
    last_gc_round_mem: usize,
    fns: AHashMap<String, FuncDef>,
    env: AHashMap<String, Object>
}

impl Debug for VM {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#?}", &self.fns)

    }
}

#[derive(Clone, Debug)]
struct FuncDef {
    f: FuncDecl,
    env: AHashMap<String, Object>
}

impl VM {
    pub fn new(opts: Opts, parser: Parser) -> Self {
        Self {
            opts,
            parser,
            ..Default::default()
        }
    }

    pub fn run_func(&mut self, name: &str) {
        //let f = self.fns.get(name).unwrap().clone();
        self.eval_call_expr(Call{
            pos: (0, 0),
            args: vec![],
            func: Box::new(Expression::Ident(Ident{ pos: 0, name: name.to_string() })),
            dots: None,
        });
    }

    pub fn run(&mut self) {
        let ast = self.parser.parse_file().unwrap();
        for decl in ast.decl {
            //println!("{:#?}", decl);
            self.eval(Node::Decl(decl));
        }
    }

    fn eval(&mut self, node: Node) {
        self.run_gc();
        match node {
            Node::Expr(expr) => {
                self.eval_expr(expr);
            },
            Node::Statement(stmt) => {
                //self.eval_stmt(&mut self.env, stmt);
            },
            Node::Decl(declr) => {
                match declr {
                    Declaration::Function(fndeclr) => {
                        self.fns.insert(fndeclr.name.name.clone(), FuncDef{ f: fndeclr, env: AHashMap::with_capacity(10) });
                    }
                    _ => {
                        //println!("unimplemented: {:#?}", node);
                    }
                }
                //
            }
            _ => {}
        }
    }

    fn eval_expr(&mut self, expr: Expression) -> Option<Object> {
        match expr {
            Expression::Call(call) => {
                self.eval_call_expr(call)
            }
            Expression::BasicLit(bl) => {
                match bl.kind {
                    LitKind::Integer => {
                        let i: i64 = bl.value.parse().unwrap();
                        return Some(Object::Int64(i))
                    }
                    _ => {
                        unimplemented!()
                    }
                }
                //
            }
            Expression::Operation(op) => {
                self.eval_op(op);
            }
            _ => {}
        }
        None
    }

    fn eval_op(&mut self, op: Operation) -> Option<Object> {
        let left = self.eval_expr(*op.x).unwrap();
        let right = op.y.map(|a|self.eval_expr(*a).unwrap());

        match op.op {
            Operator::Add => {
                return Some(left + right.unwrap());
            }
            Operator::Sub => {
                return Some(left - right.unwrap());
            }
            _ => {
                unimplemented!()
            }
        }

        None
    }

    fn eval_call_expr(&mut self, call: Call) {
        let Call{pos, args, func,
            dots} = call;

        match *func {
            Expression::Ident(func_name) => {
                let fn_declr = match self.fns.get(&func_name.name).cloned() {
                    Some(s) => s,
                    None => return,
                };

                let mut arg_vals = Vec::with_capacity(args.len() * 2);
                for arg in args {
                    let a = self.eval_expr(arg);
                    arg_vals.extend(a);
                }

                let body = fn_declr.f.body.as_ref().unwrap();
                //let mut env = AHashMap::with_capacity(body.list.len());
                // for stmt in body.list {
                //     self.eval_stmt(&mut env, stmt);
                // }
            }
            _ => todo!(),
        }
    }

    // fn eval_func_declr(&self, func: &FuncDecl) {
    //     let body = func.body.as_ref().unwrap();
    //     for stmt in &body.list {
    //         self.eval_stmt(stmt);
    //     }
    // }

    // fn eval_call_exp(call_exp: CallExpression, env: &mut Env) -> Box<dyn Object> {
    //     let func = eval(call_exp.func.to_node(), env);
    //     if func.is_error() {
    //         return func;
    //     }
    //
    //     let args = eval_exprs(call_exp.args, env);
    //     if let Some(arg) = args.get(0) {
    //         if arg.is_error() {
    //             return arg.clone_obj();
    //         }
    //     }
    //
    //     let ins = func.inspect();
    //     if let Type::Builtin(function) = func.get_type() {
    //         return function(args, call_exp.token.line);
    //     }
    //     new_error(format!("not a function: {}", ins), call_exp.token.line)
    // }

    fn eval_stmt(&self, env: &mut AHashMap<String, Object>,  stmt: Statement) {
        match stmt {
            Statement::Declaration(declr) => match declr {
                DeclStmt::Const(cnst) => {
                    //println!("const declaration: {:#?}", cnst);
                }
                DeclStmt::Variable(var) => {
                    for spec in &var.specs {
                        //spec.typ
                    }
                    //println!("const declaration: {:#?}", cnst);
                }
                _ => {
                    //println!("unimplemented: {:#?}", declr);
                }
            },
            _ => {
                //println!("unimplemented: {:#?}", stmt);
            }
        }
    }

    #[inline]
    fn cur_mem(&self) -> usize {
        self.heap.len()
    }

    #[inline]
    fn prev_mem(&self) -> usize {
        self.last_gc_round_mem
    }

    #[inline]
    fn run_gc(&mut self) {
        let percent = (self.opts.gogc * self.prev_mem() as f64) / 100.0;
        let target = percent as usize + self.prev_mem();

        if self.opts.min_gc < target && self.cur_mem() >= target {
            dbg!(
                "running GC: current memory: {} target memory: {}",
                self.cur_mem(),
                target
            );
            self.heap.clean();
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    TypeError(String),
    SyntaxError(String),
    ReferenceError(String),
    IndexError(String),
    ArgumentError(String),
}

// all the types that will be handled by the GC
#[derive(Clone, Debug)]
pub enum Object {
    Int64(i64),
    Float64(f64),
    String(String),
    List(Vec<Handle<Self>>),
    Box(Rooted<Self>),
    Fn{ip: u32, num_locals: u16},
    Nil,
}

impl Add for Object {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int64(a), Self::Int64(b)) => {
                return Self::Int64(a + b)
            }
            _ => unimplemented!()
        }
    }
}

impl Sub for Object {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int64(a), Self::Int64(b)) => {
                return Self::Int64(a - b)
            }
            _ => unimplemented!()
        }
    }
}

impl Trace<Self> for Object {
    fn trace(&self, tracer: &mut Tracer<Self>) {
        match self {
            Self::Int64(_) => {}
            Self::Float64(_) => {}
            Self::List(objects) => objects.trace(tracer),
            Self::Box(root) => root.trace(tracer),
            Self::String(_) => {}
            Self::Fn{ .. } => {}
            Self::Nil => {}
        }
    }
}

#[derive(Default, Debug)]
pub struct Opts {
    // percantage of the heap increasing
    // to trigger a garbage collection cycle
    pub gogc: f64,
    // min heap size in bytes to trigger a
    // garbage collection cycle
    pub min_gc: usize,
}

#[derive(Default, Debug)]
struct Stats {
    allocs: usize,
    prev_allocs: usize,
}