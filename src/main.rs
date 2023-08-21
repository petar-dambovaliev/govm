pub mod vm;
pub mod parser;

use std::time::Instant;
use crate::parser::Parser;
use crate::vm::compiler::{bytecode_to_human, Compiler};
use crate::vm::VM;

fn main() {
    //todo definition order matters and it shouldn't
    let _i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
        package main

        type Foo struct {
            i int
            b int
        }
        func main() {
            f := Foo{b: 2, i: 1}
            print(f)
        }
    "#,
    );

    // let opts = Opts {
    //     gogc: 100.0,
    //     min_gc: 1024 * 1024,
    // };

    let f = parser.parse_file().unwrap();
    //panic!("{:#?}", f);
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();
    //println!("{:#?}", bytecode_to_human(&code.instructions, false));
    let mut vm = VM::new();
    let _ = vm.run(code.clone()).unwrap();

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    // println!("{:#?}", i.elapsed());
}