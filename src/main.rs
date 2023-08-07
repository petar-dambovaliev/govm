use std::time::Instant;
use parser::Parser;
use vm::{Opts, VM, compiler::Compiler};
use vm::compiler::bytecode_to_human;

fn main() {
    let mut parser = Parser::from(
        r#"
        package main

       var b = true;
    "#,
    );

    let opts = Opts {
        gogc: 100.0,
        min_gc: 1024 * 1024,
    };

    let f = parser.parse_file().unwrap();
    let mut goc = Compiler::new();
    let code = goc.compile_ast(f).unwrap();
    println!("{:#?}", bytecode_to_human(&code.instructions, false));

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    // println!("{:#?}", i.elapsed());
}
