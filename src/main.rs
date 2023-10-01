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

type base struct {
    num int
}

func (b base) describe() string {
    return sprintf("base with num=", b.num)
}

type container struct {
    base
    str string
    num int
}

func main() {
    type describer interface {
        describe() string
    }
    
    co := container {
        base: base{
            num: 1,
        },
        str: "some name",
    }

    println(co.num, co.base.num)
    
    println("also num:", co.describe())
    // 
    // println("describe:", co.describe())
    // 
    // var d describer = co
    // println("describer:", d.describe())
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
