pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::{bytecode_to_human, Compiler};
use crate::vm::VM;
use std::time::Instant;

fn main() {
    //todo definition order matters and it shouldn't
    let _i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
        package main
        
        func newClosure() func() {
           return func() {
                return
                //print(123)
           }
           //print(123)
        }
        
        func main() {
        
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
