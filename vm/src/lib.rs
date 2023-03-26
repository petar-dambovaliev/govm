mod nodes;

use crate::nodes::Node;
use broom::prelude::{Trace, Tracer};
use broom::{Handle, Heap};
use parser::ast::{DeclStmt, Statement};
use parser::Parser;

pub struct VM {
    opts: Opts,
    stats: Stats,
    parser: Parser,
    heap: Heap<Object>,
}

impl VM {
    pub fn new(opts: Opts, parser: Parser) -> Self {
        Self {
            opts,
            parser,
            stats: Stats::default(),
            heap: Heap::default(),
        }
    }

    pub fn run(&mut self) {
        let ast = self.parser.parse_file().unwrap();
        for decl in ast.decl {
            self.eval(Node::Decl(decl));
        }
    }

    fn eval(&mut self, node: Node) {
        self.run_gc();
        match node {
            Node::Statement(stmt) => self.eval_stmt(stmt),
            _ => {
                println!("unimplemented: {:#?}", node);
            }
        }
    }

    fn eval_stmt(&mut self, stmt: Statement) {
        match stmt {
            Statement::Declaration(declr) => match declr {
                DeclStmt::Const(cnst) => {
                    println!("const declaration: {:#?}", cnst);
                }
                _ => {
                    println!("unimplemented: {:#?}", declr);
                }
            },
            _ => {
                println!("unimplemented: {:#?}", stmt);
            }
        }
    }

    #[inline]
    fn cur_mem(&self) -> usize {
        0
    }

    #[inline]
    fn prev_mem(&self) -> usize {
        0
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

// all the types that will be handled by the GC
pub enum Object {
    Int64(i64),
    Float64(f64),
    List(Vec<Handle<Self>>),
}

impl Trace<Self> for Object {
    fn trace(&self, tracer: &mut Tracer<Self>) {
        match self {
            Object::Int64(_) => {}
            Object::Float64(_) => {}
            Object::List(objects) => objects.trace(tracer),
        }
    }
}

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
