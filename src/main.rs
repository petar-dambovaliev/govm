pub mod vm;
pub mod parser;

use std::time::Instant;
use crate::parser::Parser;
use crate::vm::compiler::{Bytecode, bytecode_to_human, Compiler};
use crate::vm::{Opts, VM};
use crate::vm::object::Object;
//use vm::compiler::bytecode_to_human;

fn main() {
    //todo definition order matters and it shouldn't
    let mut i = Instant::now();
    let mut parser = Parser::from(
        r#"
        package main

        func fibonacci(n int) int {
            if n <= 1 {
                return n
            }

            a, b := 0, 1
            for i := 2; i <= n; i++ {
                c := a + b
                a, b = b, c
            }

            return b
        }

        func main() {
            fibonacci(30)
        }
    "#,
    );

    let opts = Opts {
        gogc: 100.0,
        min_gc: 1024 * 1024,
    };

    let f = parser.parse_file().unwrap();
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();
    println!("{:#?}", bytecode_to_human(&code.instructions, false));

    for i in 0..1 {
        let mut vm = VM::new();
        let res = vm.run(code.clone()).unwrap();
    }

    println!("fibonacci_rust: {:#?}", fibonacci_rust(10));

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    // println!("{:#?}", i.elapsed());
}

fn fibonacci_rust(n: u64) -> u64 {
    if n <= 1 {
        return n;
    }
    return fibonacci_rust(n - 1) + fibonacci_rust(n - 2);
}