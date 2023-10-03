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

func (b base) describe() string {
    return sprintf("base with num=%v", b.num)
}

type base struct {
    num int
}

type container struct {
    base
    str string
}

func main() {
    co := container{
        base: base{
            num: 1,
        },
        str: "some name",
    }

    println("co={num: %v, str: %v}\n", co.num, co.str)

    println("also num:", co.base.num)

    println("describe:", co.describe())

    type describer interface {
        describe() string
    }

    var d describer = co
    println("describer:", d.describe())
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
