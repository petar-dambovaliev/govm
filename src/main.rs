pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::VM;
use std::time::Instant;

fn main() {
    //todo definition order matters and it shouldn't
    let i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
package main

func main() {
    if 7%2 == 0 {
        println("7 is even")
    } else {
        println("7 is odd")
    }
    
    if 8%4 == 0 {
        println("8 is divisible by 4")
    }

    if num := 9; num < 0 {
        println(num, "is negative")
    } else if num < 10 {
        println(num, "has 1 digit")
    } else {
        println(num, "has multiple digits")
    }
}
    "#,
    );

    let f = parser.parse_file().unwrap();
    //panic!("{:#?}", f);
    let mut goc = Compiler::new();
    let code = goc.compile_ast(&f).unwrap();

    //panic!("{}", bytecode_to_human(&code.instructions, true));
    let mut vm = VM::new();
    let _ = vm.run(code.clone()).unwrap();

    //println!("{:#?}", parser.parse_file().unwrap());
    // let mut vm = VM::new(opts, parser);
    //  let i = Instant::now();
    // vm.run();
    // println!("{:#?}", vm);
    println!("{:#?}", i.elapsed());
}
