pub mod parser;
pub mod vm;

use crate::parser::Parser;
use crate::vm::compiler::compiler::Compiler;
use crate::vm::VM;
use bdwgc_alloc::Allocator;
use std::time::Instant;

#[global_allocator]
static GLOBAL_ALLOCATOR: Allocator = Allocator;

fn main() {
    unsafe { Allocator::initialize() }

    //todo definition order matters and it shouldn't
    let i = Instant::now();
    //todo
    // this seems to make the vm crazy &[]int{1,2,3}
    let mut parser = Parser::from(
        r#"
package main

var b = a + 1 + c
var a = newnum()
var c = 3

func newnum() int {
    return 1
}

func main() {
    println(b)
    println(a)
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
