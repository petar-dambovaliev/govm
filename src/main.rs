use std::time::Instant;
use parser::Parser;
use vm::{Opts, VM, compiler::Compiler, Object};
use vm::compiler::bytecode_to_human;

fn main() {
    //todo definition order matters and it shouldn't
    let mut i = Instant::now();
    let mut parser = Parser::from(
        r#"
        package main

       func FibonacciRecursion(n int) int {
            if n < 2 {
                return n
            }
            return FibonacciRecursion(n-1) + FibonacciRecursion(n-2)
        }

        func main() {
            var a = FibonacciRecursion(30);
            print("{}", a);
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
    //println!("{:#?}", bytecode_to_human(&code.instructions, false));

    let mut vm = VM::new();
    let res = vm.run(code).unwrap();

    println!("{:#?}", i.elapsed().as_millis());

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    // println!("{:#?}", i.elapsed());
}
